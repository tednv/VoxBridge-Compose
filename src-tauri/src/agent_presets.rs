//! Reusable Compose agent prompt presets: a small set of built-in ones bundled with the
//! app (versioned in the repo under `resources/agent_presets/`, so anyone can read or
//! contribute one as a normal text file - not buried as a Rust string constant), plus
//! any the user has saved themselves. Kept as a flat list on disk rather than app state,
//! since presets are edited rarely compared to how often agents run.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPreset {
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// Built-in presets are read-only (bundled with the app, can't be edited/deleted in
    /// place) - saving over one always creates a separate custom preset instead.
    pub builtin: bool,
    /// The word-fidelity threshold (see `compose::word_fidelity`) this preset expects to
    /// run at, carried with the prompt itself rather than left as a separate per-agent
    /// setting - a "literal cleanup" preset and a "comprehensive rewrite" preset need
    /// fundamentally different thresholds to not constantly reject their own output, so
    /// the preset is the natural place to pin that down. Loading a preset into an agent
    /// applies this value; saving an agent as a new preset carries the agent's current
    /// value forward.
    #[serde(default = "default_preset_min_fidelity")]
    pub min_fidelity: f64,
}

fn default_preset_min_fidelity() -> f64 {
    0.85
}

/// One `(id, name, prompt file)` entry per bundled preset. Adding a new built-in preset
/// means adding its `.txt` file under `resources/agent_presets/` and one line here -
/// `include_str!` needs a compile-time-known path, so this can't be a runtime directory
/// scan, but that's a fair trade for "ships inside the binary, no install-time copying."
fn builtin_presets() -> Vec<AgentPreset> {
    vec![
        AgentPreset {
            id: "literal-editor".to_string(),
            name: "Literal Context-Preserving Editor".to_string(),
            prompt: include_str!("../resources/agent_presets/literal-editor.txt").to_string(),
            builtin: true,
            min_fidelity: 0.85,
        },
        AgentPreset {
            id: "punctuation-cleanup".to_string(),
            name: "Punctuation Cleanup".to_string(),
            prompt: include_str!("../resources/agent_presets/punctuation-cleanup.txt")
                .to_string(),
            builtin: true,
            min_fidelity: 0.85,
        },
        AgentPreset {
            id: "comprehensive-rewrite".to_string(),
            name: "Comprehensive Rewrite".to_string(),
            prompt: include_str!("../resources/agent_presets/comprehensive-rewrite.txt")
                .to_string(),
            builtin: true,
            // Substantive editorial rewriting is exactly what the default threshold
            // exists to reject - this preset needs real room to reword, condense, and
            // restructure without every pass getting discarded.
            min_fidelity: 0.35,
        },
    ]
}

fn custom_presets_path() -> Result<PathBuf, String> {
    let path = crate::get_app_config_root_dir()?.join("agent_presets.json");
    Ok(path)
}

fn load_custom_presets() -> Vec<AgentPreset> {
    let path = match custom_presets_path() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_custom_presets(presets: &[AgentPreset]) -> Result<(), String> {
    let path = custom_presets_path()?;
    let text = serde_json::to_string_pretty(presets).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| e.to_string())
}

/// All presets available to pick from: built-ins first, then the user's own, in save
/// order.
pub fn get_all_presets() -> Vec<AgentPreset> {
    let mut all = builtin_presets();
    all.extend(load_custom_presets());
    all
}

/// Saves `preset` as a custom preset - a new one if `preset.id` is empty or doesn't
/// already match an existing custom preset, otherwise updates that one in place.
/// Refuses to let a built-in id be overwritten; the caller should treat that as "save
/// as new" and clear the id first.
pub fn save_custom_preset(mut preset: AgentPreset) -> Result<AgentPreset, String> {
    if builtin_presets().iter().any(|p| p.id == preset.id) {
        return Err("Cannot overwrite a built-in preset - save as a new one instead.".to_string());
    }
    preset.builtin = false;
    if preset.id.trim().is_empty() {
        preset.id = format!("custom-{}", uuid_like());
    }

    let mut customs = load_custom_presets();
    if let Some(existing) = customs.iter_mut().find(|p| p.id == preset.id) {
        *existing = preset.clone();
    } else {
        customs.push(preset.clone());
    }
    save_custom_presets(&customs)?;
    Ok(preset)
}

pub fn delete_custom_preset(id: &str) -> Result<(), String> {
    let mut customs = load_custom_presets();
    customs.retain(|p| p.id != id);
    save_custom_presets(&customs)
}

/// A short, good-enough-for-this-purpose unique id - not a real UUID (avoids adding a
/// dependency for something this low-stakes), just needs to not collide in practice.
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos)
}
