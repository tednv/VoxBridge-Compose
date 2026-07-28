use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const INPUT_SENSITIVITY_MIN: f32 = 0.1;
pub const INPUT_SENSITIVITY_MAX: f32 = 2.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OutputMethod {
    Typewriter,
    Clipboard,
    /// "VoxBridge Compose": raw transcribed text lands immediately in an in-app editor
    /// buffer (not typed into external apps), then an async pass through a small LLM
    /// (embedded via VoxBridge, or proxied to a remote Ollama instance) cleans up
    /// spacing/punctuation/context without ever blocking the live transcription stream.
    Compose,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TranscriptionMode {
    API,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_api_key")]
    pub openai_api_key: String,
    #[serde(default = "default_api_url")]
    pub api_url: String,
    #[serde(default = "default_api_model")]
    pub api_model: String,
    #[serde(default = "default_transcription_mode")]
    pub transcription_mode: TranscriptionMode,
    #[serde(default = "default_local_model_size")]
    pub local_model_size: String,
    #[serde(default = "default_local_engine")]
    pub local_engine: String,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default = "default_typing_speed")]
    pub typing_speed_interval: f64,
    #[serde(default = "default_key_press_duration")]
    pub key_press_duration_ms: u64,
    #[serde(default = "default_pixels_from_bottom")]
    pub pixels_from_bottom: i32,
    #[serde(default = "default_audio_device")]
    pub audio_device: Option<String>,
    #[serde(default = "default_debug_mode")]
    pub debug_mode: bool,
    #[serde(default = "default_enable_recording_logs")]
    pub enable_recording_logs: bool,
    #[serde(default = "default_recording_log_retention_days")]
    pub recording_log_retention_days: u64,
    #[serde(default = "default_enable_history")]
    pub enable_history: bool,
    /// 0 retains history indefinitely; positive values expire entries after that many days.
    #[serde(default)]
    pub history_retention_days: u64,
    #[serde(default)]
    pub offload_locations: Vec<String>,
    #[serde(default)]
    pub default_offload_location: String,
    #[serde(default)]
    pub remember_offload_location: bool,
    #[serde(default)]
    pub last_offload_location: String,
    #[serde(default = "default_include_recognized_in_offload")]
    pub include_recognized_in_offload: bool,
    #[serde(default)]
    pub include_recording_in_offload: bool,
    #[serde(default = "default_input_sensitivity")]
    pub input_sensitivity: f32,
    #[serde(default = "default_output_method")]
    pub output_method: OutputMethod,
    #[serde(default = "default_copy_on_typewriter")]
    pub copy_on_typewriter: bool,
    #[serde(default)]
    pub shortcuts_token: Option<String>,
    #[serde(default)]
    pub input_token: Option<String>,
    #[serde(default = "default_enable_gpu")]
    pub enable_gpu: bool,
    #[serde(default = "default_post_roll_ms")]
    pub post_roll_ms: u64,
    /// Physical-pixel position the user dragged the overlay pill to, if any. When set,
    /// this overrides the computed `pixels_from_bottom` auto-positioning entirely.
    #[serde(default)]
    pub overlay_position: Option<(i32, i32)>,
    /// Whether to silently check for updates on startup. Off by default for this fork -
    /// upstream release cadence/stability doesn't necessarily match what this fork wants
    /// to track, so checking (and surfacing an "update available" badge) is opt-in. The
    /// manual "Check for Updates" button in Settings always works regardless of this.
    #[serde(default)]
    pub check_for_updates_on_startup: bool,
    /// "sticky" (default): press the hotkey to start recording, press it again to stop -
    /// no need to hold it down. "hold": classic push-to-talk, release to stop.
    #[serde(default = "default_hotkey_mode")]
    pub hotkey_mode: String,
    /// Continuous mode only: how long a pause in speech has to last before that utterance
    /// is considered finished and gets transcribed. Lower = more responsive but more
    /// likely to cut off a mid-sentence breath/reflection pause; higher = more patient
    /// but slower to actually send text.
    #[serde(default = "default_continuous_silence_ms")]
    pub continuous_silence_ms: u64,
    /// A short, fixed list of proper nouns/jargon (names, project names, technical
    /// terms) fed to Whisper as its initial_prompt on every transcription, to bias
    /// recognition/spelling toward them. Deliberately just a word list, not the running
    /// session's own transcript - that caused a real, documented failure (see
    /// recording_flow.rs) where a long/growing prompt sent whisper.cpp's
    /// temperature-fallback retry logic into multi-second stalls and hallucinated
    /// output. A short static list doesn't hit that; comma-separated, free text.
    #[serde(default)]
    pub custom_vocabulary: String,
    /// "embedded" (default): run the small LLM in-process via VoxBridge. "ollama_remote":
    /// proxy to an Ollama instance instead (local or over the network).
    #[serde(default = "default_compose_backend")]
    pub compose_backend: String,
    /// Embedded backend only: whether to offload to GPU (Vulkan) instead of running on CPU.
    #[serde(default = "default_compose_use_gpu")]
    pub compose_use_gpu: bool,
    /// Embedded backend only: path to a local GGUF model file.
    #[serde(default)]
    pub compose_model_path: String,
    /// Remote backend only: base URL of the Ollama instance, e.g. "http://localhost:11434"
    /// or "http://192.168.1.50:11434" for one elsewhere on the network.
    #[serde(default = "default_compose_ollama_url")]
    pub compose_ollama_url: String,
    /// Remote backend only: which model name (from that instance's own model list) to use.
    #[serde(default)]
    pub compose_ollama_model: String,
    /// How many utterances to batch together before running a correction pass, giving the
    /// model that many sentences of real flow/context instead of correcting one in
    /// isolation. 0 means "Everything": never fire on count, only once the user actually
    /// pauses, so a batch grows to cover everything said since the last correction.
    #[serde(default = "default_compose_context_sentences")]
    pub compose_context_sentences: u32,
    /// How long to wait before actually running a correction pass once a batch is ready -
    /// "low"/"medium"/"high", mapped to a debounce duration in `compose.rs`. Separate from
    /// `compose_context_sentences` (how much to batch): this is purely about latency/how
    /// eager vs. patient the pipeline is.
    #[serde(default = "default_compose_edit_latency")]
    pub compose_edit_latency: String,
    /// If true, ignore `compose_context_sentences`'s count threshold entirely and only
    /// ever run a correction pass after a real pause in speech - useful for keeping
    /// refactoring fully out of the way while actively dictating in a burst.
    #[serde(default)]
    pub compose_pause_only: bool,
    /// The chain of LLM passes a batch runs through, in priority order (lower runs
    /// first), each seeing the previous agent's output. Lets the user compose their own
    /// pipeline (e.g. a punctuation-cleanup pass, then a separate tone/style pass) instead
    /// of being limited to one fixed prompt.
    #[serde(default = "default_compose_agents")]
    pub compose_agents: Vec<ComposeAgent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComposeAgent {
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// Lower runs first.
    pub priority: i32,
    /// "low"/"medium"/"high" - how generous a token budget this agent's pass gets
    /// (a fast, narrow-scope agent vs. a slower, more thorough one), mapped to a
    /// per-word token multiplier in `compose.rs`.
    pub speed: String,
    pub enabled: bool,
    /// Which preset (built-in or custom) this agent's prompt currently traces back to,
    /// if any - lets the UI offer "Reset" to restore that preset's original text after
    /// local edits. `None` once the prompt has always been hand-written, not loaded
    /// from a preset.
    #[serde(default)]
    pub preset_id: Option<String>,
    /// The minimum word-fidelity score (0.0-1.0) this agent's output must clear to be
    /// accepted - see `compose::word_fidelity`. Defaults to 0.85 (preserving existing
    /// behavior for a literal cleanup pass); an agent meant to do substantive editorial
    /// rewriting needs this dialed down, since real rewriting is exactly what the
    /// default threshold exists to catch and reject.
    #[serde(default = "default_agent_min_fidelity")]
    pub min_fidelity: f64,
    /// Whether this agent may use recent saved History entries as read-only context.
    /// Private mode always suppresses this context.
    #[serde(default)]
    pub include_history: bool,
    /// Maximum number of recent saved History entries supplied to this agent.
    #[serde(default = "default_agent_history_items")]
    pub history_items: usize,
}

fn default_agent_min_fidelity() -> f64 {
    0.85
}

fn default_agent_history_items() -> usize {
    3
}

fn default_include_recognized_in_offload() -> bool {
    true
}

impl Config {
    pub fn normalize_input_sensitivity(&mut self) {
        self.input_sensitivity = self
            .input_sensitivity
            .clamp(INPUT_SENSITIVITY_MIN, INPUT_SENSITIVITY_MAX);
    }
}

fn default_api_key() -> String {
    "your_api_key_here".to_string()
}
fn default_api_url() -> String {
    "https://api.openai.com/v1/audio/transcriptions".to_string()
}
fn default_api_model() -> String {
    "gpt-4o-transcribe".to_string()
}
fn default_transcription_mode() -> TranscriptionMode {
    TranscriptionMode::Local
}
fn default_local_model_size() -> String {
    "base".to_string()
}
fn default_local_engine() -> String {
    "VoxBridge".to_string()
}
fn default_hotkey() -> String {
    "ctrl+shift+space".to_string()
}
fn default_hotkey_mode() -> String {
    "sticky".to_string()
}
fn default_continuous_silence_ms() -> u64 {
    900
}
fn default_compose_backend() -> String {
    "embedded".to_string()
}
fn default_compose_ollama_url() -> String {
    "http://localhost:11434".to_string()
}
fn default_compose_context_sentences() -> u32 {
    // 0 = "Everything": wait for a real pause rather than a sentence count. Tonight's
    // actual testing settled on this as the setting that works well by default -
    // narrative dictation especially benefits from more context per correction pass,
    // and the read-only prior-context window means even short back-and-forth
    // utterances still get real context without needing a bigger batch.
    0
}
fn default_compose_edit_latency() -> String {
    "medium".to_string()
}
/// The stock editing pass every fresh install starts with, loaded from the same
/// bundled preset file exposed in Settings.
const DEFAULT_CLEANUP_AGENT_PROMPT: &str =
    include_str!("../resources/agent_presets/comprehensive-rewrite.txt");
fn default_compose_agents() -> Vec<ComposeAgent> {
    vec![ComposeAgent {
        id: "comprehensive-rewrite".to_string(),
        name: "Comprehensive Rewrite".to_string(),
        prompt: DEFAULT_CLEANUP_AGENT_PROMPT.to_string(),
        priority: 0,
        speed: "medium".to_string(),
        enabled: true,
        preset_id: Some("comprehensive-rewrite".to_string()),
        min_fidelity: 0.35,
        include_history: false,
        history_items: default_agent_history_items(),
    }]
}
fn default_typing_speed() -> f64 {
    0.001
}
fn default_key_press_duration() -> u64 {
    2
}
fn default_pixels_from_bottom() -> i32 {
    150
}
fn default_audio_device() -> Option<String> {
    Some("default".to_string())
}
fn default_debug_mode() -> bool {
    false
}
fn default_enable_recording_logs() -> bool {
    false
}
fn default_recording_log_retention_days() -> u64 {
    7
}
fn default_enable_history() -> bool {
    true
}
fn default_input_sensitivity() -> f32 {
    1.0
}
fn default_output_method() -> OutputMethod {
    OutputMethod::Compose
}
fn default_copy_on_typewriter() -> bool {
    false
}
fn default_enable_gpu() -> bool {
    // Safe by default: `ensure_model_loaded_with_fallback` always tries GPU first and
    // transparently falls back to CPU (with a friendly status message) if the GPU attempt
    // fails or times out - so defaulting to "try GPU" doesn't risk breaking anyone without
    // one, it's strictly faster when a GPU is available and a no-op otherwise.
    true
}
fn default_post_roll_ms() -> u64 {
    400
}
fn default_compose_use_gpu() -> bool {
    // Unlike `default_enable_gpu`, this doesn't just blindly default true and lean on
    // the fallback (`ensure_backend_loaded` does fall back to CPU if a GPU load fails,
    // same pattern) - LLM inference is meaningfully more VRAM-hungry than whisper, and a
    // dGPU with too little VRAM tends to thrash or OOM rather than cleanly fail over.
    // Only default it on if there's a dedicated GPU with a reasonable amount of VRAM to
    // work with.
    const MIN_VRAM_BYTES: u64 = 4 * 1024 * 1024 * 1024; // 4 GB
    crate::gpu_info::get_primary_gpu_vram_info()
        .map(|info| info.dedicated_vram_bytes >= MIN_VRAM_BYTES)
        .unwrap_or(false)
}

fn normalize_legacy_portal_hotkey(hotkey: &str) -> Option<String> {
    let trimmed = hotkey.trim();
    let lower = trimmed.to_lowercase();

    if !lower.starts_with("press <") {
        return None;
    }

    let mut modifiers: Vec<&str> = Vec::new();
    if lower.contains("<control>") {
        modifiers.push("ctrl");
    }
    if lower.contains("<shift>") {
        modifiers.push("shift");
    }
    if lower.contains("<alt>") {
        modifiers.push("alt");
    }
    if lower.contains("<super>") || lower.contains("<logo>") {
        modifiers.push("super");
    }

    let key_start_index = lower.rfind('>').map(|index| index + 1).unwrap_or(0);
    let key = lower[key_start_index..].trim();

    if key.is_empty() {
        return None;
    }

    let mut normalized = modifiers
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>();
    normalized.push(key.to_string());

    Some(normalized.join("+"))
}

impl Default for Config {
    fn default() -> Self {
        Self {
            openai_api_key: default_api_key(),
            api_url: default_api_url(),
            api_model: default_api_model(),
            transcription_mode: default_transcription_mode(),
            local_model_size: default_local_model_size(),
            local_engine: default_local_engine(),
            hotkey: default_hotkey(),
            hotkey_mode: default_hotkey_mode(),
            continuous_silence_ms: default_continuous_silence_ms(),
            custom_vocabulary: String::new(),
            compose_backend: default_compose_backend(),
            compose_use_gpu: default_compose_use_gpu(),
            compose_model_path: String::new(),
            compose_ollama_url: default_compose_ollama_url(),
            compose_ollama_model: String::new(),
            compose_context_sentences: default_compose_context_sentences(),
            compose_edit_latency: default_compose_edit_latency(),
            compose_pause_only: false,
            compose_agents: default_compose_agents(),
            typing_speed_interval: default_typing_speed(),
            key_press_duration_ms: default_key_press_duration(),
            pixels_from_bottom: default_pixels_from_bottom(),
            audio_device: default_audio_device(),
            debug_mode: default_debug_mode(),
            enable_recording_logs: default_enable_recording_logs(),
            recording_log_retention_days: default_recording_log_retention_days(),
            enable_history: default_enable_history(),
            history_retention_days: 0,
            offload_locations: Vec::new(),
            default_offload_location: String::new(),
            remember_offload_location: false,
            last_offload_location: String::new(),
            include_recognized_in_offload: default_include_recognized_in_offload(),
            include_recording_in_offload: false,
            input_sensitivity: default_input_sensitivity(),
            output_method: default_output_method(),
            copy_on_typewriter: default_copy_on_typewriter(),
            shortcuts_token: None,
            input_token: None,
            enable_gpu: default_enable_gpu(),
            post_roll_ms: default_post_roll_ms(),
            overlay_position: None,
            check_for_updates_on_startup: false,
        }
    }
}

pub fn get_config_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let config_dir = dirs::config_dir()
        .ok_or("Could not find config directory")?
        .join("foss-voquill");

    fs::create_dir_all(&config_dir)?;
    Ok(config_dir.join("config.json"))
}

pub fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let config_path = get_config_path()?;

    if config_path.exists() {
        let config_str = fs::read_to_string(&config_path)?;

        // Migrate legacy linux_portal_hotkey into hotkey, then drop the legacy field
        let mut config_value: serde_json::Value = serde_json::from_str(&config_str)?;
        if let Some(portal_hotkey) = config_value
            .get("linux_portal_hotkey")
            .and_then(|value| value.as_str())
        {
            if !portal_hotkey.trim().is_empty() {
                config_value["hotkey"] = serde_json::Value::String(portal_hotkey.to_string());
            }
        }
        if let Some(obj) = config_value.as_object_mut() {
            obj.remove("linux_portal_hotkey");

            if let Some(hotkey) = obj.get("hotkey").and_then(|value| value.as_str()) {
                if let Some(normalized_hotkey) = normalize_legacy_portal_hotkey(hotkey) {
                    obj.insert(
                        "hotkey".to_string(),
                        serde_json::Value::String(normalized_hotkey),
                    );
                }
            }
        }

        let mut config = serde_json::from_value::<Config>(config_value)?;
        config.normalize_input_sensitivity();
        for agent in &mut config.compose_agents {
            if agent.name.trim().eq_ignore_ascii_case("Agent 1")
                && matches!(
                    agent.preset_id.as_deref(),
                    None | Some("") | Some("comprehensive-rewrite")
                )
            {
                agent.name = "Comprehensive Rewrite".to_string();
                agent.prompt = DEFAULT_CLEANUP_AGENT_PROMPT.to_string();
                agent.preset_id = Some("comprehensive-rewrite".to_string());
            }
        }
        // Persist migration to disk to keep config clean
        save_config(&config)?;
        Ok(config)
    } else {
        // Create default config file
        let default_config = Config::default();
        save_config(&default_config)?;
        Ok(default_config)
    }
}

pub fn is_first_launch() -> Result<bool, Box<dyn std::error::Error>> {
    let config_path = get_config_path()?;

    // If config file doesn't exist, it's definitely first launch
    if !config_path.exists() {
        return Ok(true);
    }

    // If config exists but API key is still default, treat as first launch
    let config = load_config()?;
    Ok(config.openai_api_key == "your_api_key_here" || config.openai_api_key.is_empty())
}

pub fn save_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = get_config_path()?;
    log_info!("Attempting to save config to: {:?}", config_path);

    let mut normalized_config = config.clone();
    normalized_config.normalize_input_sensitivity();
    let config_str = serde_json::to_string_pretty(&normalized_config)?;
    log_info!(
        "Config summary: mode={:?}, engine={}, model={}, hotkey={}, audio_device={:?}, debug_mode={}, recording_logs={}, gpu={}, input_sensitivity={:.2}",
        normalized_config.transcription_mode,
        normalized_config.local_engine,
        normalized_config.local_model_size,
        normalized_config.hotkey,
        normalized_config.audio_device,
        normalized_config.debug_mode,
        normalized_config.enable_recording_logs,
        normalized_config.enable_gpu,
        normalized_config.input_sensitivity
    );

    fs::write(&config_path, config_str)?;
    log_info!("Config saved successfully to: {:?}", config_path);
    Ok(())
}
