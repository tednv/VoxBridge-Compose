use crate::config::{self, Config, TranscriptionMode};
use crate::{audio, history, local_whisper, transcription, typing};
use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};

static LAST_RECORDING_LOG: LazyLock<Mutex<Option<std::path::PathBuf>>> =
    LazyLock::new(|| Mutex::new(None));

pub fn take_last_recording_log() -> Option<std::path::PathBuf> {
    LAST_RECORDING_LOG.lock().unwrap().take()
}

fn validate_audio_duration(
    audio_data: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if audio_data.len() < 44 {
        return Err("Audio file too small".into());
    }
    let sample_rate = u32::from_le_bytes([
        audio_data[24],
        audio_data[25],
        audio_data[26],
        audio_data[27],
    ]);
    let channels = u16::from_le_bytes([audio_data[22], audio_data[23]]);
    let bits_per_sample = u16::from_le_bytes([audio_data[34], audio_data[35]]);

    let mut data_size = 0u32;
    let mut pos = 36;
    while pos + 8 <= audio_data.len() {
        let chunk_id = &audio_data[pos..pos + 4];
        let chunk_size = u32::from_le_bytes([
            audio_data[pos + 4],
            audio_data[pos + 5],
            audio_data[pos + 6],
            audio_data[pos + 7],
        ]);
        if chunk_id == b"data" {
            data_size = chunk_size;
            break;
        }
        pos += 8 + chunk_size as usize;
        if chunk_size % 2 == 1 {
            pos += 1;
        }
    }

    if data_size == 0 {
        return Err("No data chunk".into());
    }
    let bytes_per_sample = (bits_per_sample / 8) as u32;
    let bytes_per_second = sample_rate * channels as u32 * bytes_per_sample;
    let duration_seconds = data_size as f64 / bytes_per_second as f64;

    crate::log_info!("Audio duration: {:.3}s", duration_seconds);
    if duration_seconds < 0.1 {
        return Err("Audio too short".into());
    }
    Ok(())
}

pub async fn record_and_transcribe(
    config: Arc<Mutex<Config>>,
    is_recording: Arc<Mutex<bool>>,
    app_handle: AppHandle,
    audio_engine: Arc<Mutex<Option<audio::PersistentAudioEngine>>>,
    whisper_engine: local_whisper::WhisperEngineCache,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let reset_status_on_exit = || async {
        crate::app::status::emit_status_to_frontend("Ready").await;
    };

    let audio_data = match audio::record_audio_while_flag(&is_recording, audio_engine, {
        let config_guard = config.lock().unwrap();
        config_guard.post_roll_ms
    })
    .await
    {
        Ok(data) => data,
        Err(error) => {
            reset_status_on_exit().await;
            return Err(error);
        }
    };

    if audio_data.is_empty() {
        reset_status_on_exit().await;
        return Ok(());
    }
    if let Err(error) = validate_audio_duration(&audio_data) {
        crate::log_info!("Audio validation failed: {}", error);
        reset_status_on_exit().await;
        return Ok(());
    }

    transcribe_and_deliver(audio_data, config, app_handle, whisper_engine, "Ready", None).await
}

/// Continuous/"always listening" mode: instead of one recording bounded by hotkey
/// press/release, this keeps the mic open and uses energy-based Voice Activity
/// Detection (`audio::listen_continuously`) to auto-segment speech into utterances,
/// transcribing (and typing) each one as soon as it's detected - so the user can just
/// keep talking, pause, keep talking, without touching the hotkey again until they want
/// to stop the whole session.
///
/// Utterances are transcribed concurrently with continued listening (each spawned as its
/// own task) rather than blocking the capture loop, so a slow transcription doesn't cause
/// the next utterance's audio to be dropped or delayed.
pub async fn continuous_listen_and_transcribe(
    config: Arc<Mutex<Config>>,
    is_recording: Arc<Mutex<bool>>,
    app_handle: AppHandle,
    audio_engine: Arc<Mutex<Option<audio::PersistentAudioEngine>>>,
    whisper_engine: local_whisper::WhisperEngineCache,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    crate::app::status::emit_status_to_frontend("Recording").await;

    let silence_end_ms = { config.lock().unwrap().continuous_silence_ms };
    let utterance_rx =
        audio::listen_continuously(is_recording.clone(), audio_engine, silence_end_ms)?;

    // Shared across utterances in this one continuous session: lets each new utterance
    // (a) get fed the tail of what was just said as Whisper's own "initial prompt" (a
    // real whisper.cpp feature - it conditions decoding on preceding text, helping with
    // continuity/disambiguation instead of each utterance being transcribed in a vacuum),
    // and (b) know whether the previous utterance ended a sentence, so the joined text
    // gets sensible spacing/capitalization instead of reading like disconnected
    // fragments. Utterances are still transcribed concurrently, so under heavy overlap
    // (a slow one finishing after a faster later one) this ordering is best-effort, not
    // guaranteed.
    let session_context = Arc::new(Mutex::new(String::new()));

    let spawn_utterance = |audio_data: Vec<u8>| {
        if let Err(error) = validate_audio_duration(&audio_data) {
            crate::log_info!("Continuous listen: utterance skipped ({})", error);
            return None;
        }
        let config = config.clone();
        let app_handle = app_handle.clone();
        let whisper_engine = whisper_engine.clone();
        let session_context = session_context.clone();
        Some(tauri::async_runtime::spawn(async move {
            if let Err(error) = transcribe_and_deliver(
                audio_data,
                config,
                app_handle,
                whisper_engine,
                "Recording",
                Some(session_context),
            )
            .await
            {
                crate::log_info!("Continuous listen: utterance transcription failed: {}", error);
            }
        }))
    };

    let mut transcription_tasks = Vec::new();
    loop {
        let still_recording = *is_recording.lock().unwrap();
        match utterance_rx.try_recv() {
            Ok(wav) => {
                if let Some(task) = spawn_utterance(wav) {
                    transcription_tasks.push(task);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if !still_recording {
                    // Hotkey pressed again to stop: keep draining (with a short blocking
                    // wait) until the capture thread finishes flushing its last
                    // in-progress utterance and closes the channel, so nothing said
                    // right before stopping gets dropped.
                    while let Ok(wav) =
                        utterance_rx.recv_timeout(std::time::Duration::from_millis(500))
                    {
                        if let Some(task) = spawn_utterance(wav) {
                            transcription_tasks.push(task);
                        }
                    }
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        }
    }

    // Do not announce a completed session while its final rolling windows are still
    // transcribing. Once all captured speech has arrived, schedule one authoritative
    // whole-document reconciliation; the UI waits for that correction before asking
    // the model for the final topic-based filename.
    for task in transcription_tasks {
        let _ = task.await;
    }
    crate::app::status::emit_status_to_frontend("Finalizing refinement").await;
    crate::compose::rerun_current_document(&app_handle);
    Ok(())
}

/// Shared tail for both single-shot (`record_and_transcribe`) and per-utterance
/// (`continuous_listen_and_transcribe`) capture: transcribes `audio_data` and injects the
/// result. `idle_status` is what the status overlay returns to when this call finishes -
/// "Ready" for a single recording, "Recording" for continuous mode (since listening is
/// still ongoing for the next utterance). `session_context`, when present (continuous
/// mode only), carries the tail of what's already been said this session - both as
/// Whisper's own initial-prompt context and to decide how this utterance's text should
/// be joined onto what came before.
async fn transcribe_and_deliver(
    audio_data: Vec<u8>,
    config: Arc<Mutex<Config>>,
    app_handle: AppHandle,
    whisper_engine: local_whisper::WhisperEngineCache,
    idle_status: &str,
    session_context: Option<Arc<Mutex<String>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let reset_status_on_exit = || async {
        crate::app::status::emit_status_to_frontend(idle_status).await;
    };

    let session_stats = app_handle.state::<crate::AppState>().session_stats.clone();
    session_stats
        .bytes_recorded
        .fetch_add(audio_data.len() as u64, Ordering::Relaxed);

    let (
        transcription_mode,
        api_key,
        api_url,
        api_model,
        enable_recording_logs,
        recording_log_retention_days,
        enable_history,
        history_retention_days,
        enable_gpu,
        custom_vocabulary,
    ) = {
        let config_guard = config.lock().unwrap();
        (
            config_guard.transcription_mode.clone(),
            config_guard.openai_api_key.clone(),
            config_guard.api_url.clone(),
            config_guard.api_model.clone(),
            config_guard.enable_recording_logs,
            config_guard.recording_log_retention_days,
            config_guard.enable_history,
            config_guard.history_retention_days,
            config_guard.enable_gpu,
            config_guard.custom_vocabulary.clone(),
        )
    };

    let transcribing_status = match transcription_mode {
        TranscriptionMode::API => "Transcribing (API)",
        TranscriptionMode::Local if enable_gpu => "Recognizing speech (graphics)",
        TranscriptionMode::Local => "Recognizing speech (processor)",
    };
    crate::app::status::emit_status_to_frontend(transcribing_status).await;

    // Always auto-detect language - the old per-locale "hint" (e.g. forcing "en" plus a
    // prompt like "Australian spelling.") didn't reliably steer whisper's output and just
    // added a confusing setting. The one real exception is English-only ("*.en") ggml
    // models below, which have no other-language vocabulary at all and require "en".
    let mut lang_code: Option<&str> = None;
    // NOTE: previously fed the running session context to whisper as an initial-prompt
    // here (a real whisper.cpp feature), but in practice with VoxBridge's GPU path this
    // caused a ~40x slowdown (0.46s -> 20.2s) on the very next utterance and produced
    // hallucinated, unrelated text - the well-documented "long/odd prompt causes
    // hallucination" failure mode. That was specifically the *dynamic, ever-growing*
    // prompt; a short, fixed custom-vocabulary list (names/jargon the user configures
    // once in Settings) is a fundamentally different shape - same tiny prompt every
    // call, not a longer one each time - so it doesn't hit that failure mode.
    let custom_vocabulary_trimmed = custom_vocabulary.trim();
    let prompt_hint: Option<&str> = if custom_vocabulary_trimmed.is_empty() {
        None
    } else {
        Some(custom_vocabulary_trimmed)
    };

    if enable_history && enable_recording_logs {
        let debug_dir = dirs::config_dir()
            .unwrap_or_default()
            .join("foss-voquill")
            .join("debug");
        let retention = std::time::Duration::from_secs(
            recording_log_retention_days.max(1).saturating_mul(86_400),
        );
        if let Ok(entries) = std::fs::read_dir(&debug_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_recording = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("recording_") && name.ends_with(".wav"));
                let expired = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
                    .is_ok_and(|age| age > retention);
                if is_recording && expired {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        let debug_path = debug_dir.join(format!(
                "recording_{}.wav",
                ::chrono::Local::now().format("%Y%m%d_%H%M%S")
            ));

        if let Err(error) = std::fs::create_dir_all(debug_path.parent().unwrap()) {
            crate::log_info!("Failed to create debug directory: {}", error);
        } else if let Err(error) = std::fs::write(&debug_path, &audio_data) {
            crate::log_info!("Failed to save debug recording: {}", error);
        } else {
            *LAST_RECORDING_LOG.lock().unwrap() = Some(debug_path.clone());
            crate::log_info!("Debug recording saved to: {:?}", debug_path);
        }
    }

    crate::log_info!("Transcription Mode: {:?}", transcription_mode);
    crate::log_info!("Language: {:?}, Hint: {:?}", lang_code, prompt_hint);

    let service: Box<dyn transcription::TranscriptionService + Send + Sync> =
        match transcription_mode {
            TranscriptionMode::API => Box::new(transcription::APITranscriptionService {
                api_key,
                api_url,
                api_model,
            }),
            TranscriptionMode::Local => {
                let (model_size, use_gpu, local_engine) = {
                    let config_lock = config.lock().unwrap();
                    (
                        config_lock.local_model_size.clone(),
                        config_lock.enable_gpu,
                        config_lock.local_engine.clone(),
                    )
                };

                // English-only ("*.en") ggml models have no multilingual vocabulary, so
                // whisper.cpp's language auto-detection is invalid for them — attempting
                // it produces near-random results (e.g. "detected Danish" at ~1%
                // confidence) which corrupts the decoder state for the transcription
                // that follows. Force English rather than passing through "auto"/None.
                if model_size.ends_with(".en") && lang_code.is_none() {
                    crate::log_info!(
                        "Local model '{}' is English-only; overriding language 'auto' -> 'en' (auto-detect is unsupported on .en models)",
                        model_size
                    );
                    lang_code = Some("en");
                }

                let app_state = app_handle.try_state::<crate::app::state::AppState>();
                let last_gpu_error = app_state.as_ref().map(|state| state.whisper_last_gpu_error.clone());
                // Only attempt VoxBridge when it's explicitly selected in Settings ->
                // Local Engine - keeps "Whisper.cpp" a clean, unmodified whisper-rs-only
                // path for direct A/B comparison, rather than VoxBridge silently trying
                // itself first regardless of the user's choice.
                let (voxbridge_cache, voxbridge_resource_base) = if local_engine == "VoxBridge" {
                    (
                        app_state.as_ref().map(|state| state.voxbridge_engine.clone()),
                        app_handle.path().resource_dir().ok(),
                    )
                } else {
                    (None, None)
                };

                match local_whisper::LocalWhisperService::new_full(
                    whisper_engine.clone(),
                    &model_size,
                    use_gpu,
                    last_gpu_error,
                    voxbridge_cache,
                    voxbridge_resource_base,
                ) {
                    Ok(service) => Box::new(service),
                    Err(error) => {
                        crate::log_info!("Failed to initialize Local Whisper: {}", error);
                        reset_status_on_exit().await;
                        return Err(error.into());
                    }
                }
            }
        };

    let transcribe_start = Instant::now();
    let text = match service
        .transcribe(&audio_data, lang_code, prompt_hint)
        .await
    {
        Ok(text) => {
            crate::log_info!(
                "Transcription received ({}): \"{}\"",
                service.service_name(),
                text
            );

            session_stats
                .transcribe_ms_total
                .fetch_add(transcribe_start.elapsed().as_millis() as u64, Ordering::Relaxed);
            session_stats.transcriptions_count.fetch_add(1, Ordering::Relaxed);
            session_stats
                .words_transcribed
                .fetch_add(text.split_whitespace().count() as u64, Ordering::Relaxed);
            match transcription_mode {
                TranscriptionMode::API => session_stats.api_count.fetch_add(1, Ordering::Relaxed),
                TranscriptionMode::Local if enable_gpu => {
                    session_stats.gpu_count.fetch_add(1, Ordering::Relaxed)
                }
                TranscriptionMode::Local => session_stats.cpu_count.fetch_add(1, Ordering::Relaxed),
            };

            text
        }

        Err(error) => {
            crate::log_info!(
                "Transcription failed ({}): {}",
                service.service_name(),
                error
            );
            reset_status_on_exit().await;
            return Err(error.into());
        }
    };

    // Continuous mode only: each utterance is transcribed independently, so Whisper has
    // no idea whether it's starting a fresh sentence or continuing mid-thought - left
    // alone that reads as a string of disconnected, all-capitalized fragments. Use
    // whether the previous utterance ended with sentence-final punctuation to decide
    // spacing/capitalization for this one, and extend the running context for the next
    // utterance's prompt above.
    let text = if let Some(ctx) = &session_context {
        let trimmed_new = text.trim();
        if trimmed_new.is_empty() {
            text
        } else {
            let mut ctx_guard = ctx.lock().unwrap();
            let previous_ended_sentence =
                ctx_guard.is_empty() || ctx_guard.trim_end().ends_with(['.', '!', '?']);

            let mut joined = String::new();
            if !ctx_guard.is_empty() {
                joined.push(' ');
            }

            let mut chars = trimmed_new.chars();
            match chars.next() {
                Some(first) if previous_ended_sentence => {
                    joined.push(first.to_ascii_uppercase());
                    joined.push_str(chars.as_str());
                }
                Some(first) if trimmed_new.split_whitespace().next() != Some("I") => {
                    // Whisper capitalizes the first word of every independent utterance
                    // by default; mid-sentence that reads wrong, so undo it - except for
                    // "I", which is always capitalized regardless of position.
                    joined.push(first.to_ascii_lowercase());
                    joined.push_str(chars.as_str());
                }
                _ => joined.push_str(trimmed_new),
            }

            ctx_guard.push_str(&joined);
            let overflow = ctx_guard.len().saturating_sub(200);
            if overflow > 0 {
                // Find the nearest char boundary at or after `overflow` - `ctx_guard` is
                // arbitrary transcribed text, so a raw byte-offset slice could otherwise
                // panic mid-UTF-8-character.
                let safe_start = (overflow..ctx_guard.len())
                    .find(|&i| ctx_guard.is_char_boundary(i))
                    .unwrap_or(ctx_guard.len());
                let truncated = ctx_guard[safe_start..].to_string();
                *ctx_guard = truncated;
            }

            joined
        }
    } else {
        text
    };

    if !text.trim().is_empty() {
        if enable_history {
            let _ = history::add_history_item(&text, history_retention_days);
            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.emit("history-updated", ());
            }
        }

        let output_method = { config.lock().unwrap().output_method.clone() };

        if output_method == config::OutputMethod::Compose {
            // Compose doesn't touch external apps at all - no hardware typing race to
            // guard against, so skip the "Typing" status/sleep dance used by the other
            // two methods below.
            let state = app_handle.state::<crate::AppState>();
            crate::compose::append_and_correct(
                text.clone(),
                state.compose_buffer.clone(),
                state.compose_raw_buffer.clone(),
                state.compose_pending.clone(),
                state.compose_history.clone(),
                state.compose_backend.clone(),
                config.clone(),
                app_handle.clone(),
            );
        } else {
            crate::app::status::emit_status_to_frontend("Typing").await;
            let (typing_speed, hold_duration, copy_on_typewriter) = {
                let config_guard = config.lock().unwrap();
                (
                    config_guard.typing_speed_interval,
                    config_guard.key_press_duration_ms,
                    config_guard.copy_on_typewriter,
                )
            };

            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

            match output_method {
                config::OutputMethod::Typewriter => {
                    if copy_on_typewriter {
                        if let Err(error) = typing::copy_to_clipboard(&text) {
                            crate::log_info!("CLIPBOARD ERROR: {}", error);
                        }
                    }
                    crate::log_info!("Forwarding text to hardware typing engine...");
                    let state = app_handle.state::<crate::AppState>();
                    if let Err(error) = state
                        .display_backend
                        .type_text_hardware(&app_handle, &text, typing_speed, hold_duration)
                        .await
                    {
                        crate::log_info!("TYPING ENGINE ERROR: {}", error);
                    }
                }
                config::OutputMethod::Clipboard => {
                    crate::log_info!("Copying text to clipboard (Clipboard Mode)...");
                    if let Err(error) = typing::copy_to_clipboard(&text) {
                        crate::log_info!("CLIPBOARD ERROR: {}", error);
                    }
                }
                config::OutputMethod::Compose => unreachable!(),
            }
        }
    } else {
        crate::log_info!("Transcription was empty, skipping typing.");
    }

    reset_status_on_exit().await;
    Ok(())
}
