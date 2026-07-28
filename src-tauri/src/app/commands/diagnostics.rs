use crate::typing;

#[tauri::command]
pub async fn open_debug_folder() -> Result<(), String> {
    crate::log_info!("Tauri Command: open_debug_folder invoked");
    let path = dirs::config_dir()
        .ok_or("Could not find config directory")?
        .join("foss-voquill")
        .join("debug");

    crate::log_info!("Target debug path: {:?}", path);

    if !path.exists() {
        crate::log_info!("Creating debug directory...");
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        crate::log_info!("Executing: xdg-open {:?}", path);
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|error| {
                crate::log_info!("Failed to execute xdg-open: {}", error);
                error.to_string()
            })?;
    }
    #[cfg(target_os = "windows")]
    {
        crate::log_info!("Executing: explorer {:?}", path);
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|error| {
                crate::log_info!("Failed to execute explorer: {}", error);
                error.to_string()
            })?;
    }
    #[cfg(target_os = "macos")]
    {
        crate::log_info!("Executing: open {:?}", path);
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|error| {
                crate::log_info!("Failed to execute open: {}", error);
                error.to_string()
            })?;
    }

    Ok(())
}

#[tauri::command]
pub async fn clear_recording_logs() -> Result<u64, String> {
    let debug_path = dirs::config_dir()
        .ok_or("Could not find config directory")?
        .join("foss-voquill")
        .join("debug");
    if !debug_path.exists() {
        return Ok(0);
    }

    let mut removed = 0;
    for entry in std::fs::read_dir(debug_path).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let is_recording = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("recording_") && name.ends_with(".wav"));
        if is_recording {
            std::fs::remove_file(path).map_err(|error| error.to_string())?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[tauri::command]
pub async fn get_session_log_text() -> Result<String, String> {
    let log_path = crate::resolve_session_log_path()?;
    std::fs::read_to_string(&log_path).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn copy_session_log_to_clipboard() -> Result<(), String> {
    let logs = get_session_log_text().await?;
    typing::copy_to_clipboard(&logs).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn open_session_log() -> Result<(), String> {
    let log_path = crate::resolve_session_log_path()?;

    #[cfg(target_os = "linux")]
    {
        crate::log_info!("Executing: xdg-open {:?}", log_path);
        std::process::Command::new("xdg-open")
            .arg(&log_path)
            .spawn()
            .map_err(|error| {
                crate::log_info!("Failed to execute xdg-open for session log: {}", error);
                error.to_string()
            })?;
    }

    #[cfg(target_os = "windows")]
    {
        crate::log_info!("Executing: explorer {:?}", log_path);
        std::process::Command::new("explorer")
            .arg(&log_path)
            .spawn()
            .map_err(|error| {
                crate::log_info!("Failed to execute explorer for session log: {}", error);
                error.to_string()
            })?;
    }

    #[cfg(target_os = "macos")]
    {
        crate::log_info!("Executing: open {:?}", log_path);
        std::process::Command::new("open")
            .arg(&log_path)
            .spawn()
            .map_err(|error| {
                crate::log_info!("Failed to execute open for session log: {}", error);
                error.to_string()
            })?;
    }

    Ok(())
}
