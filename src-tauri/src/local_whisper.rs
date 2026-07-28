use crate::model_manager::ModelManager;
use crate::transcription::{TranscriptionError, TranscriptionService};
use async_trait::async_trait;
use hound;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Hard ceiling on a single local transcription. If whisper.cpp doesn't finish (or
/// cooperatively abort) within this window, we give up waiting and surface an error
/// instead of leaving the app stuck in "Transcribing" forever.
const TRANSCRIBE_TIMEOUT_SECS: u64 = 180;
/// Extra grace period after signalling abort, to let whisper.cpp unwind cooperatively
/// before we stop waiting on the worker thread entirely.
const TRANSCRIBE_ABORT_GRACE_SECS: u64 = 5;
/// Hard ceiling on loading a model into memory (and GPU memory, if enabled). There's no
/// way to cooperatively cancel `WhisperContext::new_with_params` — it has no abort hook —
/// so this only bounds how long we *wait*; a genuinely hung GPU/Vulkan init keeps running
/// on its own thread in the background rather than blocking the app forever.
const MODEL_LOAD_TIMEOUT_SECS: u64 = 120;

/// A Whisper model already loaded into memory (and GPU memory, if `use_gpu` was set).
/// Kept alive in `AppState` so repeated transcriptions reuse it instead of reloading
/// the model file from disk on every recording.
pub struct LoadedWhisperModel {
    pub context: WhisperContext,
    pub model_size: String,
    pub use_gpu: bool,
}

pub type WhisperEngineCache = Arc<Mutex<Option<LoadedWhisperModel>>>;

/// A VoxBridge model already loaded into memory. Kept alive in `AppState` so repeated
/// transcriptions reuse it instead of reloading the model file (and re-dlopen-ing the
/// engine DLL) on every recording — same caching shape as `LoadedWhisperModel`.
pub struct LoadedVoxBridgeModel {
    pub engine: Arc<voxbridge::Engine>,
    pub model: voxbridge::Model,
    pub model_size: String,
    pub use_gpu: bool,
}

pub type VoxBridgeEngineCache = Arc<Mutex<Option<LoadedVoxBridgeModel>>>;

/// Tries to (re)use a cached VoxBridge model matching `model_size`/`use_gpu`, loading
/// one if there isn't a match yet (including when only `use_gpu` changed - a GPU and a
/// CPU engine are different DLLs, never interchangeable). No GPU->CPU fallback of its
/// own: on any failure here, the caller (`LocalWhisperService::transcribe`) falls
/// through to the existing whisper-rs path, which already has its own GPU->CPU
/// fallback - VoxBridge doesn't need to duplicate that.
fn ensure_voxbridge_model_loaded(
    cache: &VoxBridgeEngineCache,
    model_size: &str,
    use_gpu: bool,
    resource_base: Option<&std::path::Path>,
) -> Result<(), String> {
    {
        let guard = cache.lock().unwrap();
        if let Some(loaded) = guard.as_ref() {
            if loaded.model_size == model_size && loaded.use_gpu == use_gpu {
                return Ok(());
            }
        }
    }

    let dev_fallback_base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidate_bases: Vec<&std::path::Path> = resource_base
        .into_iter()
        .chain(std::iter::once(dev_fallback_base))
        .collect();
    let engines_dir = voxbridge::resolve_engines_dir(&candidate_bases)
        .ok_or_else(|| "no VoxBridge engines-dist/ found".to_string())?;
    let engine = if use_gpu {
        voxbridge::Engine::load_best_gpu(&engines_dir)?
    } else {
        voxbridge::Engine::load_best(&engines_dir)?
    };

    let model_manager = ModelManager::new()?;
    let model_path = model_manager.get_model_path(model_size);
    let model = engine.load_model(
        model_path
            .to_str()
            .ok_or_else(|| "invalid model path".to_string())?,
    )?;

    log_info!(
        "VoxBridge: loaded model '{}' via engine variant '{}'",
        model_size,
        engine.variant_name()
    );

    *cache.lock().unwrap() = Some(LoadedVoxBridgeModel {
        engine,
        model,
        model_size: model_size.to_string(),
        use_gpu,
    });
    Ok(())
}

/// Preloads and exercises the same VoxBridge model path used by real transcription.
/// Loading the whisper-rs fallback alone is not sufficient: VoxBridge owns a separate
/// engine/model cache, so without this warmup the first utterance reloads the model and
/// pays the initial GPU setup cost.
pub fn preload_voxbridge_model(
    cache: &VoxBridgeEngineCache,
    model_size: &str,
    use_gpu: bool,
    resource_base: Option<&std::path::Path>,
) -> Result<(), String> {
    let start = Instant::now();
    ensure_voxbridge_model_loaded(cache, model_size, use_gpu, resource_base)?;

    let silence = vec![0.0f32; 8000];
    let guard = cache.lock().unwrap();
    let loaded = guard
        .as_ref()
        .ok_or_else(|| "VoxBridge model failed to stay cached during warmup".to_string())?;
    loaded.model.transcribe(&silence, Some("en"), None)?;

    log_info!(
        "VoxBridge warm-up complete for '{}' (gpu={}) in {:.2}s",
        model_size,
        use_gpu,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

fn load_context(model_path: &PathBuf, use_gpu: bool) -> Result<WhisperContext, TranscriptionError> {
    let context_params = WhisperContextParameters {
        use_gpu,
        ..Default::default()
    };
    if use_gpu {
        log_info!("Attempting to use GPU acceleration (Vulkan/Metal/CUDA)...");
    }

    WhisperContext::new_with_params(
        model_path
            .to_str()
            .ok_or_else(|| TranscriptionError::ModelError("Invalid model path".to_string()))?,
        context_params,
    )
    .map_err(|e| TranscriptionError::ModelError(e.to_string()))
}

/// Runs one throwaway inference on silence right after loading. On GPU, the Vulkan
/// backend JIT-compiles its compute shaders the first time they're actually *used* (not
/// when the context/weights are loaded) — that compilation is a one-time cost per
/// machine (the driver caches the compiled shaders on disk afterward), but if we don't
/// pay it here, the user's *first real recording* pays it instead, turning something
/// that should take under a second into a many-second wait with no explanation. Doing it
/// during the already-visible "Loading Model" step means it never surprises anyone.
/// Best-effort: a warm-up failure doesn't fail the load, it's just logged.
fn warm_up(context: &WhisperContext, model_size: &str, use_gpu: bool) {
    let start = Instant::now();

    // Distinct from "Loading Model": the weights are already in memory/VRAM by this
    // point, this step is specifically about compiling/caching GPU shaders (or spinning
    // up the CPU thread pool) so it doesn't show up as a mysterious pause later.
    if use_gpu {
        tauri::async_runtime::spawn(async move {
            crate::app::status::emit_status_update("Warmup").await;
        });
    }

    let mut state = match context.create_state() {
        Ok(s) => s,
        Err(e) => {
            log_warn!("Warm-up skipped for '{}': failed to create state: {}", model_size, e);
            return;
        }
    };

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_translate(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    params.set_no_context(true);

    // A little under 1 second of silence is enough to exercise the full encode/decode
    // path (whisper.cpp pads to a fixed internal window regardless of input length).
    let silence = vec![0.0f32; 8000];

    match state.full(params, &silence) {
        Ok(()) => log_info!(
            "Warm-up complete for '{}' (gpu={}) in {:.2}s",
            model_size,
            use_gpu,
            start.elapsed().as_secs_f64()
        ),
        Err(e) => log_warn!(
            "Warm-up failed for '{}' (gpu={}) after {:.2}s: {} (real transcriptions may still work; this only means the first one will be slower)",
            model_size,
            use_gpu,
            start.elapsed().as_secs_f64(),
            e
        ),
    }
}

/// Returns the cached model if it already matches `model_size`/`use_gpu`, otherwise
/// loads it fresh (evicting whatever was cached before) and caches the result.
///
/// Deliberately does *not* hold the cache lock while `load_context` runs: that call can
/// take a long time (or, on a broken GPU/driver combo, hang indefinitely), and holding
/// the mutex through it would permanently lock every other caller out of the cache —
/// including a later attempt to fall back to CPU-only — the moment one load gets stuck.
pub fn ensure_model_loaded(
    cache: &WhisperEngineCache,
    model_size: &str,
    use_gpu: bool,
) -> Result<(), TranscriptionError> {
    let model_manager = ModelManager::new().map_err(|e| TranscriptionError::ModelError(e))?;
    let model_path = model_manager.get_model_path(model_size);

    if !model_path.exists() {
        return Err(TranscriptionError::ModelError(format!(
            "Model {} not found. Please download it in settings.",
            model_size
        )));
    }

    {
        let guard = cache.lock().unwrap();
        let already_loaded = matches!(
            &*guard,
            Some(loaded) if loaded.model_size == model_size && loaded.use_gpu == use_gpu
        );
        if already_loaded {
            return Ok(());
        }
    }

    log_info!(
        "Loading Whisper model '{}' (gpu={}) into memory...",
        model_size,
        use_gpu
    );
    let context = load_context(&model_path, use_gpu)?;
    warm_up(&context, model_size, use_gpu);

    let mut guard = cache.lock().unwrap();
    *guard = Some(LoadedWhisperModel {
        context,
        model_size: model_size.to_string(),
        use_gpu,
    });
    log_info!("Whisper model '{}' loaded and cached", model_size);
    Ok(())
}

/// Runs `ensure_model_loaded` off the async runtime with a hard wall-clock timeout, so a
/// caller never waits on it forever. If the timeout fires, the underlying blocking thread
/// keeps running (there's no safe way to kill it) — if it eventually succeeds, the model
/// ends up cached anyway and the *next* attempt will be fast; if it's genuinely hung, it
/// just leaks one blocking-pool thread rather than freezing the app.
pub async fn ensure_model_loaded_with_timeout(
    cache: WhisperEngineCache,
    model_size: String,
    use_gpu: bool,
) -> Result<(), TranscriptionError> {
    let task_cache = cache.clone();
    let task_model_size = model_size.clone();
    let handle = tokio::task::spawn_blocking(move || {
        ensure_model_loaded(&task_cache, &task_model_size, use_gpu)
    });

    match tokio::time::timeout(Duration::from_secs(MODEL_LOAD_TIMEOUT_SECS), handle).await {
        Ok(Ok(inner_result)) => inner_result,
        Ok(Err(join_error)) => Err(TranscriptionError::ModelError(format!(
            "Model loading task crashed unexpectedly: {}",
            join_error
        ))),
        Err(_elapsed) => {
            log_warn!(
                "Loading Whisper model '{}' (gpu={}) timed out after {}s; it may still finish in the background",
                model_size,
                use_gpu,
                MODEL_LOAD_TIMEOUT_SECS
            );
            Err(TranscriptionError::ModelError(format!(
                "Loading the '{}' model timed out after {}s{}. Try disabling GPU acceleration in Settings, or pick a smaller model.",
                model_size,
                MODEL_LOAD_TIMEOUT_SECS,
                if use_gpu {
                    " — GPU/Vulkan initialization may be stuck"
                } else {
                    ""
                }
            )))
        }
    }
}

/// Result of `ensure_model_loaded_with_fallback`: which backend actually ended up
/// loaded, and — if it wasn't the one requested — why, so the UI can tell the user
/// what happened instead of silently doing something different than they asked for.
pub struct LoadOutcome {
    pub use_gpu: bool,
    pub fell_back_from_gpu: Option<String>,
}

/// Like `ensure_model_loaded_with_timeout`, but never lets a GPU failure be the end of
/// the story: if `use_gpu` is requested and fails for *any* reason (timeout, driver
/// error, whatever), automatically retries once on CPU instead of surfacing the error
/// to the user as "transcription is broken." Per-attempt only — does not change the
/// user's saved GPU setting, so it'll try GPU again next time (in case the failure was
/// transient), but the app never becomes unusable just because GPU didn't work on this
/// particular piece of hardware.
pub async fn ensure_model_loaded_with_fallback(
    cache: WhisperEngineCache,
    model_size: String,
    use_gpu: bool,
) -> Result<LoadOutcome, TranscriptionError> {
    if !use_gpu {
        ensure_model_loaded_with_timeout(cache, model_size, false).await?;
        return Ok(LoadOutcome {
            use_gpu: false,
            fell_back_from_gpu: None,
        });
    }

    match ensure_model_loaded_with_timeout(cache.clone(), model_size.clone(), true).await {
        Ok(()) => Ok(LoadOutcome {
            use_gpu: true,
            fell_back_from_gpu: None,
        }),
        Err(gpu_error) => {
            log_warn!(
                "GPU load of '{}' failed ({}), falling back to CPU for this attempt",
                model_size,
                gpu_error
            );
            unload_model(&cache); // drop anything left over from the failed GPU attempt
            ensure_model_loaded_with_timeout(cache, model_size, false).await?;
            Ok(LoadOutcome {
                use_gpu: false,
                fell_back_from_gpu: Some(gpu_error.to_string()),
            })
        }
    }
}

pub fn unload_model(cache: &WhisperEngineCache) {
    let mut guard = cache.lock().unwrap();
    *guard = None;
}

pub struct LocalWhisperService {
    cache: WhisperEngineCache,
    model_size: String,
    use_gpu: bool,
    /// Shared with `AppState.whisper_last_gpu_error` (when constructed from the app) so a
    /// GPU failure discovered here (not just at preload time) still feeds the hardware
    /// report. `None` when there's nowhere to report it (e.g. standalone/test usage).
    last_gpu_error: Option<Arc<Mutex<Option<String>>>>,
    /// VoxBridge's model cache, if the app has one (`AppState.voxbridge_engine`). Tried
    /// first for both CPU and GPU, with an automatic fall-through to the whisper-rs path
    /// below on any failure (no VoxBridge build present, load failure, transcribe
    /// failure) - VoxBridge is a performance opportunity, never a new way for
    /// transcription to break.
    voxbridge_cache: Option<VoxBridgeEngineCache>,
    /// The app's resource directory (`app_handle.path().resource_dir()`), for finding a
    /// packaged VoxBridge build. `None` in dev/test contexts, where VoxBridge falls back
    /// to looking next to its own crate source instead (see `resolve_engines_dir`).
    voxbridge_resource_base: Option<PathBuf>,
}

impl LocalWhisperService {
    pub fn new_full(
        cache: WhisperEngineCache,
        model_size: &str,
        use_gpu: bool,
        last_gpu_error: Option<Arc<Mutex<Option<String>>>>,
        voxbridge_cache: Option<VoxBridgeEngineCache>,
        voxbridge_resource_base: Option<PathBuf>,
    ) -> Result<Self, TranscriptionError> {
        // Validate the model exists up front so callers get an immediate, clear error
        // instead of failing deep inside transcribe().
        let model_manager = ModelManager::new().map_err(|e| TranscriptionError::ModelError(e))?;
        let model_path = model_manager.get_model_path(model_size);
        if !model_path.exists() {
            return Err(TranscriptionError::ModelError(format!(
                "Model {} not found. Please download it in settings.",
                model_size
            )));
        }

        Ok(Self {
            cache,
            voxbridge_cache,
            voxbridge_resource_base,
            model_size: model_size.to_string(),
            use_gpu,
            last_gpu_error,
        })
    }
}

#[async_trait]
impl TranscriptionService for LocalWhisperService {
    async fn transcribe(
        &self,
        audio_data: &[u8],
        language: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<String, TranscriptionError> {
        let start = Instant::now();
        log_info!(
            "Local transcription: starting (model='{}', gpu={}, audio_bytes={})",
            self.model_size,
            self.use_gpu,
            audio_data.len()
        );

        // Convert WAV bytes to f32 samples
        let mut reader = hound::WavReader::new(std::io::Cursor::new(audio_data))
            .map_err(|e| TranscriptionError::AudioError(e.to_string()))?;

        let spec = reader.spec();
        if spec.channels != 1 || spec.sample_rate != 16000 {
            return Err(TranscriptionError::AudioError(format!(
                "Unsupported audio format: {} channels, {}Hz. Expected 1 channel, 16000Hz.",
                spec.channels, spec.sample_rate
            )));
        }

        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| TranscriptionError::AudioError(e.to_string()))?;
        log_info!(
            "Local transcription: decoded {} samples (~{:.2}s of audio)",
            samples.len(),
            samples.len() as f64 / 16000.0
        );

        // Try VoxBridge first: proper per-CPU-ISA dispatch (AVX2 vs. a true SSE4.2
        // floor) on CPU, or the Vulkan-enabled variant on GPU, instead of whisper-rs's
        // one-size-fits-all generic build. Any failure here (no VoxBridge build present,
        // load failure, transcribe failure) falls straight through to the existing
        // whisper-rs path below unchanged - including whisper-rs's own GPU->CPU
        // fallback, which VoxBridge doesn't need to duplicate. VoxBridge is strictly a
        // performance opportunity, never a new way for transcription to break.
        {
            if let Some(voxbridge_cache) = &self.voxbridge_cache {
                match ensure_voxbridge_model_loaded(
                    voxbridge_cache,
                    &self.model_size,
                    self.use_gpu,
                    self.voxbridge_resource_base.as_deref(),
                ) {
                    Ok(()) => {
                        let vb_cache = voxbridge_cache.clone();
                        let vb_samples = samples.clone();
                        let vb_language = language.map(|s| s.to_string());
                        let vb_prompt = prompt.map(|s| s.to_string());

                        let vb_result = tokio::task::spawn_blocking(move || -> Result<String, String> {
                            let guard = vb_cache.lock().unwrap();
                            let loaded = guard
                                .as_ref()
                                .ok_or_else(|| "VoxBridge model failed to stay cached".to_string())?;
                            loaded
                                .model
                                .transcribe(&vb_samples, vb_language.as_deref(), vb_prompt.as_deref())
                        })
                        .await;

                        match vb_result {
                            Ok(Ok(text)) => {
                                log_info!(
                                    "Local transcription: handled by VoxBridge in {:.2}s",
                                    start.elapsed().as_secs_f64()
                                );
                                return Ok(text);
                            }
                            Ok(Err(e)) => {
                                log_warn!(
                                    "VoxBridge transcription failed ({}), falling back to whisper-rs",
                                    e
                                );
                            }
                            Err(join_error) => {
                                log_warn!(
                                    "VoxBridge transcription task failed ({}), falling back to whisper-rs",
                                    join_error
                                );
                            }
                        }
                    }
                    Err(e) => {
                        log_info!("VoxBridge not available ({}), using whisper-rs", e);
                    }
                }
            }
        }

        // Reuse the cached model if it already matches; otherwise load it once here
        // (e.g. first transcription after startup, or after a model/GPU setting change).
        // Falls back to CPU automatically if GPU loading fails, rather than failing the
        // whole transcription — see `ensure_model_loaded_with_fallback`.
        let load_outcome = ensure_model_loaded_with_fallback(
            self.cache.clone(),
            self.model_size.clone(),
            self.use_gpu,
        )
        .await?;
        if let Some(gpu_error) = &load_outcome.fell_back_from_gpu {
            log_warn!(
                "Local transcription: GPU load failed ({}), fell back to CPU for this transcription",
                gpu_error
            );
            if let Some(sink) = &self.last_gpu_error {
                *sink.lock().unwrap() = Some(gpu_error.clone());
            }
        }
        log_info!(
            "Local transcription: model ready after {:.2}s, starting inference (timeout={}s)",
            start.elapsed().as_secs_f64(),
            TRANSCRIBE_TIMEOUT_SECS
        );

        let cache = self.cache.clone();
        let language = language.map(|s| s.to_string());
        let prompt = prompt.map(|s| s.to_string());

        // NOTE: we deliberately do NOT set an abort_callback here. whisper-rs's
        // `set_abort_callback_safe` reliably makes whisper.cpp's encode step fail
        // (`whisper_full_with_state: failed to encode`, error -6) on this whisper-rs/
        // whisper.cpp build — confirmed via a standalone reproduction: identical
        // transcription succeeds every time with no abort callback set, and fails every
        // time with one set (even one that always returns false), CPU or GPU, regardless
        // of model or audio content. Root cause not fully understood (likely a bug in the
        // FFI trampoline or in how whisper.cpp's ggml graph build reacts to a non-null
        // abort_callback), not worth blocking a working transcription pipeline on. The
        // outer `tokio::time::timeout` below still bounds worst-case wait time; we just
        // lose the ability to tell whisper.cpp to cooperatively stop early — a stuck
        // inference keeps running in the background instead, same as an already-documented
        // limitation of the model-load timeout.
        let inference = tokio::task::spawn_blocking(move || -> Result<String, TranscriptionError> {
            let guard = cache.lock().unwrap();
            let loaded = guard.as_ref().ok_or_else(|| {
                TranscriptionError::ModelError("Model failed to stay cached".to_string())
            })?;
            let ctx = &loaded.context;

            let mut state = ctx
                .create_state()
                .map_err(|e| TranscriptionError::ModelError(e.to_string()))?;

            // Configure transcription parameters
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(language.as_deref());
            if let Some(p) = &prompt {
                params.set_initial_prompt(p);
            }
            params.set_translate(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_special(false);
            params.set_print_timestamps(false);

            // Run transcription
            state
                .full(params, &samples)
                .map_err(|e| TranscriptionError::ModelError(e.to_string()))?;

            // Extract text
            let num_segments = state.full_n_segments();
            let mut result = String::new();
            for i in 0..num_segments {
                if let Some(segment) = state.get_segment(i) {
                    if let Ok(segment_text) = segment.to_str_lossy() {
                        result.push_str(&segment_text);
                    }
                }
            }

            Ok(result.trim().to_string())
        });

        let wait_result = tokio::time::timeout(
            Duration::from_secs(TRANSCRIBE_TIMEOUT_SECS + TRANSCRIBE_ABORT_GRACE_SECS),
            inference,
        )
        .await;

        match wait_result {
            Ok(Ok(Ok(text))) => {
                log_info!(
                    "Local transcription: completed in {:.2}s ({} chars)",
                    start.elapsed().as_secs_f64(),
                    text.len()
                );
                Ok(text)
            }
            Ok(Ok(Err(error))) => {
                log_warn!(
                    "Local transcription: failed after {:.2}s: {}",
                    start.elapsed().as_secs_f64(),
                    error
                );
                Err(error)
            }
            Ok(Err(join_error)) => {
                log_warn!(
                    "Local transcription: worker task panicked after {:.2}s: {}",
                    start.elapsed().as_secs_f64(),
                    join_error
                );
                Err(TranscriptionError::ModelError(
                    "Transcription worker crashed unexpectedly".to_string(),
                ))
            }
            Err(_elapsed) => {
                log_warn!(
                    "Local transcription: timed out after {:.2}s (worker may still be running in the background)",
                    start.elapsed().as_secs_f64()
                );
                Err(TranscriptionError::ModelError(format!(
                    "Transcription timed out after {}s. The model may be too large for this hardware, GPU acceleration may be stuck, or the audio device is misbehaving.",
                    TRANSCRIBE_TIMEOUT_SECS
                )))
            }
        }
    }

    fn service_name(&self) -> &'static str {
        "Local Whisper"
    }
}
