use crate::{gpu_info, model_manager, AppState};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

pub const DEFAULT_EMBEDDED_MODEL_NAME: &str = "qwen3-4b-instruct-2507-q4_k_m.gguf";
pub const LEGACY_EMBEDDED_MODEL_NAME: &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";
pub const DEFAULT_OLLAMA_MODEL: &str = "qwen3:4b-instruct";
const DEFAULT_EMBEDDED_MODEL_URL: &str =
    "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf";
const EMBEDDED_MODEL_SAFETY_RESERVE_BYTES: u64 = 384 * 1024 * 1024;

struct EmbeddedModelDefinition {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    file_name: &'static str,
    url: &'static str,
    download_bytes: u64,
    recommended: bool,
}

const EMBEDDED_MODELS: &[EmbeddedModelDefinition] = &[
    EmbeddedModelDefinition {
        id: "qwen3-4b-instruct-2507-q4",
        name: "Qwen3 4B Instruct",
        description: "Best embedded editing quality; recommended for graphics cards with 6 GB or more.",
        file_name: DEFAULT_EMBEDDED_MODEL_NAME,
        url: DEFAULT_EMBEDDED_MODEL_URL,
        download_bytes: 2_497_000_000,
        recommended: true,
    },
    EmbeddedModelDefinition {
        id: "qwen2.5-1.5b-instruct-q4",
        name: "Qwen2.5 1.5B Instruct",
        description: "Lighter fallback for systems with less graphics memory.",
        file_name: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        download_bytes: 986_000_000,
        recommended: false,
    },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedModelOption {
    pub id: String,
    pub name: String,
    pub description: String,
    pub file_path: String,
    pub download_bytes: u64,
    pub installed: bool,
    pub recommended: bool,
    pub fits_graphics_memory: Option<bool>,
    pub estimated_combined_vram_bytes: u64,
    pub graphics_memory_note: String,
}

pub fn uses_legacy_default_embedded_model(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(LEGACY_EMBEDDED_MODEL_NAME))
}

pub async fn ensure_default_embedded_model() -> Result<String, String> {
    download_embedded_model_definition(&EMBEDDED_MODELS[0]).await
}

async fn download_embedded_model_definition(
    definition: &EmbeddedModelDefinition,
) -> Result<String, String> {
    let directory = crate::get_app_config_root_dir()?.join("models").join("compose");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let destination = directory.join(definition.file_name);
    if destination.exists() {
        return Ok(destination.to_string_lossy().to_string());
    }

    let temporary = destination.with_extension("gguf.download");
    let mut response = reqwest::Client::new()
        .get(format!("{}?download=true", definition.url))
        .send()
        .await
        .map_err(|error| format!("Could not download the default embedded model: {}", error))?
        .error_for_status()
        .map_err(|error| format!("Default embedded model download failed: {}", error))?;
    let total_bytes = response.content_length();
    let mut downloaded_bytes = 0_u64;
    let mut last_reported_percent = u64::MAX;
    let mut file = tokio::fs::File::create(&temporary)
        .await
        .map_err(|error| error.to_string())?;
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        file.write_all(&chunk).await.map_err(|error| error.to_string())?;
        downloaded_bytes = downloaded_bytes.saturating_add(chunk.len() as u64);
        if let Some(total) = total_bytes.filter(|total| *total > 0) {
            let percent = downloaded_bytes.saturating_mul(100) / total;
            if percent != last_reported_percent {
                last_reported_percent = percent;
                crate::app::status::emit_status_to_frontend(&format!(
                    "Downloading Compose model · {}% · {:.0}/{:.0} MB",
                    percent,
                    downloaded_bytes as f64 / 1_048_576.0,
                    total as f64 / 1_048_576.0
                ))
                .await;
            }
        }
    }
    file.flush().await.map_err(|error| error.to_string())?;
    tokio::fs::rename(&temporary, &destination)
        .await
        .map_err(|error| error.to_string())?;
    Ok(destination.to_string_lossy().to_string())
}

fn embedded_model_memory_fit(
    state: &AppState,
    definition: &EmbeddedModelDefinition,
) -> (Option<bool>, u64, String) {
    let config = state.config.lock().unwrap();
    let whisper_bytes = if config.enable_gpu {
        model_manager::ModelManager::get_available_models()
            .iter()
            .find(|model| model.size == config.local_model_size)
            .map(|model| (model.file_size as f64 * VRAM_OVERHEAD_MULTIPLIER) as u64)
            .unwrap_or(0)
    } else {
        0
    };
    let compose_bytes =
        (definition.download_bytes as f64 * VRAM_OVERHEAD_MULTIPLIER) as u64;
    let required = whisper_bytes
        .saturating_add(compose_bytes)
        .saturating_add(EMBEDDED_MODEL_SAFETY_RESERVE_BYTES);
    drop(config);

    match gpu_info::get_primary_gpu_vram_info() {
        Some(gpu) => {
            let fits = required <= gpu.dedicated_vram_bytes;
            let note = if fits {
                format!(
                    "Estimated combined allocation {:.1} GB of {:.1} GB, including a {:.0} MB safety reserve.",
                    required as f64 / 1_073_741_824.0,
                    gpu.dedicated_vram_bytes as f64 / 1_073_741_824.0,
                    EMBEDDED_MODEL_SAFETY_RESERVE_BYTES as f64 / 1_048_576.0
                )
            } else {
                format!(
                    "Needs about {:.1} GB combined, but {} reports {:.1} GB dedicated graphics memory.",
                    required as f64 / 1_073_741_824.0,
                    gpu.adapter_name,
                    gpu.dedicated_vram_bytes as f64 / 1_073_741_824.0
                )
            };
            (Some(fits), required, note)
        }
        None => (
            None,
            required,
            "Graphics-memory capacity could not be verified on this platform.".to_string(),
        ),
    }
}

#[tauri::command]
pub async fn list_embedded_compose_models(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<EmbeddedModelOption>, String> {
    let directory = crate::get_app_config_root_dir()?.join("models").join("compose");
    Ok(EMBEDDED_MODELS
        .iter()
        .map(|definition| {
            let path = directory.join(definition.file_name);
            let (fits, estimated, note) = embedded_model_memory_fit(&state, definition);
            EmbeddedModelOption {
                id: definition.id.to_string(),
                name: definition.name.to_string(),
                description: definition.description.to_string(),
                file_path: path.to_string_lossy().to_string(),
                download_bytes: definition.download_bytes,
                installed: path.exists(),
                recommended: definition.recommended,
                fits_graphics_memory: fits,
                estimated_combined_vram_bytes: estimated,
                graphics_memory_note: note,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn download_embedded_compose_model(
    state: tauri::State<'_, AppState>,
    model_id: String,
) -> Result<String, String> {
    let definition = EMBEDDED_MODELS
        .iter()
        .find(|model| model.id == model_id)
        .ok_or_else(|| "Unknown embedded model selection".to_string())?;
    let (fits, _, note) = embedded_model_memory_fit(&state, definition);
    if fits == Some(false) {
        return Err(format!("Download blocked: {note}"));
    }
    crate::app::status::emit_status_to_frontend(&format!(
        "Downloading {}",
        definition.name
    ))
    .await;
    download_embedded_model_definition(definition).await
}

pub async fn ensure_ollama_model(base_url: &str, model: &str) -> Result<(), String> {
    let available = list_ollama_models(base_url.to_string()).await?;
    if available.iter().any(|entry| entry.name == model) {
        return Ok(());
    }
    let url = format!("{}/api/pull", base_url.trim_end_matches('/'));
    reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "model": model, "stream": false }))
        .timeout(std::time::Duration::from_secs(1800))
        .send()
        .await
        .map_err(|error| format!("Could not download Ollama model '{}': {}", model, error))?
        .error_for_status()
        .map_err(|error| format!("Ollama model download failed: {}", error))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct ComposePreloadStatus {
    pub loading: bool,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn get_compose_preload_status(
    state: tauri::State<'_, AppState>,
) -> Result<ComposePreloadStatus, String> {
    Ok(ComposePreloadStatus {
        loading: *state.compose_loading.lock().unwrap(),
        error: state.compose_preload_error.lock().unwrap().clone(),
    })
}

#[tauri::command]
pub async fn get_compose_text(
    state: tauri::State<'_, AppState>,
) -> Result<crate::compose::ComposeState, String> {
    Ok(crate::compose::ComposeState {
        raw_text: state.compose_raw_buffer.lock().unwrap().clone(),
        text: state.compose_buffer.lock().unwrap().clone(),
        correcting: false,
        active_agent: None,
    })
}

#[tauri::command]
pub async fn get_compose_agents(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<crate::config::ComposeAgent>, String> {
    Ok(state.config.lock().unwrap().compose_agents.clone())
}

/// Replaces the whole agent chain (add/remove/edit/reorder all happen client-side; this
/// just persists the resulting list) and saves it to disk. Agents are read fresh from
/// config on every batch, so no cache to invalidate - the very next correction pass picks
/// up the change.
#[tauri::command]
pub async fn save_compose_agents(
    state: tauri::State<'_, AppState>,
    agents: Vec<crate::config::ComposeAgent>,
) -> Result<(), String> {
    let updated = {
        let mut config = state.config.lock().unwrap();
        config.compose_agents = agents;
        config.clone()
    };
    crate::config::save_config(&updated).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_agent_presets() -> Result<Vec<crate::agent_presets::AgentPreset>, String> {
    Ok(crate::agent_presets::get_all_presets())
}

/// Saves (or updates) a custom preset. Returns the saved preset, since a brand-new one
/// gets its id assigned server-side.
#[tauri::command]
pub async fn save_agent_preset(
    preset: crate::agent_presets::AgentPreset,
) -> Result<crate::agent_presets::AgentPreset, String> {
    crate::agent_presets::save_custom_preset(preset)
}

#[tauri::command]
pub async fn delete_agent_preset(id: String) -> Result<(), String> {
    crate::agent_presets::delete_custom_preset(&id)
}

#[tauri::command]
pub async fn get_compose_history(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<crate::compose::BatchRecord>, String> {
    Ok(crate::compose::history_snapshot(&state.compose_history))
}

/// Reverts one batch to the output after its first `keep_stage_count` agent stages (0 =
/// undo every agent, back to the raw transcript) - the contribution history's per-batch
/// "revert to here" action.
#[tauri::command]
pub async fn revert_compose_batch(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    batch_id: u64,
    keep_stage_count: usize,
) -> Result<(), String> {
    crate::compose::revert_batch(
        &state.compose_history,
        &state.compose_buffer,
        &state.compose_raw_buffer,
        &app_handle,
        batch_id,
        keep_stage_count,
    )
}

/// Applies the saved fidelity setting by rebuilding the current document once with the
/// active agent chain. Contribution batches may overlap by design, so replaying every
/// retained batch separately is both expensive and incorrect for a live document.
#[tauri::command]
pub async fn recompute_compose_fidelity(
    app_handle: tauri::AppHandle,
    agent_id: String,
    threshold: f64,
) -> Result<(), String> {
    crate::log_info!(
        "Compose: rebuilding current document after fidelity change (agent_id='{}', threshold={:.2})",
        agent_id,
        threshold
    );
    crate::compose::rerun_current_document(&app_handle);
    Ok(())
}

#[tauri::command]
pub async fn clear_compose_text(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.compose_buffer.lock().unwrap().clear();
    state.compose_raw_buffer.lock().unwrap().clear();
    crate::compose::reset_pending(&state.compose_pending);
    // The batch history is a set of offsets into the buffer just cleared - once the
    // buffer resets to empty, those offsets are stale, so any later revert attempt
    // would misbehave if the history stuck around.
    crate::compose::clear_history(&state.compose_history);
    Ok(())
}

fn default_dictations_dir() -> Result<std::path::PathBuf, String> {
    Ok(dirs::document_dir()
        .ok_or_else(|| "Could not resolve your Documents folder".to_string())?
        .join("VoxBridge Compose Offloads"))
}

fn dictations_dir(state: &AppState) -> Result<std::path::PathBuf, String> {
    if let Some(override_path) = state.compose_dump_dir_override.lock().unwrap().clone() {
        return Ok(std::path::PathBuf::from(override_path));
    }
    let configured_default = state.config.lock().unwrap().default_offload_location.clone();
    let remembered = {
        let config = state.config.lock().unwrap();
        config.remember_offload_location
            .then(|| config.last_offload_location.clone())
            .filter(|path| !path.trim().is_empty())
    };
    if let Some(path) = remembered {
        return Ok(std::path::PathBuf::from(path));
    }
    if !configured_default.trim().is_empty() {
        return Ok(std::path::PathBuf::from(configured_default));
    }
    default_dictations_dir()
}

/// The folder Offload writes into, for display in the UI before anything has been
/// saved (e.g. in a tooltip) - doesn't create it, just resolves the path.
#[tauri::command]
pub async fn get_dictations_dir(state: tauri::State<'_, AppState>) -> Result<String, String> {
    Ok(dictations_dir(&state)?.to_string_lossy().to_string())
}

/// The built-in Documents/VoxBridge Compose Offloads location, regardless of overrides.
/// for Settings to show what "Reset to default" would actually reset to.
#[tauri::command]
pub async fn get_default_dictations_dir() -> Result<String, String> {
    Ok(default_dictations_dir()?.to_string_lossy().to_string())
}

/// Whether a session-only override is currently active, for Settings to show (and offer
/// to reset) - distinct from `get_dictations_dir`, which always returns the effective
/// path (override or default) rather than whether one is set.
#[tauri::command]
pub async fn get_dump_dir_override(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    Ok(state.compose_dump_dir_override.lock().unwrap().clone())
}

/// Selects a different offload folder for this app session without changing the saved default.
#[tauri::command]
pub async fn set_dump_dir_override(
    state: tauri::State<'_, AppState>,
    path: String,
    remember: Option<bool>,
) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Path cannot be empty.".to_string());
    }
    std::fs::create_dir_all(trimmed).map_err(|e| format!("Could not use that folder: {}", e))?;
    *state.compose_dump_dir_override.lock().unwrap() = Some(trimmed.to_string());
    if let Some(remember) = remember {
        let updated = {
            let mut config = state.config.lock().unwrap();
            config.remember_offload_location = remember;
            config.last_offload_location = if remember { trimmed.to_string() } else { String::new() };
            config.clone()
        };
        crate::config::save_config(&updated).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn clear_dump_dir_override(state: tauri::State<'_, AppState>) -> Result<(), String> {
    *state.compose_dump_dir_override.lock().unwrap() = None;
    Ok(())
}

/// Opens the effective offload folder, creating it if it has not been used yet.
#[tauri::command]
pub async fn open_dictations_folder(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let path = dictations_dir(&state)?;
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeDumpResult {
    pub file_name: String,
    pub file_path: String,
}

/// Writes the current composed text to its own file in the effective offload location.
/// (named from a short slug of its own opening words, so it's recognizable in a file
/// listing without opening it) and clears the Compose buffers/history for a fresh start -
/// the "I'm done with this one, save it and move on" action.
///
/// `text` is passed in by the caller rather than read from `compose_buffer` because the
/// frontend's fidelity slider preview is a client-side what-if that never gets written
/// back to that buffer - dumping the buffer directly could silently save a different
/// (stale) version than whatever's actually showing in the Polished pane.
#[tauri::command]
pub async fn dump_compose_text(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    text: String,
) -> Result<ComposeDumpResult, String> {
    if text.trim().is_empty() {
        return Err("Nothing to save yet.".to_string());
    }

    let dictations_dir = dictations_dir(&state)?;
    std::fs::create_dir_all(&dictations_dir).map_err(|e| e.to_string())?;

    let title_text = text.clone();
    let backend_cache = state.compose_backend.clone();
    let config = state.config.lock().unwrap().clone();
    let title_app_handle = app_handle.clone();
    let slug = tokio::task::spawn_blocking(move || {
        crate::compose::semantic_filename_slug(
            &title_text,
            &backend_cache,
            &config,
            &title_app_handle,
        )
    })
    .await
    .unwrap_or_else(|_| crate::compose::filename_slug(&text));
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let file_name = format!("{}_{}.txt", slug, timestamp);
    let file_path = dictations_dir.join(&file_name);

    let include_recognized = state.config.lock().unwrap().include_recognized_in_offload;
    let file_contents = if include_recognized {
        let recognized = state.compose_raw_buffer.lock().unwrap().trim().to_string();
        format!(
            "Raw transcript\n==============\n{}\n\nRefined\n=======\n{}\n",
            recognized,
            text.trim()
        )
    } else {
        text.clone()
    };
    std::fs::write(&file_path, file_contents).map_err(|e| e.to_string())?;
    let include_recording = {
        let config = state.config.lock().unwrap();
        config.enable_history
            && config.enable_recording_logs
            && config.include_recording_in_offload
    };
    if include_recording {
        if let Some(source_audio) = crate::app::recording_flow::take_last_recording_log() {
            let audio_path = dictations_dir.join(format!("{}_{}.wav", slug, timestamp));
            std::fs::copy(source_audio, audio_path).map_err(|e| e.to_string())?;
        }
    }

    state.compose_buffer.lock().unwrap().clear();
    state.compose_raw_buffer.lock().unwrap().clear();
    crate::compose::clear_history(&state.compose_history);

    Ok(ComposeDumpResult {
        file_name,
        file_path: file_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub async fn suggest_compose_filename(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    text: String,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Ok("dictation".to_string());
    }
    crate::log_info!(
        "Compose: generating semantic filename from {} words",
        text.split_whitespace().count()
    );
    let backend_cache = state.compose_backend.clone();
    let config = state.config.lock().unwrap().clone();
    let slug = tokio::task::spawn_blocking(move || {
        crate::compose::semantic_filename_slug(&text, &backend_cache, &config, &app_handle)
    })
    .await
    .map_err(|error| error.to_string())?;
    crate::log_info!("Compose: semantic filename selected '{}'", slug);
    Ok(slug)
}

/// Drops the cached Compose backend so the next utterance reloads it from current
/// config - called when the user changes backend/model/GPU settings while the app is
/// running, rather than requiring a restart to pick up the change.
#[tauri::command]
pub async fn invalidate_compose_backend(state: tauri::State<'_, AppState>) -> Result<(), String> {
    crate::compose::invalidate_backend(
        &state.compose_backend,
        &state.compose_backend_generation,
    );
    Ok(())
}

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModelEntry>,
}

#[derive(Deserialize)]
struct OllamaModelEntry {
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    details: OllamaModelDetails,
}

#[derive(Deserialize, Default)]
struct OllamaModelDetails {
    #[serde(default)]
    parameter_size: String,
    #[serde(default)]
    quantization_level: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelInfo {
    pub name: String,
    pub size_bytes: u64,
    pub parameter_size: String,
    pub quantization_level: String,
}

const VRAM_OVERHEAD_MULTIPLIER: f64 = 1.3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CombinedVramCheck {
    pub gpu_detected: bool,
    pub adapter_name: Option<String>,
    pub dedicated_vram_bytes: Option<u64>,
    pub gpu_current_usage_bytes: Option<u64>,
    pub gpu_available_vram_bytes: Option<u64>,
    pub system_memory_total_bytes: Option<u64>,
    pub system_memory_available_bytes: Option<u64>,
    /// Estimated whisper VRAM usage, only if Local+GPU transcription is selected.
    pub whisper_estimate_bytes: u64,
    /// Embedded model estimate or live selected-model VRAM reported by local Ollama.
    pub compose_estimate_bytes: u64,
    pub compose_memory_source: String,
    pub total_usage_percent: Option<f64>,
}

#[derive(Deserialize)]
struct OllamaRunningModels {
    #[serde(default)]
    models: Vec<OllamaRunningModel>,
}

#[derive(Deserialize)]
struct OllamaRunningModel {
    #[serde(default, alias = "model")]
    name: String,
    #[serde(default)]
    size_vram: u64,
}

fn is_local_ollama_url(url: &str) -> bool {
    let normalized = url.to_ascii_lowercase();
    normalized.contains("://localhost")
        || normalized.contains("://127.0.0.1")
        || normalized.contains("://[::1]")
}

async fn local_ollama_vram(url: &str, selected_model: &str) -> Option<u64> {
    if !is_local_ollama_url(url) {
        return None;
    }
    let endpoint = format!("{}/api/ps", url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(endpoint)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<OllamaRunningModels>()
        .await
        .ok()?;
    response
        .models
        .iter()
        .find(|model| model.name == selected_model)
        .map(|model| model.size_vram)
        .or_else(|| {
            let total = response.models.iter().map(|model| model.size_vram).sum();
            (total > 0).then_some(total)
        })
}

/// Breaks the GPU VRAM estimate down by which feature is actually using it - whisper
/// transcription and/or the embedded Compose model - so the Settings UI can show where
/// the usage actually comes from instead of one opaque number, since both can be active
/// on GPU at once and draw from the same budget.
#[tauri::command]
pub async fn check_combined_vram(
    state: tauri::State<'_, AppState>,
) -> Result<CombinedVramCheck, String> {
    let (
        enable_gpu,
        local_model_size,
        compose_backend,
        compose_use_gpu,
        compose_model_path,
        compose_ollama_url,
        compose_ollama_model,
    ) = {
        let config = state.config.lock().unwrap();
        (
            config.enable_gpu,
            config.local_model_size.clone(),
            config.compose_backend.clone(),
            config.compose_use_gpu,
            config.compose_model_path.clone(),
            config.compose_ollama_url.clone(),
            config.compose_ollama_model.clone(),
        )
    };

    let whisper_estimate_bytes = if enable_gpu {
        model_manager::ModelManager::get_available_models()
            .iter()
            .find(|m| m.size == local_model_size)
            .map(|m| (m.file_size as f64 * VRAM_OVERHEAD_MULTIPLIER) as u64)
            .unwrap_or(0)
    } else {
        0
    };

    let (compose_estimate_bytes, compose_memory_source) =
        if compose_backend == "embedded" && compose_use_gpu {
            (
                std::fs::metadata(&compose_model_path)
            .map(|m| (m.len() as f64 * VRAM_OVERHEAD_MULTIPLIER) as u64)
                    .unwrap_or(0),
                "Embedded model estimate".to_string(),
            )
        } else if compose_backend == "ollama_remote" {
            match local_ollama_vram(&compose_ollama_url, &compose_ollama_model).await {
                Some(bytes) => (bytes, "Local Ollama reported usage".to_string()),
                None => (0, "Remote Ollama not measured".to_string()),
            }
        } else {
            (0, "Processor".to_string())
        };

    let system_memory = crate::hardware_report::system_memory_bytes();

    match gpu_info::get_primary_gpu_vram_info() {
        Some(info) => {
            let total = whisper_estimate_bytes + compose_estimate_bytes;
            let total_usage_percent = if info.dedicated_vram_bytes > 0 {
                Some((total as f64 / info.dedicated_vram_bytes as f64) * 100.0)
            } else {
                None
            };
            Ok(CombinedVramCheck {
                gpu_detected: true,
                adapter_name: Some(info.adapter_name),
                dedicated_vram_bytes: Some(info.dedicated_vram_bytes),
                gpu_current_usage_bytes: Some(info.current_usage_bytes),
                gpu_available_vram_bytes: Some(info.available_vram_bytes),
                system_memory_total_bytes: system_memory.map(|value| value.0),
                system_memory_available_bytes: system_memory.map(|value| value.1),
                whisper_estimate_bytes,
                compose_estimate_bytes,
                compose_memory_source,
                total_usage_percent,
            })
        }
        None => Ok(CombinedVramCheck {
            gpu_detected: false,
            adapter_name: None,
            dedicated_vram_bytes: None,
            gpu_current_usage_bytes: None,
            gpu_available_vram_bytes: None,
            system_memory_total_bytes: system_memory.map(|value| value.0),
            system_memory_available_bytes: system_memory.map(|value| value.1),
            whisper_estimate_bytes,
            compose_estimate_bytes,
            compose_memory_source,
            total_usage_percent: None,
        }),
    }
}

/// Queries a real Ollama instance's own model list - not a static guess, so Settings can
/// show exactly what's actually available on that instance right now, including the
/// per-model stats (parameter count, quantization, size) Ollama's `/api/tags` already
/// reports, rather than just a bare name.
#[tauri::command]
pub async fn list_ollama_models(base_url: String) -> Result<Vec<OllamaModelInfo>, String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("Could not reach Ollama at {}: {}", base_url, e))?;

    let parsed: OllamaTagsResponse = response
        .json()
        .await
        .map_err(|e| format!("Unexpected response from {}: {}", url, e))?;

    Ok(parsed
        .models
        .into_iter()
        .map(|m| OllamaModelInfo {
            name: m.name,
            size_bytes: m.size,
            parameter_size: m.details.parameter_size,
            quantization_level: m.details.quantization_level,
        })
        .collect())
}
