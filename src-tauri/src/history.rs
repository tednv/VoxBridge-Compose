use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub id: u64,
    pub text: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct History {
    pub items: Vec<HistoryItem>,
}

fn get_history_file_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut path = dirs::config_dir().ok_or("Could not find config directory")?;
    path.push("foss-voquill");

    // Create directory if it doesn't exist
    if !path.exists() {
        fs::create_dir_all(&path)?;
    }

    path.push("history.json");
    Ok(path)
}

pub fn load_history() -> Result<History, Box<dyn std::error::Error>> {
    let path = get_history_file_path()?;

    if !path.exists() {
        return Ok(History::default());
    }

    let content = fs::read_to_string(path)?;
    let history: History = serde_json::from_str(&content)?;
    Ok(history)
}

pub fn save_history(history: &History) -> Result<(), Box<dyn std::error::Error>> {
    let path = get_history_file_path()?;
    let content = serde_json::to_string_pretty(history)?;
    fs::write(path, content)?;
    Ok(())
}

fn retain_recent_items(history: &mut History, retention_days: u64) {
    if retention_days == 0 {
        return;
    }
    let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
    history.items.retain(|item| {
        chrono::DateTime::parse_from_rfc3339(&item.timestamp)
            .map(|timestamp| timestamp.with_timezone(&Utc) >= cutoff)
            .unwrap_or(true)
    });
}

pub fn prune_history(retention_days: u64) -> Result<History, Box<dyn std::error::Error>> {
    let mut history = load_history()?;
    let previous_len = history.items.len();
    retain_recent_items(&mut history, retention_days);
    if history.items.len() != previous_len {
        save_history(&history)?;
    }
    Ok(history)
}

pub fn add_history_item(
    text: &str,
    retention_days: u64,
) -> Result<HistoryItem, Box<dyn std::error::Error>> {
    let mut history = load_history()?;
    retain_recent_items(&mut history, retention_days);

    // Store as ISO 8601 UTC timestamp for easy parsing in frontend
    let timestamp = Utc::now().to_rfc3339();
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;

    let item = HistoryItem {
        id,
        text: text.to_string(),
        timestamp,
    };

    // Add to beginning of list (most recent first)
    history.items.insert(0, item.clone());

    // Keep only last 100 items
    if history.items.len() > 100 {
        history.items.truncate(100);
    }

    save_history(&history)?;
    Ok(item)
}

pub fn clear_history() -> Result<(), Box<dyn std::error::Error>> {
    let history = History::default();
    save_history(&history)?;
    Ok(())
}
