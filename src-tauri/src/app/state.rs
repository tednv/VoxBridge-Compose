use crate::audio;
use crate::config::Config;
use crate::hotkey::HardwareHotkey;
use crate::local_whisper::{
    FasterWhisperEngineCache, VoxBridgeEngineCache, WhisperEngineCache,
};
use crate::platform;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub is_recording: Arc<Mutex<bool>>,
    pub is_mic_test_active: Arc<Mutex<bool>>,
    pub is_configuring_hotkey: Arc<Mutex<bool>>,
    pub hotkey_error: Arc<Mutex<Option<String>>>,
    pub hotkey_binding_state: Arc<Mutex<HotkeyBindingState>>,
    pub setup_status: Arc<Mutex<Option<String>>>,
    pub hardware_hotkey: Arc<Mutex<HardwareHotkey>>,
    pub cached_device: Arc<Mutex<Option<cpal::Device>>>,
    pub playback_stream: Arc<Mutex<Option<cpal::Stream>>>,
    pub mic_test_samples: Arc<Mutex<Vec<f32>>>,
    pub audio_engine: Arc<Mutex<Option<audio::PersistentAudioEngine>>>,
    pub whisper_engine: WhisperEngineCache,
    pub whisper_loading: Arc<Mutex<bool>>,
    pub whisper_preload_error: Arc<Mutex<Option<String>>>,
    /// Bumped every time a new preload/reload is kicked off. A stale in-flight attempt
    /// checks this before applying its result, so rapidly toggling GPU/model settings
    /// can't have an old, still-running load clobber a newer one's state when it finally
    /// finishes (there's no way to actually kill the underlying blocking thread).
    pub whisper_load_generation: Arc<AtomicU64>,
    /// Set whenever GPU transcription/loading fails and we fall back to CPU. Feeds the
    /// "Generate hardware report" flow so the report can include the concrete error that
    /// this machine's GPU setup hit, not just "GPU didn't work".
    pub whisper_last_gpu_error: Arc<Mutex<Option<String>>>,
    /// VoxBridge's model cache (see `local_whisper.rs`) - the per-CPU-ISA-dispatch (or
    /// Vulkan GPU) engine tried before whisper-rs for both CPU and GPU transcription,
    /// with automatic fall-through to whisper-rs on any failure. `None` cached entry
    /// just means "not loaded/available yet", not an error.
    pub voxbridge_engine: VoxBridgeEngineCache,
    pub faster_whisper_engine: FasterWhisperEngineCache,
    #[cfg(target_os = "linux")]
    pub hotkey_engine_cancel: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    #[cfg(target_os = "linux")]
    pub wayland_input_sender:
        Arc<Mutex<Option<platform::linux::wayland::input::WaylandTypeSender>>>,
    #[cfg(target_os = "linux")]
    pub wayland_input_cancel: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    #[cfg(target_os = "linux")]
    pub wayland_input_ready: Arc<Mutex<bool>>,
    #[cfg(target_os = "linux")]
    pub wayland_host_app_registration_error: Arc<Mutex<Option<String>>>,
    pub display_backend: Arc<dyn platform::traits::DisplayBackend>,
    /// Running counters for the Debug panel's "Session Stats" - reset each time the app
    /// launches (not persisted), just a lightweight at-a-glance view of what's happened
    /// this session, not a historical log.
    pub session_stats: Arc<SessionStats>,
    /// "VoxBridge Compose" output mode's accumulated corrected-text buffer for this session.
    pub compose_buffer: crate::compose::ComposeBuffer,
    /// The untouched raw transcript buffer, kept alongside `compose_buffer` so the
    /// frontend can show a "what was actually said" pane next to the corrected one.
    pub compose_raw_buffer: crate::compose::ComposeBuffer,
    /// Tracks the current not-yet-corrected batch of utterances for Compose, so a
    /// correction pass covers a few sentences of real context instead of just one.
    pub compose_pending: crate::compose::ComposePendingState,
    /// Per-batch agent-chain history for Compose (what each agent did, in order), so the
    /// frontend can show a contribution log and let the user revert a batch to an
    /// earlier stage.
    pub compose_history: crate::compose::ComposeHistoryState,
    /// Cached embedded-or-remote LLM backend for Compose - `None` until first used, or
    /// after `invalidate_backend` following a settings change.
    pub compose_backend: crate::compose::ComposeBackendCache,
    /// Invalidates replies and preload attempts started against an older refinement
    /// provider. Blocking native inference cannot be force-killed safely, so stale work
    /// is discarded before it can mutate the document or status.
    pub compose_backend_generation: Arc<AtomicU64>,
    /// Set by a live provider/model change when the current document must be rerun after
    /// the replacement backend has finished loading and warming.
    pub compose_rerun_after_preload: Arc<AtomicBool>,
    pub compose_loading: Arc<Mutex<bool>>,
    pub compose_preload_error: Arc<Mutex<Option<String>>>,
    /// Session-only offload location selected in the UI or with `--offload-location`.
    pub compose_dump_dir_override: Arc<Mutex<Option<String>>>,
}

#[derive(Default)]
pub struct SessionStats {
    pub bytes_recorded: AtomicU64,
    pub words_transcribed: AtomicU64,
    pub transcriptions_count: AtomicU64,
    pub transcribe_ms_total: AtomicU64,
    pub gpu_count: AtomicU64,
    pub cpu_count: AtomicU64,
    pub api_count: AtomicU64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct HotkeyBindingState {
    pub bound: bool,
    pub listening: bool,
    pub detail: Option<String>,
    pub active_trigger: Option<String>,
}

impl Default for HotkeyBindingState {
    fn default() -> Self {
        Self {
            bound: false,
            listening: false,
            detail: None,
            active_trigger: None,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            config: Arc::new(Mutex::new(Config::default())),
            is_recording: Arc::new(Mutex::new(false)),
            is_mic_test_active: Arc::new(Mutex::new(false)),
            is_configuring_hotkey: Arc::new(Mutex::new(false)),
            hotkey_error: Arc::new(Mutex::new(None)),
            hotkey_binding_state: Arc::new(Mutex::new(HotkeyBindingState::default())),
            setup_status: Arc::new(Mutex::new(None)),
            hardware_hotkey: Arc::new(Mutex::new(HardwareHotkey::default())),
            cached_device: Arc::new(Mutex::new(None)),
            playback_stream: Arc::new(Mutex::new(None)),
            mic_test_samples: Arc::new(Mutex::new(Vec::new())),
        audio_engine: Arc::new(Mutex::new(None)),
            whisper_engine: Arc::new(Mutex::new(None)),
            whisper_loading: Arc::new(Mutex::new(false)),
            whisper_preload_error: Arc::new(Mutex::new(None)),
            whisper_load_generation: Arc::new(AtomicU64::new(0)),
            whisper_last_gpu_error: Arc::new(Mutex::new(None)),
            voxbridge_engine: Arc::new(Mutex::new(None)),
            faster_whisper_engine: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "linux")]
            hotkey_engine_cancel: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "linux")]
            wayland_input_sender: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "linux")]
            wayland_input_cancel: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "linux")]
            wayland_input_ready: Arc::new(Mutex::new(false)),
            #[cfg(target_os = "linux")]
            wayland_host_app_registration_error: Arc::new(Mutex::new(None)),
            display_backend: platform::initialize(),
            session_stats: Arc::new(SessionStats::default()),
            compose_buffer: Arc::new(Mutex::new(String::new())),
            compose_raw_buffer: Arc::new(Mutex::new(String::new())),
            compose_pending: Arc::new(Mutex::new(crate::compose::PendingBatch::default())),
            compose_history: Arc::new(Mutex::new(crate::compose::ComposeHistory::default())),
            compose_backend: Arc::new(Mutex::new(None)),
            compose_backend_generation: Arc::new(AtomicU64::new(0)),
            compose_rerun_after_preload: Arc::new(AtomicBool::new(false)),
            compose_loading: Arc::new(Mutex::new(false)),
            compose_preload_error: Arc::new(Mutex::new(None)),
            compose_dump_dir_override: Arc::new(Mutex::new(None)),
        }
    }
}
