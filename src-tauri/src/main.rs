// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Logging macro with timestamps
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        {
            let __timestamp = ::chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
            let __message = format!($($arg)*);
            println!("[{}] {}", __timestamp, __message);
            crate::append_session_log("INFO", &__timestamp, &__message);
        }
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        {
            let __timestamp = ::chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
            let __message = format!($($arg)*);
            eprintln!("[{}] WARNING: {}", __timestamp, __message);
            crate::append_session_log("WARN", &__timestamp, &__message);
        }
    };
}

#[cfg(not(target_os = "linux"))]
use serde::Serialize;
use tauri::Manager;

mod agent_presets;
mod app;
mod audio;
mod compose;
mod config;
mod gpu_info;
mod hardware_report;
mod history;
mod hotkey;
mod local_whisper;
mod model_manager;
pub mod platform;
mod transcription;
mod typing;

pub use app::commands::hotkey::set_hotkey_binding_state;
use app::commands::*;
pub use app::session_log::append_session_log;
pub(crate) use app::session_log::{
    clear_directory_contents, get_app_config_root_dir, resolve_session_log_path,
    set_persistence_enabled, truncate_session_log_with_header,
};
pub use app::state::{AppState, HotkeyBindingState};
#[cfg(target_os = "linux")]
use platform::linux::wayland::env::configure_linux_session_environment;

#[cfg(not(target_os = "linux"))]
#[derive(Clone, Debug, Serialize)]
pub struct PortalDiagnostics {
    pub available: bool,
    pub version: u32,
    pub supports_configure_shortcuts: bool,
    pub has_record_shortcut: bool,
    pub active_trigger: Option<String>,
    pub status: String,
    pub detail: Option<String>,
}

fn main() {
    // Third-party Vulkan "implicit layers" (Steam's overlay/shader-cache hooks, OBS's
    // capture hook, and NVIDIA/AMD's own hybrid-graphics switching layers) get injected
    // into every Vulkan process system-wide, including ours. On some hybrid-graphics
    // (Optimus/switchable) machines, the interaction between these layers corrupts
    // `vkEnumeratePhysicalDevices` (observed failing with VK_INCOMPLETE), which can then
    // cause whisper.cpp's Vulkan backend init to hang indefinitely — and since ggml's
    // backend registry is a one-time-per-process lazy init shared by every backend
    // (including the plain CPU one), a single bad GPU attempt can end up blocking all
    // *future* local transcription in that process too, GPU or not.
    //
    // VoxBridge Compose only uses Vulkan for local compute, never for rendering or
    // screenshots, so none of these layers do anything useful for us — disabling them
    // for our own process (not system-wide; Steam/OBS/etc. are unaffected elsewhere)
    // removes the interaction entirely. Must be set before any Vulkan-touching code runs.
    // SAFETY: called as the very first statement in main(), before any other threads exist.
    unsafe {
        std::env::set_var("VK_LOADER_LAYERS_DISABLE", "~all~");
    }

    let initial_config = config::load_config().unwrap_or_default();
    app::session_log::set_persistence_enabled(initial_config.enable_history);
    app::session_log::initialize_session_logging();

    #[cfg(target_os = "linux")]
    {
        configure_linux_session_environment();
    }

    env_logger::init();

    let _is_first_launch = config::is_first_launch().unwrap_or(false);
    let app_state = app::bootstrap::build_app_state(&initial_config);
    let args: Vec<String> = std::env::args().collect();
    let offload_argument = args
        .iter()
        .find_map(|argument| argument.strip_prefix("--offload-location="))
        .map(str::to_string)
        .or_else(|| {
            args.iter()
                .position(|argument| argument == "--offload-location")
                .and_then(|index| args.get(index + 1).cloned())
        });
    if let Some(path) = offload_argument.filter(|path| !path.trim().is_empty()) {
        *app_state.compose_dump_dir_override.lock().unwrap() = Some(path);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    // Ignore plugin hotkeys on Wayland, use Portal instead
                    if std::env::var("WAYLAND_DISPLAY").is_ok() {
                        return;
                    }
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        let app_handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let state = app_handle.state::<AppState>();
                            let is_sticky = state.config.lock().unwrap().hotkey_mode != "hold";
                            let already_recording = *state.is_recording.lock().unwrap();
                            if is_sticky && already_recording {
                                let _ = stop_recording(state).await;
                            } else {
                                let _ = start_recording(state, app_handle.clone()).await;
                            }
                        });
                    } else {
                        let app_handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let state = app_handle.state::<AppState>();
                            // In sticky mode the hotkey release doesn't stop anything -
                            // only a second press does (handled above).
                            if state.config.lock().unwrap().hotkey_mode == "hold" {
                                let _ = stop_recording(state).await;
                            }
                        });
                    }
                })
                .build(),
        )
        .manage(app_state)
        .setup(move |app| app::bootstrap::run_setup(app, &initial_config))
        .invoke_handler(tauri::generate_handler![
            start_recording,
            stop_recording,
            get_config,
            save_config,
            reset_application_to_defaults,
            test_api_key,
            get_current_status,
            get_history,
            clear_history,
            check_hotkey_status,
            manual_register_hotkey,
            configure_hotkey,
            apply_captured_hotkey,
            get_hotkey_binding_state,
            minimize_to_tray_or_taskbar,
            quit_application,
            get_audio_devices,
            start_mic_test,
            stop_mic_test,
            stop_mic_playback,
            open_debug_folder,
            clear_recording_logs,
            get_session_log_text,
            copy_session_log_to_clipboard,
            open_session_log,
            log_ui_event,
            get_available_engines,
            get_available_models,
            check_model_status,
            download_model,
            get_linux_setup_status,
            request_audio_permission,
            request_input_permission,
            set_configuring_hotkey,
            get_wayland_portal_version,
            get_portal_diagnostics,
            get_system_shortcut_context,
            get_overlay_positioning_capabilities,
            check_for_updates,
            check_gpu_vram,
            get_whisper_preload_status,
            get_compose_preload_status,
            save_overlay_position,
            get_hardware_report,
            get_session_stats,
            get_compose_text,
            clear_compose_text,
            dump_compose_text,
            get_dictations_dir,
            get_default_dictations_dir,
            get_dump_dir_override,
            open_dictations_folder,
            set_dump_dir_override,
            clear_dump_dir_override,
            invalidate_compose_backend,
            list_ollama_models,
            check_combined_vram,
            get_compose_agents,
            save_compose_agents,
            get_agent_presets,
            save_agent_preset,
            delete_agent_preset,
            get_compose_history,
            revert_compose_batch,
            recompute_compose_fidelity,
            test_engine_loader
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
