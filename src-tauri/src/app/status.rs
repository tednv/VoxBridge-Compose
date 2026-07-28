use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager};

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static CURRENT_STATUS: OnceLock<Mutex<String>> = OnceLock::new();
static STATUS_UPDATE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize)]
struct StatusUpdatePayload {
    seq: u64,
    status: String,
}

pub fn initialize(app_handle: AppHandle) {
    let _ = APP_HANDLE.set(app_handle);
    let _ = CURRENT_STATUS.set(Mutex::new("Ready".to_string()));
}

pub fn get_current_status() -> String {
    if let Some(status_mutex) = CURRENT_STATUS.get() {
        if let Ok(status) = status_mutex.lock() {
            return status.clone();
        }
    }
    "Ready".to_string()
}

pub async fn emit_status_update(status: &str) {
    let sequence = STATUS_UPDATE_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    let mut previous_status: Option<String> = None;
    let mut changed = false;
    if let Some(status_mutex) = CURRENT_STATUS.get() {
        if let Ok(mut global_status) = status_mutex.lock() {
            previous_status = Some(global_status.clone());
            if *global_status != status {
                *global_status = status.to_string();
                changed = true;
            }
        }
    }

    if !changed {
        return;
    }

    crate::log_info!(
        "App Status Change: '{}' -> '{}'",
        previous_status.as_deref().unwrap_or("<unknown>"),
        status
    );

    if let Some(app_handle) = APP_HANDLE.get() {
        let payload = StatusUpdatePayload {
            seq: sequence,
            status: status.to_string(),
        };
        if let Some(window) = app_handle.get_webview_window("main") {
            let _ = window.emit("status-update", payload);
        }
    }
}

pub async fn emit_status_to_frontend(status: &str) {
    emit_status_update(status).await;
}
