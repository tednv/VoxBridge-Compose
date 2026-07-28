use crate::{gpu_info, hardware_report, history, model_manager, transcription, AppState};
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::Emitter;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatsSnapshot {
    pub bytes_recorded: u64,
    pub words_transcribed: u64,
    pub transcriptions_count: u64,
    pub transcribe_ms_total: u64,
    pub gpu_count: u64,
    pub cpu_count: u64,
    pub api_count: u64,
    pub compose: crate::compose::ComposeStatsSnapshot,
}

/// Session-scoped (resets on app restart) counters for the Debug panel's "Session
/// Stats" - a lightweight at-a-glance view, not a persisted history.
#[tauri::command]
pub async fn get_session_stats(state: tauri::State<'_, AppState>) -> Result<SessionStatsSnapshot, String> {
    let stats = &state.session_stats;
    Ok(SessionStatsSnapshot {
        bytes_recorded: stats.bytes_recorded.load(Ordering::Relaxed),
        words_transcribed: stats.words_transcribed.load(Ordering::Relaxed),
        transcriptions_count: stats.transcriptions_count.load(Ordering::Relaxed),
        transcribe_ms_total: stats.transcribe_ms_total.load(Ordering::Relaxed),
        gpu_count: stats.gpu_count.load(Ordering::Relaxed),
        cpu_count: stats.cpu_count.load(Ordering::Relaxed),
        api_count: stats.api_count.load(Ordering::Relaxed),
        compose: crate::compose::compose_stats_snapshot(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct WhisperPreloadStatus {
    pub loading: bool,
    pub error: Option<String>,
}

/// Pull-based read of the model-loading state, for the frontend to sync on mount.
/// The `whisper-preload-status` event alone isn't enough: preload can start (and finish,
/// for a fast/cached load) before the webview's JS has registered its event listener,
/// which would otherwise leave the UI stuck showing a stale "Ready" state.
#[tauri::command]
pub async fn get_whisper_preload_status(
    state: tauri::State<'_, AppState>,
) -> Result<WhisperPreloadStatus, String> {
    Ok(WhisperPreloadStatus {
        loading: *state.whisper_loading.lock().unwrap(),
        error: state.whisper_preload_error.lock().unwrap().clone(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuVramCheck {
    pub vulkan_runtime_available: bool,
    pub gpu_detected: bool,
    pub adapter_name: Option<String>,
    pub available_vram_bytes: Option<u64>,
    pub dedicated_vram_bytes: Option<u64>,
    pub required_estimate_bytes: u64,
    /// (required_estimate_bytes / available_vram_bytes) * 100, for a UI usage bar.
    /// `None` when we couldn't determine available VRAM at all.
    pub usage_percent: Option<f64>,
    pub supported: bool,
    pub reason: Option<String>,
}

/// Estimate of how much VRAM whisper.cpp needs beyond the raw model file size
/// (KV cache, activations, compute buffers). This is a rule-of-thumb margin, not
/// an exact figure — actual usage varies by model and context settings.
const VRAM_OVERHEAD_MULTIPLIER: f64 = 1.3;
/// Leave some headroom for the OS/other apps rather than declaring a model "supported"
/// right up to 100% of the reported budget.
const MAX_SAFE_VRAM_USAGE_PERCENT: f64 = 90.0;

/// Checks whether this machine can realistically run GPU-accelerated transcription
/// for a *specific* model, so the UI can disable the toggle (with a concrete reason
/// and a usage percentage) instead of letting users flip a switch that silently falls
/// back to CPU or OOMs on an underpowered card.
#[tauri::command]
pub async fn check_gpu_vram(model_size: String) -> Result<GpuVramCheck, String> {
    let models = model_manager::ModelManager::get_available_models();
    let model_info = models
        .iter()
        .find(|m| m.size == model_size)
        .ok_or_else(|| format!("Model size {} not found", model_size))?;

    let required_estimate_bytes = (model_info.file_size as f64 * VRAM_OVERHEAD_MULTIPLIER) as u64;
    let vulkan_runtime_available = gpu_info::vulkan_runtime_available();

    if !vulkan_runtime_available {
        return Ok(GpuVramCheck {
            vulkan_runtime_available,
            gpu_detected: false,
            adapter_name: None,
            available_vram_bytes: None,
            dedicated_vram_bytes: None,
            required_estimate_bytes,
            usage_percent: None,
            supported: false,
            reason: Some(
                "Vulkan runtime not found. Update your GPU drivers to enable GPU acceleration."
                    .to_string(),
            ),
        });
    }

    match gpu_info::get_primary_gpu_vram_info() {
        Some(info) => {
            // Compare against the GPU's total dedicated VRAM (a fixed hardware spec),
            // not live "currently free" memory. Live free memory already reflects
            // whatever the Whisper model itself has resident once GPU mode is in use -
            // comparing against it here would mean the model's own footprint counts
            // against itself, making this check increasingly (and wrongly) pessimistic
            // the longer GPU mode has already been working correctly.
            let capacity_bytes = if info.dedicated_vram_bytes > 0 {
                info.dedicated_vram_bytes
            } else {
                info.available_vram_bytes
            };
            let usage_percent = if capacity_bytes > 0 {
                (required_estimate_bytes as f64 / capacity_bytes as f64) * 100.0
            } else {
                f64::INFINITY
            };
            let supported = usage_percent <= MAX_SAFE_VRAM_USAGE_PERCENT;
            let reason = if !supported {
                Some(format!(
                    "The '{}' model needs an estimated {:.1} GB of VRAM ({}% of the {:.1} GB on {}) \
                     — too much for this GPU.",
                    model_size,
                    required_estimate_bytes as f64 / 1_073_741_824.0,
                    if usage_percent.is_finite() { format!("{:.0}", usage_percent) } else { ">100".to_string() },
                    capacity_bytes as f64 / 1_073_741_824.0,
                    info.adapter_name
                ))
            } else {
                None
            };

            Ok(GpuVramCheck {
                vulkan_runtime_available,
                gpu_detected: true,
                adapter_name: Some(info.adapter_name),
                available_vram_bytes: Some(info.available_vram_bytes),
                dedicated_vram_bytes: Some(info.dedicated_vram_bytes),
                required_estimate_bytes,
                usage_percent: if usage_percent.is_finite() {
                    Some(usage_percent)
                } else {
                    None
                },
                supported,
                reason,
            })
        }
        None => Ok(GpuVramCheck {
            vulkan_runtime_available,
            gpu_detected: false,
            adapter_name: None,
            available_vram_bytes: None,
            dedicated_vram_bytes: None,
            required_estimate_bytes,
            usage_percent: None,
            supported: false,
            reason: Some("No compatible GPU adapter detected.".to_string()),
        }),
    }
}

/// Builds a plain-text hardware report (CPU/GPU/OS/Vulkan availability, plus the most
/// recent GPU transcription failure, if any) for the user to attach to a bug report. This
/// is the "submit hardware info so maintainers can build a profile for it" half of the
/// GPU graceful-fallback system — see `local_whisper::ensure_model_loaded_with_fallback`.
#[tauri::command]
pub async fn get_hardware_report(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let last_gpu_error = state.whisper_last_gpu_error.lock().unwrap().clone();
    Ok(hardware_report::build_hardware_report(
        last_gpu_error.as_deref(),
    ))
}

/// Dev/verification-only command: exercises the full VoxBridge engine pipeline
/// (resource-dir resolution -> CPUID variant selection -> dlopen -> transcribe) from
/// inside the actual running/packaged app, so the build pipeline can be confirmed
/// end-to-end without a separate standalone harness. NOT the production transcription
/// path yet - see `local_whisper.rs`'s VoxBridge-with-whisper-rs-fallback wiring.
#[tauri::command]
pub async fn test_engine_loader(
    app_handle: tauri::AppHandle,
    model_path: String,
    wav_path: String,
) -> Result<String, String> {
    use tauri::Manager;
    use std::path::Path;

    let resource_base = app_handle.path().resource_dir().ok();
    let dev_fallback_base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidate_bases: Vec<&Path> = resource_base
        .iter()
        .map(|p| p.as_path())
        .chain(std::iter::once(dev_fallback_base))
        .collect();
    let engines_dir = voxbridge::resolve_engines_dir(&candidate_bases)
        .ok_or_else(|| "No engine variants found (neither bundled resources nor dev engines-dist/ present)".to_string())?;

    let engine = voxbridge::Engine::load_best(&engines_dir)?;
    let variant = engine.variant_name().to_string();

    let mut reader = hound::WavReader::open(&wav_path).map_err(|e| e.to_string())?;
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as f32 / 32768.0))
        .collect::<Result<_, _>>()
        .map_err(|e: hound::Error| e.to_string())?;

    let model = engine.load_model(&model_path)?;
    let text = model.transcribe(&samples, Some("en"), None)?;

    Ok(format!("[engine={}] {}", variant, text))
}

#[tauri::command]
pub async fn test_api_key(api_key: String, api_url: String) -> Result<bool, String> {
    transcription::test_api_key(&api_key, &api_url)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_available_engines() -> Result<Vec<String>, String> {
    Ok(model_manager::ModelManager::get_available_engines())
}

#[tauri::command]
pub async fn get_available_models() -> Result<Vec<model_manager::ModelInfo>, String> {
    Ok(model_manager::ModelManager::get_available_models())
}

#[tauri::command]
pub async fn check_model_status(model_size: String) -> Result<bool, String> {
    let manager = model_manager::ModelManager::new().map_err(|error| error.to_string())?;
    Ok(manager.is_model_downloaded(&model_size))
}

#[tauri::command]
pub async fn download_model(
    model_size: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let manager = model_manager::ModelManager::new().map_err(|error| error.to_string())?;

    manager
        .download_model(&model_size, {
            let app_handle = app_handle.clone();
            move |progress| {
                let _ = app_handle.emit("model-download-progress", progress);
            }
        })
        .await?;

    // Now that the model is actually on disk, warm it up if it's the one currently
    // selected - `spawn_whisper_preload` was already skipped for this model while it
    // was undownloaded, so this is what actually loads it into memory.
    crate::app::bootstrap::spawn_whisper_preload(app_handle);

    Ok(())
}

#[tauri::command]
pub fn get_current_status() -> String {
    crate::app::status::get_current_status()
}

#[tauri::command]
pub async fn get_history(state: tauri::State<'_, AppState>) -> Result<history::History, String> {
    let retention_days = state.config.lock().unwrap().history_retention_days;
    history::prune_history(retention_days).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn clear_history() -> Result<(), String> {
    history::clear_history().map_err(|error| error.to_string())
}
