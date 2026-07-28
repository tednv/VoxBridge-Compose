use crate::app::commands::hotkey::re_register_hotkey;
#[cfg(target_os = "linux")]
use crate::app::commands::platform::is_status_notifier_watcher_available;
use crate::app::state::AppState;
use crate::config::{Config, TranscriptionMode};
use crate::{audio, hotkey, local_whisper};
#[cfg(target_os = "linux")]
use ashpd::{register_host_app, AppID};
use std::sync::{Arc, Mutex};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

#[cfg(target_os = "linux")]
use crate::platform::linux::detection::is_wayland_session;
#[cfg(target_os = "linux")]
use crate::platform::linux::wayland::env::check_wayland_display;

#[cfg(target_os = "linux")]
fn read_linux_distribution_name() -> Option<String> {
    let contents = std::fs::read_to_string("/etc/os-release").ok()?;
    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

fn create_tray_menu(app: &tauri::AppHandle) -> Result<Menu<tauri::Wry>, tauri::Error> {
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let open_item = MenuItem::with_id(app, "open", "Open VoxBridge Compose", true, None::<&str>)?;
    Menu::with_items(app, &[&open_item, &quit_item])
}

pub fn build_app_state(initial_config: &Config) -> AppState {
    let app_state = AppState {
        config: Arc::new(Mutex::new(initial_config.clone())),
        hardware_hotkey: Arc::new(Mutex::new(hotkey::parse_hardware_hotkey(
            &initial_config.hotkey,
        ))),
        ..Default::default()
    };

    {
        let mut cached_device = app_state.cached_device.lock().unwrap();
        let device = match audio::lookup_device(initial_config.audio_device.clone()) {
            Ok(device) => Some(device),
            Err(error) => {
                crate::log_warn!(
                    "Initial audio device pre-warm failed (requested_device='{}'): {}",
                    initial_config
                        .audio_device
                        .clone()
                        .unwrap_or_else(|| "default".to_string()),
                    error
                );
                None
            }
        };
        *cached_device = device.clone();

        if let Some(device) = device {
            match audio::PersistentAudioEngine::new(&device, initial_config.input_sensitivity) {
                Ok(engine) => {
                    let mut engine_guard = app_state.audio_engine.lock().unwrap();
                    *engine_guard = Some(engine);
                    crate::log_info!("Persistent audio engine initialized");
                }
                Err(error) => {
                    crate::log_warn!(
                        "Initial persistent audio engine initialization failed (requested_device='{}', sensitivity={:.2}): {}",
                        initial_config
                            .audio_device
                            .clone()
                            .unwrap_or_else(|| "default".to_string()),
                        initial_config.input_sensitivity,
                        error
                    );
                }
            }
        }
        crate::log_info!("Initial pre-warm of audio device cache complete");
    }

    app_state
}

/// Loads the configured local Whisper model into memory (and GPU memory, if enabled)
/// in the background, so the first recording doesn't pay the load cost. Blocks
/// `start_recording` via `whisper_loading` while in flight, and records any failure
/// (e.g. insufficient VRAM) in `whisper_preload_error` for the UI to surface.
pub fn spawn_whisper_preload(app_handle: tauri::AppHandle) {
    let state = app_handle.state::<AppState>();
    let (transcription_mode, model_size, use_gpu) = {
        let config = state.config.lock().unwrap();
        (
            config.transcription_mode.clone(),
            config.local_model_size.clone(),
            config.enable_gpu,
        )
    };

    if transcription_mode != TranscriptionMode::Local {
        return;
    }

    // Don't attempt to load/warm up a model that hasn't been downloaded yet - that's
    // not a real failure (the user may have just picked it from the dropdown and not
    // clicked Download), so surfacing a "model not found" error here is just noise.
    // `download_model` calls this same function again once the download finishes, which
    // is when the load/warm-up actually happens.
    let is_downloaded = crate::model_manager::ModelManager::new()
        .map(|manager| manager.is_model_downloaded(&model_size))
        .unwrap_or(false);
    if !is_downloaded {
        crate::log_info!(
            "Skipping preload for '{}': not downloaded yet",
            model_size
        );
        return;
    }

    let voxbridge_engine = state.voxbridge_engine.clone();
    let voxbridge_resource_base = app_handle.path().resource_dir().ok();
    let whisper_loading = state.whisper_loading.clone();
    let whisper_preload_error = state.whisper_preload_error.clone();
    let whisper_load_generation = state.whisper_load_generation.clone();
    let whisper_last_gpu_error = state.whisper_last_gpu_error.clone();

    // Claim this as the latest requested load. Any earlier attempt still in flight will
    // see its generation is stale when it finishes and discard its result instead of
    // clobbering what we're about to do.
    let generation = whisper_load_generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;

    *whisper_loading.lock().unwrap() = true;
    crate::log_info!(
        "Preloading Whisper model '{}' (gpu={}) in the background... (generation {})",
        model_size,
        use_gpu,
        generation
    );

    let loading_status = if use_gpu {
        "Loading (GPU)"
    } else {
        "Loading (CPU)"
    };

    tauri::async_runtime::spawn(async move {
        crate::app::status::emit_status_update(loading_status).await;
        let _ = app_handle.emit("whisper-preload-status", serde_json::json!({ "loading": true }));

        let voxbridge_model_size = model_size.clone();
        let voxbridge_result = tokio::task::spawn_blocking(move || {
            local_whisper::preload_voxbridge_model(
                &voxbridge_engine,
                &voxbridge_model_size,
                use_gpu,
                voxbridge_resource_base.as_deref(),
            )
        })
        .await;

        let result: Result<local_whisper::LoadOutcome, String> = match voxbridge_result {
            Ok(Ok(())) => Ok(local_whisper::LoadOutcome {
                use_gpu,
                fell_back_from_gpu: None,
            }),
            Ok(Err(error)) => {
                crate::log_warn!("VoxBridge preload failed: {}", error);
                Err(error)
            }
            Err(error) => {
                let message = error.to_string();
                crate::log_warn!("VoxBridge preload task failed: {}", message);
                Err(message)
            }
        };

        let is_current =
            whisper_load_generation.load(std::sync::atomic::Ordering::SeqCst) == generation;
        if !is_current {
            crate::log_info!(
                "Whisper preload (generation {}) finished after being superseded by a newer config change; discarding its status effects",
                generation
            );
            // The model itself (if it loaded successfully) is still cached under its own
            // model_size/use_gpu key and can still be reused later — only the loading
            // flag/error/status side effects below are skipped, since a newer attempt
            // already owns those.
            return;
        }

        *whisper_loading.lock().unwrap() = false;

        match result {
            Ok(outcome) => {
                crate::log_info!(
                    "Whisper model preload complete (running on {})",
                    if outcome.use_gpu { "GPU" } else { "CPU" }
                );
                let fell_back_to_cpu = outcome.fell_back_from_gpu.is_some();
                if let Some(gpu_error) = outcome.fell_back_from_gpu {
                    let message = format!(
                        "GPU acceleration failed on this hardware, using CPU instead ({}). \
                         You can generate a hardware report in Settings to help get this fixed.",
                        gpu_error
                    );
                    crate::log_warn!("{}", message);
                    *whisper_last_gpu_error.lock().unwrap() = Some(gpu_error);
                    *whisper_preload_error.lock().unwrap() = Some(message);
                } else {
                    *whisper_preload_error.lock().unwrap() = None;
                }
                let _ = app_handle.emit(
                    "whisper-preload-status",
                    serde_json::json!({
                        "loading": false,
                        "error": null,
                        "fellBackToCpu": fell_back_to_cpu
                    }),
                );
            }
            Err(error) => {
                let message = error.to_string();
                crate::log_warn!("Whisper model preload failed: {}", message);
                *whisper_preload_error.lock().unwrap() = Some(message.clone());
                let _ = app_handle.emit(
                    "whisper-preload-status",
                    serde_json::json!({ "loading": false, "error": message }),
                );
            }
        }

        // Warm the two large local engines sequentially. Starting both together makes
        // first launch appear frozen while they compete for disk bandwidth and graphics
        // memory, especially with the larger transcription models.
        crate::compose::spawn_compose_preload(app_handle.clone());
        let compose_is_loading = {
            let state = app_handle.state::<AppState>();
            let loading = *state.compose_loading.lock().unwrap();
            loading
        };
        if !compose_is_loading {
            crate::app::status::emit_status_update("Ready").await;
        }
    });
}

pub fn run_setup(
    app: &mut tauri::App<tauri::Wry>,
    initial_config: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    crate::app::status::initialize(app.handle().clone());

    #[cfg(target_os = "linux")]
    {
        check_wayland_display();
        let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
        let x11_display = std::env::var("DISPLAY").ok();
        let session_type = std::env::var("XDG_SESSION_TYPE").ok();
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
        let prg_name = gtk::glib::prgname();
        let detected = crate::platform::linux::detection::detect_display_server();
        crate::log_info!(
            "Launch context: detected={:?}, XDG_SESSION_TYPE={:?}, WAYLAND_DISPLAY={:?}, DISPLAY={:?}, XDG_CURRENT_DESKTOP={:?}, prgname={:?}",
            detected,
            session_type,
            wayland_display,
            x11_display,
            desktop,
            prg_name
        );
        crate::log_info!("App version: {}", env!("CARGO_PKG_VERSION"));
        if let Some(distro_name) = read_linux_distribution_name() {
            crate::log_info!("Linux distro: {}", distro_name);
        }

        if is_wayland_session() {
            let state = app.state::<AppState>();
            let host_app_registration = tauri::async_runtime::block_on(async {
                let app_id = AppID::try_from("com.voxbridge.compose")
                    .map_err(|error| format!("Invalid host app id: {error}"))?;
                register_host_app(app_id)
                    .await
                    .map_err(|error| format!("Failed to register host app with portal: {error}"))
            });

            match host_app_registration {
                Ok(()) => {
                    let mut registration_error =
                        state.wayland_host_app_registration_error.lock().unwrap();
                    *registration_error = None;
                    crate::log_info!("Registered host app ID with portal registry");
                }
                Err(error) => {
                    let mut registration_error =
                        state.wayland_host_app_registration_error.lock().unwrap();
                    *registration_error = Some(error.clone());
                    crate::log_warn!("Host app registration failed: {}", error);
                }
            }

            let tray_watcher_available =
                tauri::async_runtime::block_on(is_status_notifier_watcher_available());
            crate::log_info!(
                "StatusNotifier watcher available: {}",
                tray_watcher_available
            );
        }
    }

    let _ = audio::get_input_devices();

    let menu = create_tray_menu(app.handle())?;
    let _tray = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .icon(app.default_window_icon().unwrap().clone())
        .on_menu_event(|app_handle, event| match event.id.as_ref() {
            "quit" => {
                std::process::exit(0);
            }
            "open" => {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                ..
            } = event
            {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    if let Some(window) = app.get_webview_window("main") {
        let window_clone = window.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window_clone.hide();
            }
        });
    }

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
    }

    let hotkey_string = initial_config.hotkey.clone();
    let app_handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = re_register_hotkey(&app_handle, &hotkey_string).await {
            crate::log_warn!("Initial hotkey registration failed: {}", error);
            let state = app_handle.state::<AppState>();
            let mut hotkey_error = state.hotkey_error.lock().unwrap();
            *hotkey_error = Some(error);
        }
    });

    spawn_whisper_preload(app.handle().clone());

    #[cfg(target_os = "linux")]
    {
        if is_wayland_session() {
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = crate::platform::linux::wayland::input::establish_input_session(
                    &app_handle,
                    false,
                )
                .await
                {
                    crate::log_warn!("Wayland input session restore failed: {}", error);
                }
            });
        }
    }

    Ok(())
}
