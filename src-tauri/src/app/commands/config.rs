use crate::app::commands::hotkey::re_register_hotkey;
use crate::config::Config;
use crate::{audio, config, history, AppState};
use tauri::Emitter;

#[tauri::command]
pub async fn get_config(state: tauri::State<'_, AppState>) -> Result<Config, String> {
    let config = state.config.lock().unwrap();
    Ok(config.clone())
}

/// Persists where the user dragged the overlay pill to, so it stays there instead of
/// resetting to the default corner every time it's shown.
#[tauri::command]
pub async fn save_overlay_position(
    x: i32,
    y: i32,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let updated_config = {
        let mut config_guard = state.config.lock().unwrap();
        config_guard.overlay_position = Some((x, y));
        config_guard.clone()
    };
    config::save_config(&updated_config).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn save_config(
    new_config: Config,
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut normalized_config = new_config;
    normalized_config.normalize_input_sensitivity();

    let is_mic_test_active = *state.is_mic_test_active.lock().unwrap();

    let (restart_engine, hotkey_changed, whisper_settings_changed, compose_settings_changed, compose_backend_changed, history_settings_changed, mut merged_config) = {
        let config_guard = state.config.lock().unwrap();
        let audio_changed = config_guard.audio_device != normalized_config.audio_device
            || config_guard.input_sensitivity != normalized_config.input_sensitivity;
        let hotkey_changed = config_guard.hotkey != normalized_config.hotkey;
        let whisper_settings_changed = config_guard.local_model_size
            != normalized_config.local_model_size
            || config_guard.local_engine != normalized_config.local_engine
            || config_guard.enable_gpu != normalized_config.enable_gpu
            || config_guard.transcription_mode != normalized_config.transcription_mode;
        let compose_settings_changed = config_guard.output_method
            != normalized_config.output_method
            || config_guard.compose_backend != normalized_config.compose_backend
            || config_guard.compose_use_gpu != normalized_config.compose_use_gpu
            || config_guard.compose_model_path != normalized_config.compose_model_path
            || config_guard.compose_ollama_url != normalized_config.compose_ollama_url
            || config_guard.compose_ollama_model != normalized_config.compose_ollama_model;
        let compose_backend_changed = config_guard.compose_backend != normalized_config.compose_backend;
        let history_settings_changed = config_guard.enable_history != normalized_config.enable_history
            || config_guard.history_retention_days != normalized_config.history_retention_days;

        let mut merged_config = normalized_config.clone();
        if merged_config.shortcuts_token.is_none() {
            merged_config.shortcuts_token = config_guard.shortcuts_token.clone();
        }
        if merged_config.input_token.is_none() {
            merged_config.input_token = config_guard.input_token.clone();
        }
        // compose_agents is deliberately not part of the frontend's general Config
        // state/autosave - it's managed entirely through the dedicated
        // save_compose_agents/save_agent_preset commands, so any request coming
        // through this general path never actually carries the caller's real agent
        // list. Without this, a periodic autosave of something unrelated (any other
        // setting changing) would silently reset every custom agent/prompt/fidelity
        // threshold back to serde's single-default-agent fallback, since the field is
        // simply absent from that request's JSON body. Always keep whatever's already
        // on the server.
        merged_config.compose_agents = config_guard.compose_agents.clone();

        (
            audio_changed,
            hotkey_changed,
            whisper_settings_changed,
            compose_settings_changed,
            compose_backend_changed,
            history_settings_changed,
            merged_config,
        )
    };

    // Claim a generation before any provider preparation that can await a download or
    // network request. Without this, rapid Embedded -> Ollama -> Embedded changes can
    // finish out of order and an older, slower request can overwrite the user's latest
    // selection.
    let compose_switch_generation = if compose_backend_changed {
        crate::compose::invalidate_backend(
            &state.compose_backend,
            &state.compose_backend_generation,
        );
        Some(
            state
                .compose_backend_generation
                .load(std::sync::atomic::Ordering::SeqCst),
        )
    } else {
        None
    };

    if compose_backend_changed {
        *state.compose_loading.lock().unwrap() = true;
        let _ = app_handle.emit(
            "compose-preload-status",
            serde_json::json!({ "loading": true, "error": null }),
        );
        crate::app::status::emit_status_to_frontend("Preparing refinement model").await;

        let preparation = if merged_config.compose_backend == "embedded" {
            let configured = std::path::Path::new(&merged_config.compose_model_path);
            if merged_config.compose_model_path.trim().is_empty()
                || !configured.exists()
                || crate::app::commands::compose::uses_legacy_default_embedded_model(configured)
            {
                crate::app::commands::compose::ensure_default_embedded_model()
                    .await
                    .map(|path| merged_config.compose_model_path = path)
            } else {
                Ok(())
            }
        } else {
            if merged_config.compose_ollama_model.trim().is_empty() {
                merged_config.compose_ollama_model =
                    crate::app::commands::compose::DEFAULT_OLLAMA_MODEL.to_string();
            }
            crate::app::commands::compose::ensure_ollama_model(
                &merged_config.compose_ollama_url,
                &merged_config.compose_ollama_model,
            )
            .await
        };

        if let Err(error) = preparation {
            if compose_switch_generation
                != Some(
                    state
                        .compose_backend_generation
                        .load(std::sync::atomic::Ordering::SeqCst),
                )
            {
                crate::log_info!(
                    "Compose: ignored preparation failure from a superseded provider switch"
                );
                return Ok(());
            }
            *state.compose_loading.lock().unwrap() = false;
            *state.compose_preload_error.lock().unwrap() = Some(error.clone());
            let _ = app_handle.emit(
                "compose-preload-status",
                serde_json::json!({ "loading": false, "error": error }),
            );
            return Err(error);
        }

        if compose_switch_generation
            != Some(
                state
                    .compose_backend_generation
                    .load(std::sync::atomic::Ordering::SeqCst),
            )
        {
            crate::log_info!("Compose: discarded a superseded provider switch");
            return Ok(());
        }
    }

    let mut prepared_device: Option<cpal::Device> = None;
    let mut prepared_engine: Option<audio::PersistentAudioEngine> = None;

    if restart_engine {
        if is_mic_test_active {
            let selected_device = merged_config
                .audio_device
                .clone()
                .unwrap_or_else(|| "default".to_string());
            crate::log_warn!(
                "Audio config change rejected while mic test is active (requested_device='{}', sensitivity={:.2})",
                selected_device,
                merged_config.input_sensitivity
            );
            return Err(
                "Cannot change audio settings while mic test is active. Stop mic test and try again."
                    .to_string(),
            );
        }

        let selected_device = merged_config.audio_device.clone();
        let resolved_device = audio::lookup_device(selected_device.clone()).map_err(|error| {
            format!(
                "Failed to resolve input device '{}': {}",
                selected_device.unwrap_or_else(|| "default".to_string()),
                error
            )
        })?;

        crate::log_info!(
            "Audio config changed, validating persistent engine restart (requested_device='{}', sensitivity={:.2})",
            merged_config
                .audio_device
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            merged_config.input_sensitivity
        );

        let new_engine = audio::PersistentAudioEngine::new(
            &resolved_device,
            merged_config.input_sensitivity,
        )
        .map_err(|error| {
            format!(
                "Failed to initialize persistent audio engine for device '{}' (sensitivity {:.2}): {}",
                merged_config
                    .audio_device
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
                merged_config.input_sensitivity,
                error
            )
        })?;

        prepared_device = Some(resolved_device);
        prepared_engine = Some(new_engine);
    }

    {
        let mut config_guard = state.config.lock().unwrap();
        *config_guard = merged_config.clone();
    }
    crate::set_persistence_enabled(merged_config.enable_history);

    if restart_engine {
        {
            let mut cached_device = state.cached_device.lock().unwrap();
            *cached_device = prepared_device;
        }
        {
            let mut engine_guard = state.audio_engine.lock().unwrap();
            *engine_guard = prepared_engine;
        }
        crate::log_info!("Persistent engine restarted");
    } else {
        let cached = match audio::lookup_device(merged_config.audio_device.clone()) {
            Ok(device) => Some(device),
            Err(error) => {
                crate::log_warn!(
                    "Failed to pre-warm audio device cache (requested_device='{}'): {}",
                    merged_config
                        .audio_device
                        .clone()
                        .unwrap_or_else(|| "default".to_string()),
                    error
                );
                None
            }
        };
        let mut cached_device = state.cached_device.lock().unwrap();
        *cached_device = cached;
        crate::log_info!("Pre-warmed audio device cache");
    }

    if let Err(error) = config::save_config(&merged_config) {
        return Err(format!("Failed to save config: {}", error));
    }
    if compose_backend_changed {
        let _ = app_handle.emit("config-updated", merged_config.clone());
    }

    if history_settings_changed {
        history::prune_history(merged_config.history_retention_days)
            .map_err(|error| error.to_string())?;
        let _ = app_handle.emit("history-updated", ());
    }

    if hotkey_changed {
        if let Err(error) = re_register_hotkey(&app_handle, &merged_config.hotkey).await {
            let mut error_lock = state.hotkey_error.lock().unwrap();
            *error_lock = Some(error.clone());
            return Err(format!(
                "Config saved but failed to update hotkey: {}",
                error
            ));
        } else {
            let mut error_lock = state.hotkey_error.lock().unwrap();
            *error_lock = None;
        }
    }

    if whisper_settings_changed {
        // The model/GPU setting changed underneath the cached context; drop it so the
        // next transcription (or the preload we're about to kick off) loads the right one.
        crate::local_whisper::unload_model(&state.whisper_engine);
        crate::app::bootstrap::spawn_whisper_preload(app_handle.clone());
    }

    if compose_settings_changed {
        let has_document = !state.compose_raw_buffer.lock().unwrap().trim().is_empty();
        state
            .compose_rerun_after_preload
            .store(has_document, std::sync::atomic::Ordering::SeqCst);
        // Backend changes already claimed and invalidated their generation before the
        // potentially slow provider preparation above. Other Compose setting changes
        // can invalidate here immediately before their replacement preload.
        if !compose_backend_changed {
            crate::compose::invalidate_backend(
                &state.compose_backend,
                &state.compose_backend_generation,
            );
        }
        crate::compose::spawn_compose_preload(app_handle.clone());
    }

    Ok(())
}

#[tauri::command]
pub async fn reset_application_to_defaults(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    crate::log_info!("Factory reset requested");

    crate::local_whisper::unload_model(&state.whisper_engine);
    *state.whisper_preload_error.lock().unwrap() = None;

    let root_dir = crate::get_app_config_root_dir()?;

    let models_dir = root_dir.join("models");
    if models_dir.exists() {
        std::fs::remove_dir_all(&models_dir).map_err(|error| error.to_string())?;
    }
    std::fs::create_dir_all(&models_dir).map_err(|error| error.to_string())?;

    let debug_dir = root_dir.join("debug");
    std::fs::create_dir_all(&debug_dir).map_err(|error| error.to_string())?;
    crate::clear_directory_contents(&debug_dir, &["session.log", "last-session.log"])?;

    if let Err(error) = crate::truncate_session_log_with_header() {
        crate::log_warn!(
            "Could not truncate session log during factory reset: {}",
            error
        );
    }

    history::clear_history().map_err(|error| error.to_string())?;

    let custom_presets_path = root_dir.join("agent_presets.json");
    if custom_presets_path.exists() {
        std::fs::remove_file(custom_presets_path).map_err(|error| error.to_string())?;
    }

    let default_config = Config::default();
    config::save_config(&default_config).map_err(|error| error.to_string())?;

    {
        let mut config_lock = state.config.lock().unwrap();
        *config_lock = default_config.clone();
    }

    {
        let mut cached_device = state.cached_device.lock().unwrap();
        *cached_device = None;
    }

    {
        let mut mic_test_active = state.is_mic_test_active.lock().unwrap();
        *mic_test_active = false;
    }

    {
        let mut playback_stream = state.playback_stream.lock().unwrap();
        *playback_stream = None;
    }

    {
        let mut dump_dir_override = state.compose_dump_dir_override.lock().unwrap();
        *dump_dir_override = None;
    }

    crate::compose::invalidate_backend(
        &state.compose_backend,
        &state.compose_backend_generation,
    );

    {
        let mut hotkey_error = state.hotkey_error.lock().unwrap();
        *hotkey_error = None;
    }

    if let Err(error) = re_register_hotkey(&app_handle, &default_config.hotkey).await {
        let mut hotkey_error = state.hotkey_error.lock().unwrap();
        *hotkey_error = Some(error.clone());
        return Err(format!(
            "Factory reset completed but failed to re-register default hotkey: {}",
            error
        ));
    }

    crate::app::status::emit_status_to_frontend("Ready").await;

    let _ = app_handle.emit("history-updated", serde_json::json!({ "items": [] }));
    let _ = app_handle.emit("config-updated", default_config.clone());
    let _ = app_handle.emit("setup-status-changed", serde_json::json!({}));

    crate::log_info!("Factory reset completed successfully");
    Ok(())
}
