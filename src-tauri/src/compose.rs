//! "VoxBridge Compose": an optional output mode where raw transcribed text lands
//! immediately in an in-app editor buffer, and a background pass through a small LLM
//! (embedded via VoxBridge, or proxied to a remote Ollama instance) cleans up
//! spacing/punctuation/context afterward - asynchronously, never blocking the live
//! transcription stream.

use crate::config::Config;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

/// Session-scoped (never persisted) counters for every real LLM call Compose makes -
/// live corrections and slider-triggered recomputes alike - mirroring the whisper-side
/// counters in `AppState::session_stats` but kept as plain module statics rather than
/// threaded through every function down to `run_agent_chain`, since this is the one
/// place all of them actually happen.
static COMPOSE_AGENT_RUNS: AtomicU64 = AtomicU64::new(0);
static COMPOSE_ACCEPTED: AtomicU64 = AtomicU64::new(0);
static COMPOSE_REJECTED: AtomicU64 = AtomicU64::new(0);
static COMPOSE_FAILED: AtomicU64 = AtomicU64::new(0);
static COMPOSE_MS_TOTAL: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeStatsSnapshot {
    pub agent_runs: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub failed: u64,
    pub ms_total: u64,
}

pub fn compose_stats_snapshot() -> ComposeStatsSnapshot {
    ComposeStatsSnapshot {
        agent_runs: COMPOSE_AGENT_RUNS.load(Ordering::Relaxed),
        accepted: COMPOSE_ACCEPTED.load(Ordering::Relaxed),
        rejected: COMPOSE_REJECTED.load(Ordering::Relaxed),
        failed: COMPOSE_FAILED.load(Ordering::Relaxed),
        ms_total: COMPOSE_MS_TOTAL.load(Ordering::Relaxed),
    }
}

/// Both the untouched raw transcript and the corrected version, plus whether a
/// correction pass is currently in flight (and which agent, if any, is running right
/// now) - the frontend shows these as two panes plus a working indicator so the user can
/// actually see what got changed and by what, not just the final result.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeState {
    pub raw_text: String,
    pub text: String,
    pub correcting: bool,
    pub active_agent: Option<String>,
}

pub type ComposeBuffer = Arc<Mutex<String>>;
type SharedComposeBackend = Arc<Mutex<voxbridge::LlmBackend>>;
pub type ComposeBackendCache = Arc<Mutex<Option<SharedComposeBackend>>>;
pub type ComposePendingState = Arc<Mutex<PendingBatch>>;
pub type ComposeHistoryState = Arc<Mutex<ComposeHistory>>;

/// One agent's attempted pass over a batch - kept even when rejected, so the
/// contribution history can show *why* a step didn't change anything, not just the
/// accepted ones.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStage {
    pub agent_id: String,
    pub agent_name: String,
    pub text: String,
    pub accepted: bool,
    pub note: Option<String>,
    /// The word-fidelity score this stage's reply actually scored, regardless of
    /// whether it was accepted - lets the frontend hypothetically re-evaluate accept/
    /// reject at a different threshold (the live fidelity-tuning slider) without
    /// calling the LLM again, since the reply text and its score are both already here.
    pub fidelity: f64,
}

/// One finalized batch's full agent-chain history: the original raw text, every agent's
/// attempt in order, and where its (possibly since-reverted) final text currently sits
/// in the corrected buffer - kept so a specific batch's chain can be reverted to an
/// earlier stage without disturbing anything before or after it.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchRecord {
    pub id: u64,
    #[serde(skip)]
    start: usize,
    #[serde(skip)]
    end: usize,
    pub raw_text: String,
    pub stages: Vec<AgentStage>,
}

#[derive(Default)]
pub struct ComposeHistory {
    batches: Vec<BatchRecord>,
    next_id: u64,
}

/// How many finalized batches' full stage-by-stage history to keep - old enough history
/// isn't worth holding onto for a revert the user will never reach for, and this keeps
/// memory bounded for very long dictation sessions.
const MAX_HISTORY_BATCHES: usize = 50;

/// Tracks the not-yet-corrected tail of the corrected buffer as a *batch* of utterances
/// rather than one at a time, so the LLM gets a few sentences of real context/flow to
/// work with instead of correcting each one in isolation.
///
/// The confirmed-prefix/revisable-tail design is independently implemented here and
/// informed by LocalAgreement in Whisper-Streaming and SimulStreaming: Macháček,
/// Dabre, and Bojar, "Turning Whisper into a Real-Time Transcription System"
/// (IJCNLP-AACL 2023). Those MIT-licensed projects are credited in NOTICE.md; no source
/// code from either project is incorporated here.
#[derive(Default)]
pub struct PendingBatch {
    /// Byte offset into the corrected buffer where this batch's uncorrected text begins.
    start: usize,
    /// How many utterances have been folded into this batch so far.
    count: usize,
    /// Bumped on every utterance; lets a scheduled debounce fire only if nothing newer
    /// has arrived to supersede it, without needing to explicitly cancel a tokio task.
    generation: u64,
    /// Only one mutable-tail refinement may run at a time. New speech is coalesced in
    /// this state and picked up by the same worker after its current snapshot finishes.
    in_flight: bool,
    /// Explicit whole-document rebuilds invalidate an older rolling snapshot. Ordinary
    /// appended speech does not: a completed pass may still safely update its exact
    /// unchanged source span while the new tail waits.
    discard_before_generation: u64,
    /// The generation of an explicit final/full-document pass. Rolling passes only
    /// refine through the last complete sentence and leave the trailing fragment
    /// unconfirmed; this generation is allowed to include that final fragment.
    full_document_generation: Option<u64>,
}

/// `Config::compose_context_sentences` sentinel meaning "no cap - only ever fire on the
/// debounce (a real pause), so a batch grows to cover everything said since the last
/// correction" - the "Everything" end of the context-width slider in Settings.
const CONTEXT_UNLIMITED: u32 = 0;
/// Eagerly loads (and warms up) the Compose backend in the background if Compose is the
/// active output method, so the first real correction does not pay the cold-load
/// cost (loading a GGUF model into VRAM, and - like whisper's GPU path - a one-time
/// shader-compile/warm-up cost the first time it's actually used). Called at app startup
/// and again whenever Compose-relevant settings change. If the selected backend has no
/// usable model yet, preparation downloads the default model before warming it.
pub fn spawn_compose_preload(app_handle: AppHandle) {
    use tauri::Manager;
    let state = app_handle.state::<crate::AppState>();
    let config = { state.config.lock().unwrap().clone() };

    if config.output_method != crate::config::OutputMethod::Compose {
        *state.compose_loading.lock().unwrap() = false;
        let _ = app_handle.emit(
            "compose-preload-status",
            serde_json::json!({ "loading": false, "error": null }),
        );
        return;
    }

    let backend_cache = state.compose_backend.clone();
    let backend_generation = state.compose_backend_generation.clone();
    let preload_generation = backend_generation.load(Ordering::SeqCst);
    {
        let guard = backend_cache.lock().unwrap();
        if guard.is_some() {
            *state.compose_loading.lock().unwrap() = false;
            let _ = app_handle.emit(
                "compose-preload-status",
                serde_json::json!({ "loading": false, "error": null }),
            );
            return; // already loaded
        }
    }

    let compose_loading = state.compose_loading.clone();
    let compose_preload_error = state.compose_preload_error.clone();
    let preload_cache: ComposeBackendCache = Arc::new(Mutex::new(None));
    *compose_loading.lock().unwrap() = true;
    *compose_preload_error.lock().unwrap() = None;

    tauri::async_runtime::spawn(async move {
        let _ = app_handle.emit(
            "compose-preload-status",
            serde_json::json!({ "loading": true, "error": null }),
        );
        crate::app::status::emit_status_to_frontend("Loading refinement model").await;

        let mut prepared_config = config;
        let preparation = if prepared_config.compose_backend == "embedded" {
            let configured_path = std::path::Path::new(&prepared_config.compose_model_path);
            if prepared_config.compose_model_path.trim().is_empty()
                || !configured_path.exists()
                || crate::app::commands::compose::uses_legacy_default_embedded_model(
                    configured_path,
                )
            {
                crate::log_info!("Compose: embedded model missing; downloading the default model");
                crate::app::status::emit_status_to_frontend("Downloading Compose model").await;
                crate::app::commands::compose::ensure_default_embedded_model()
                    .await
                    .map(|path| prepared_config.compose_model_path = path)
            } else {
                Ok(())
            }
        } else {
            if prepared_config.compose_ollama_model.trim().is_empty() {
                prepared_config.compose_ollama_model =
                    crate::app::commands::compose::DEFAULT_OLLAMA_MODEL.to_string();
            }
            crate::app::commands::compose::ensure_ollama_model(
                &prepared_config.compose_ollama_url,
                &prepared_config.compose_ollama_model,
            )
            .await
        };

        let result = match preparation {
            Ok(()) => {
                if backend_generation.load(Ordering::SeqCst) != preload_generation {
                    crate::log_info!(
                        "Compose: preload generation {} superseded before initialization",
                        preload_generation
                    );
                    return;
                }
                crate::app::status::emit_status_to_frontend("Initializing refinement model").await;
                {
                    let state = app_handle.state::<crate::AppState>();
                    *state.config.lock().unwrap() = prepared_config.clone();
                }
                let save_result =
                    crate::config::save_config(&prepared_config).map_err(|error| error.to_string());
                if let Err(error) = save_result {
                    Err(error)
                } else {
                    let _ = app_handle.emit("config-updated", prepared_config.clone());
                    let app_handle_for_blocking = app_handle.clone();
                    let preload_cache_for_blocking = preload_cache.clone();
                    crate::app::status::emit_status_to_frontend("Warming refinement model").await;
                    match tokio::task::spawn_blocking(move || {
                        ensure_backend_loaded(
                            &preload_cache_for_blocking,
                            &prepared_config,
                            &app_handle_for_blocking,
                        )?;
                        let backend = preload_cache_for_blocking
                            .lock()
                            .unwrap()
                            .as_ref()
                            .cloned()
                            .ok_or("Compose backend not loaded")?;
                        // Throwaway completion to force the model resident and any one-time
                        // GPU shader compilation to happen now, not on the user's first real use.
                        let result = backend
                            .lock()
                            .unwrap()
                            .complete(Some(WARMUP_PROMPT), "warm up.", 8);
                        result
                    })
                    .await
                    {
                        Ok(result) => result,
                        Err(error) => Err(error.to_string()),
                    }
                }
            }
            Err(error) => Err(error),
        };

        if backend_generation.load(Ordering::SeqCst) != preload_generation {
            crate::log_info!(
                "Compose: preload generation {} completed after being superseded; discarding it",
                preload_generation
            );
            return;
        }

        let error = match result {
            Ok(_) => {
                *backend_cache.lock().unwrap() = preload_cache.lock().unwrap().take();
                crate::log_info!("Compose: backend preloaded and warmed up");
                None
            }
            Err(error) => {
                crate::log_warn!("Compose: preload failed: {}", error);
                Some(error)
            }
        };
        *compose_loading.lock().unwrap() = false;
        *compose_preload_error.lock().unwrap() = error.clone();
        let _ = app_handle.emit(
            "compose-preload-status",
            serde_json::json!({ "loading": false, "error": error.clone() }),
        );
        let recording = {
            let state = app_handle.state::<crate::AppState>();
            let active = *state.is_recording.lock().unwrap();
            active
        };
        crate::app::status::emit_status_to_frontend(if recording { "Recording" } else { "Ready" }).await;

        if error.is_none() {
            rerun_after_backend_preload(&app_handle, preload_generation);
        }
    });
}

/// Small instruct models readily break character when the transcript reads like a
/// question or comment addressed to an assistant (e.g. "why is it doing that?") - they
/// answer it instead of cleaning it up. Every agent's own prompt (configured in
/// Settings, one per `Config::compose_agents` entry) is expected to say so explicitly -
/// see `default_compose_agents` in `config.rs` for the stock wording - and
/// `run_agent_chain` backs it up with a hard length-ratio/word-fidelity check that
/// discards any reply which drifted far beyond what that agent's own job would explain,
/// rather than trusting the model.
const WARMUP_PROMPT: &str = "You are a text-cleanup function. Reply with only the text given \
    to you, unchanged.";

fn engines_dir(app_handle: &AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    let resource_base = app_handle.path().resource_dir().ok();
    let dev_fallback_base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidate_bases: Vec<&std::path::Path> = resource_base
        .as_deref()
        .into_iter()
        .chain(std::iter::once(dev_fallback_base))
        .collect();
    candidate_bases.into_iter().find_map(|base| {
        let directory = voxbridge::resolve_engines_dir(&[base])?;
        let contains_llm_engine = std::fs::read_dir(&directory)
            .ok()?
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("voxbridge_llm_"));
        contains_llm_engine.then_some(directory)
    })
}

/// Loads (or returns the already-cached) `LlmBackend` for the current config. Cheap to
/// call repeatedly - the embedded model is only actually loaded once and cached.
fn ensure_backend_loaded(
    cache: &ComposeBackendCache,
    config: &Config,
    app_handle: &AppHandle,
) -> Result<(), String> {
    {
        let guard = cache.lock().unwrap();
        if guard.is_some() {
            return Ok(());
        }
    }

    let backend = if config.compose_backend == "ollama_remote" {
        if config.compose_ollama_model.is_empty() {
            return Err("No Ollama model selected in Settings.".to_string());
        }
        voxbridge::LlmBackend::OllamaRemote {
            base_url: config.compose_ollama_url.clone(),
            model: config.compose_ollama_model.clone(),
        }
    } else {
        if config.compose_model_path.is_empty() {
            return Err("No local model selected in Settings.".to_string());
        }
        let dir = engines_dir(app_handle)
            .ok_or_else(|| "Could not resolve VoxBridge engines directory".to_string())?;
        let engine = if config.compose_use_gpu {
            voxbridge::LlmEngine::load_best_gpu(&dir)
                .or_else(|_| voxbridge::LlmEngine::load_best(&dir))?
        } else {
            voxbridge::LlmEngine::load_best(&dir)?
        };
        let n_gpu_layers = if config.compose_use_gpu { 999 } else { 0 };
        let model = engine.load_model(&config.compose_model_path, 4096, n_gpu_layers)?;
        voxbridge::LlmBackend::Embedded(model)
    };

    *cache.lock().unwrap() = Some(Arc::new(Mutex::new(backend)));
    Ok(())
}

/// Drops any cached backend so the next call reloads from current config - used when
/// Compose-relevant settings change (backend, model, GPU toggle) while the app is running.
pub fn invalidate_backend(cache: &ComposeBackendCache, generation: &Arc<AtomicU64>) {
    generation.fetch_add(1, Ordering::SeqCst);
    *cache.lock().unwrap() = None;
}

fn rerun_after_backend_preload(app_handle: &AppHandle, generation: u64) {
    use tauri::Manager;
    let state = app_handle.state::<crate::AppState>();
    if state.compose_backend_generation.load(Ordering::SeqCst) != generation
        || !state.compose_rerun_after_preload.swap(false, Ordering::SeqCst)
    {
        return;
    }

    rerun_current_document(app_handle);
}

/// Rebuilds the refined document from the complete raw transcript with the current
/// backend and agent settings. This is deliberately one whole-document job: replaying
/// every overlapping contribution-history batch can saturate the model, mutate stale
/// offsets, and leave the visible document waiting behind work that is no longer useful.
pub fn rerun_current_document(app_handle: &AppHandle) {
    use tauri::Manager;
    let state = app_handle.state::<crate::AppState>();
    // Finalization should build on the rolling refined document. Replacing it with
    // the untouched raw transcript here discarded useful sentence and paragraph
    // structure that had already landed during recording.
    let current_text = state.compose_buffer.lock().unwrap().clone();
    if current_text.trim().is_empty() {
        return;
    }

    {
        let mut pending = state.compose_pending.lock().unwrap();
        pending.start = 0;
        pending.count = 1;
        pending.generation += 1;
        pending.discard_before_generation = pending.generation;
        pending.full_document_generation = Some(pending.generation);
    }
    clear_history(&state.compose_history);
    emit_state(
        app_handle,
        &state.compose_buffer,
        &state.compose_raw_buffer,
        true,
        None,
    );
    spawn_pending_correction(
        state.compose_buffer.clone(),
        state.compose_raw_buffer.clone(),
        state.compose_pending.clone(),
        state.compose_history.clone(),
        state.compose_backend.clone(),
        state.config.clone(),
        app_handle.clone(),
    );
}

/// Capitalizes the first letter of the text and the first letter after every `.`/`?`/`!`
/// - a deterministic fix-up for a real, observed model failure mode: the LLM will
/// correctly insert a missing sentence-ending period but leave the next word
/// lowercase (e.g. "story. correction working?"), since fixing that is exactly the kind
/// of small formatting slip a model is prone to and no prompt wording reliably prevents.
fn capitalize_sentence_starts(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut capitalize_next = true;
    for c in text.chars() {
        if capitalize_next && c.is_alphabetic() {
            result.extend(c.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
            if c == '.' || c == '?' || c == '!' {
                capitalize_next = true;
            } else if !c.is_whitespace() {
                capitalize_next = false;
            }
        }
    }
    result
}

fn collapse_adjacent_duplicate_words(text: &str) -> String {
    let mut result: Vec<&str> = Vec::new();
    let mut previous_normalized = String::new();
    let mut previous_ended_sentence = false;
    for token in text.split_whitespace() {
        let normalized: String = token
            .chars()
            .filter(|character| character.is_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        if !normalized.is_empty()
            && normalized == previous_normalized
            && !previous_ended_sentence
        {
            continue;
        }
        previous_ended_sentence = token.ends_with(['.', '!', '?']);
        previous_normalized = normalized;
        result.push(token);
    }
    result.join(" ")
}

fn preserve_boundary_whitespace(source: &str, replacement: &str) -> String {
    let leading = source.len() - source.trim_start().len();
    let trailing = source.len() - source.trim_end().len();
    let mut result = String::with_capacity(leading + replacement.len() + trailing);
    result.push_str(&source[..leading]);
    result.push_str(replacement.trim());
    if trailing > 0 && source.len() >= trailing {
        result.push_str(&source[source.len() - trailing..]);
    }
    result
}

/// Splits text into a lowercase, punctuation-stripped word multiset (a frequency count,
/// not a sequence) so word order changes from sentence-splitting don't register as edits.
fn word_multiset(text: &str) -> std::collections::HashMap<String, i32> {
    let mut map = std::collections::HashMap::new();
    for word in text.split_whitespace() {
        let normalized: String = word.chars().filter(|c| c.is_alphanumeric()).collect();
        if normalized.is_empty() {
            continue;
        }
        *map.entry(normalized.to_lowercase()).or_insert(0) += 1;
    }
    map
}

fn normalized_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// Repeating a phrase that appeared only once in the source is never a legitimate
/// cleanup, regardless of how permissive the user's fidelity threshold is.
fn introduces_repeated_phrase(raw: &str, corrected: &str) -> bool {
    const PHRASE_WORDS: usize = 3;
    fn counts(words: &[String]) -> std::collections::HashMap<String, usize> {
        let mut result = std::collections::HashMap::new();
        for phrase in words.windows(PHRASE_WORDS) {
            *result.entry(phrase.join("\u{1f}")).or_insert(0) += 1;
        }
        result
    }

    let raw_counts = counts(&normalized_words(raw));
    counts(&normalized_words(corrected))
        .into_iter()
        .any(|(phrase, count)| count >= 2 && count > *raw_counts.get(&phrase).unwrap_or(&0))
}

fn copies_read_only_context(input: &str, context: &str, corrected: &str) -> bool {
    const PHRASE_WORDS: usize = 5;
    let input_words = normalized_words(input);
    let context_words = normalized_words(context);
    let corrected_words = normalized_words(corrected);
    if context_words.len() < PHRASE_WORDS || corrected_words.len() < PHRASE_WORDS {
        return false;
    }

    let input_phrases: std::collections::HashSet<String> = input_words
        .windows(PHRASE_WORDS)
        .map(|words| words.join("\u{1f}"))
        .collect();
    let context_only_phrases: std::collections::HashSet<String> = context_words
        .windows(PHRASE_WORDS)
        .map(|words| words.join("\u{1f}"))
        .filter(|phrase| !input_phrases.contains(phrase))
        .collect();
    let copied_phrase = corrected_words
        .windows(PHRASE_WORDS)
        .map(|words| words.join("\u{1f}"))
        .any(|phrase| context_only_phrases.contains(&phrase));
    if copied_phrase {
        return true;
    }

    // A small model can also lift a few context words while changing their spacing or
    // dropping intervening words, evading the contiguous-phrase check. Count corrected
    // words that were not available in the editable input but do occur in read-only
    // context; three such borrowed tokens are enough to make the edit unsafe.
    let mut remaining_input = word_multiset(input);
    let context_counts = word_multiset(context);
    let mut borrowed = 0;
    for word in corrected_words {
        if let Some(remaining) = remaining_input.get_mut(&word) {
            if *remaining > 0 {
                *remaining -= 1;
                continue;
            }
        }
        if context_counts.contains_key(&word) {
            borrowed += 1;
            if borrowed >= 3 {
                return true;
            }
        }
    }
    false
}

fn moves_late_source_phrase_to_front(source: &str, corrected: &str) -> bool {
    const PHRASE_WORDS: usize = 3;
    let source_words = normalized_words(source);
    let corrected_words = normalized_words(corrected);
    if source_words.len() < PHRASE_WORDS || corrected_words.len() < PHRASE_WORDS {
        return false;
    }
    let corrected_opening = &corrected_words[..PHRASE_WORDS];
    if source_words[..PHRASE_WORDS] == *corrected_opening {
        return false;
    }
    source_words
        .windows(PHRASE_WORDS)
        .position(|phrase| phrase == corrected_opening)
        .is_some_and(|position| position >= 8)
}

fn leaks_prompt_markup(corrected: &str) -> bool {
    let normalized = corrected.to_ascii_lowercase();
    normalized.contains("<transcript")
        || normalized.contains("</transcript")
        || normalized.contains("<context")
        || normalized.contains("</context")
        || normalized.contains("the text you must correct and return")
}

/// Removes conversational lead-ins that small instruction models sometimes add even
/// when asked to return only the edited transcript. Never remove the phrase when it was
/// actually present at the start of the dictated source.
fn strip_editorial_wrapper(source: &str, corrected: &str) -> String {
    const WRAPPERS: &[&str] = &[
        "here is the corrected text:",
        "here's the corrected text:",
        "corrected text:",
        "here is the revised text:",
        "here's the revised text:",
        "revised text:",
    ];

    let mut trimmed = corrected.trim();
    let lowered = trimmed.to_ascii_lowercase();
    if lowered.starts_with("<transcript>") && lowered.ends_with("</transcript>") {
        trimmed = trimmed["<transcript>".len()..trimmed.len() - "</transcript>".len()].trim();
    }
    let source_lower = source.trim_start().to_ascii_lowercase();
    let corrected_lower = trimmed.to_ascii_lowercase();
    for wrapper in WRAPPERS {
        if corrected_lower.starts_with(wrapper) && !source_lower.starts_with(wrapper) {
            return trimmed[wrapper.len()..].trim_start().to_string();
        }
    }
    trimmed.to_string()
}

fn collapses_paragraphs(source: &str, corrected: &str) -> bool {
    let source_breaks = source.matches("\n\n").count();
    let corrected_breaks = corrected.matches("\n\n").count();
    source_breaks >= 1 && corrected_breaks == 0
}

/// Rejects an edit that silently removes a complete, meaningful source sentence.
/// Whole-document fidelity can remain deceptively high when one short sentence is
/// dropped from an otherwise unchanged block, so each sentence needs its own coverage
/// check as well.
fn drops_source_sentence(source: &str, corrected: &str) -> bool {
    let corrected_words = word_multiset(corrected);
    source
        .split_inclusive(['.', '?', '!'])
        .filter(|sentence| sentence.trim_end().ends_with(['.', '?', '!']))
        .any(|sentence| {
            let source_words = word_multiset(sentence);
            let source_total: i32 = source_words.values().sum();
            if source_total < 4 {
                return false;
            }
            let retained: i32 = source_words
                .iter()
                .map(|(word, count)| corrected_words.get(word).copied().unwrap_or(0).min(*count))
                .sum();
            retained as f64 / (source_total as f64) < 0.70
        })
}

fn has_malformed_word_joins(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.windows(2).any(|pair| {
        (pair[0].is_lowercase() && pair[1].is_uppercase())
            || (matches!(pair[0], '.' | '?' | '!' | ',') && pair[1].is_alphabetic())
    })
}

/// Rejects new tiny sentence fragments introduced by refinement. These are usually
/// punctuation mistakes at a streaming chunk boundary rather than intentional prose.
fn introduces_short_sentence_fragment(source: &str, corrected: &str) -> bool {
    fn short_sentence_count(text: &str) -> usize {
        text.split_inclusive(['.', '?', '!'])
            .filter(|sentence| sentence.trim_end().ends_with(['.', '?', '!']))
            .filter(|sentence| {
                let word_count = normalized_words(sentence).len();
                (1..=2).contains(&word_count)
            })
            .count()
    }

    short_sentence_count(corrected) > short_sentence_count(source)
}

/// Hard safety check for clause shuffling. The ordinary fidelity score ignores order
/// so punctuation and sentence-boundary edits do not look destructive; this separate
/// longest-common-subsequence score catches a model that keeps similar vocabulary but
/// moves or replaces whole clauses.
fn ordered_word_fidelity(source: &str, corrected: &str) -> f64 {
    let source_words = normalized_words(source);
    let corrected_words = normalized_words(corrected);
    if source_words.is_empty() {
        return 1.0;
    }
    let source_words = &source_words[..source_words.len().min(2048)];
    let corrected_words = &corrected_words[..corrected_words.len().min(2048)];
    let mut previous = vec![0_u16; corrected_words.len() + 1];
    let mut current = vec![0_u16; corrected_words.len() + 1];
    for source_word in source_words {
        for (index, corrected_word) in corrected_words.iter().enumerate() {
            current[index + 1] = if source_word == corrected_word {
                previous[index].saturating_add(1)
            } else {
                current[index].max(previous[index + 1])
            };
        }
        std::mem::swap(&mut previous, &mut current);
        current.fill(0);
    }
    previous[corrected_words.len()] as f64 / source_words.len() as f64
}

fn add_paragraph_breaks_to_long_block(text: &str) -> String {
    if text.contains("\n\n") || text.split_whitespace().count() < 80 {
        return text.to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len() + 8);
    let mut sentences = 0_u8;
    let mut paragraph_break_pending = false;
    for (index, character) in chars.iter().copied().enumerate() {
        if paragraph_break_pending && character.is_whitespace() {
            if !result.ends_with("\n\n") {
                result.push_str("\n\n");
            }
            paragraph_break_pending = false;
            continue;
        }
        result.push(character);
        if matches!(character, '.' | '?' | '!')
            && !chars
                .get(index + 1)
                .is_some_and(|next| matches!(next, '.' | '?' | '!'))
        {
            sentences += 1;
            if sentences >= 4 {
                sentences = 0;
                paragraph_break_pending = true;
            }
        }
    }
    result
}

/// The alphanumeric-only, lowercase, whitespace/punctuation-stripped character
/// *sequence* (order preserved, unlike a bag/multiset) - used to detect "the exact same
/// content, just with a word boundary or punctuation moved" (e.g. "Goodnight" ->
/// "Good night"): that produces an identical sequence, while genuine paraphrasing
/// (different words, even ones sharing similar letter frequencies) does not.
fn normalized_char_sequence(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Estimates how much of `raw`'s actual content survived into `corrected`, as a fraction
/// in [0, 1] - 1.0 means everything is accounted for. A pure length-ratio check misses
/// same-length paraphrasing (rewording a clause instead of just cleaning it up); this
/// catches it by checking whether the content itself changed, not just its length.
///
/// First checks whether the two are identical once whitespace/punctuation is stripped -
/// a pure word-boundary shift (a compound word getting spaced out, a run-on gaining
/// sentence breaks) produces exactly that, so it's an automatic pass regardless of how
/// differently the words tokenize. Otherwise falls back to word-level multiset
/// comparison, which still tolerates the word-count churn a real cleanup causes
/// (contractions expanding, etc.) without being fooled by wholesale paraphrasing the way
/// a bag-of-characters comparison would (natural language has similar letter frequencies
/// regardless of which words are actually used, so that comparison alone is too weak a
/// signal to catch rewording).
fn word_fidelity(raw: &str, corrected: &str) -> f64 {
    if normalized_char_sequence(raw) == normalized_char_sequence(corrected) {
        return 1.0;
    }

    let raw_words = word_multiset(raw);
    let corrected_words = word_multiset(corrected);
    let raw_total: i32 = raw_words.values().sum();
    if raw_total == 0 {
        return 1.0;
    }

    let mut mismatched = 0i32;
    for (word, &count) in &raw_words {
        let corrected_count = *corrected_words.get(word).unwrap_or(&0);
        if corrected_count < count {
            mismatched += count - corrected_count;
        }
    }
    for (word, &count) in &corrected_words {
        let raw_count = *raw_words.get(word).unwrap_or(&0);
        if count > raw_count {
            mismatched += count - raw_count;
        }
    }
    1.0 - (mismatched as f64 / (raw_total as f64 * 2.0)).min(1.0)
}

/// Maps the "Correction Timing" setting to an actual debounce duration.
fn latency_debounce_ms(latency: &str) -> u64 {
    match latency {
        "low" => 800,
        "high" => 3500,
        _ => 1800, // "medium" and any unrecognized value
    }
}

/// Maps a per-agent "speed" setting to a token-budget multiplier (tokens allowed per
/// word of input) - a fast/narrow agent gets less runway than a slow/thorough one.
fn speed_token_multiplier(speed: &str) -> i32 {
    match speed {
        "low" => 2,
        "high" => 6,
        _ => 3, // "medium" and any unrecognized value
    }
}

/// Appends `raw_text` to both the raw and corrected Compose buffers immediately (so the
/// user sees it right away in both panes), then folds it into the current pending batch.
/// A correction pass only actually runs once the batch reaches the configured size (or,
/// failing that, after a real pause) - so the model sees a few sentences of real
/// flow/context at once instead of correcting each one in total isolation. Never blocks
/// the caller.
pub fn append_and_correct(
    raw_text: String,
    buffer: ComposeBuffer,
    raw_buffer: ComposeBuffer,
    pending: ComposePendingState,
    history: ComposeHistoryState,
    backend_cache: ComposeBackendCache,
    config: Arc<Mutex<Config>>,
    app_handle: AppHandle,
) {
    let raw_text = raw_text.replace("[BLANK_AUDIO]", "");
    let raw_text = raw_text.trim().to_string();
    if raw_text.is_empty() {
        return;
    }

    {
        let mut raw_buf = raw_buffer.lock().unwrap();
        if !raw_buf.is_empty() && !raw_buf.ends_with(' ') {
            raw_buf.push(' ');
        }
        raw_buf.push_str(&raw_text);
    }

    let (context_sentences, pause_only, debounce_ms) = {
        let cfg = config.lock().unwrap();
        (
            cfg.compose_context_sentences,
            cfg.compose_pause_only,
            latency_debounce_ms(&cfg.compose_edit_latency),
        )
    };
    let (batch_count, my_generation) = {
        let mut buf = buffer.lock().unwrap();
        if !buf.is_empty() && !buf.ends_with(' ') {
            buf.push(' ');
        }
        let mut p = pending.lock().unwrap();
        if p.count == 0 && p.start >= buf.len() {
            // Include editable overlap from the already-composed text. Later speech
            // often explicitly corrects an earlier recognition error ("I said test,
            // not dust"); read-only reference context cannot repair that earlier word.
            // Live passes revisit a bounded recent passage while the earlier document
            // remains available as read-only context. The explicit stop-time pass is
            // responsible for revisiting the full refined document.
            //
            // Do not move this boundary forward merely because an inference was
            // launched. If that pass is superseded by newer speech, its unresolved
            // opening text must remain part of the next pass until a result actually
            // lands.
            let live_edit_sentences = if context_sentences == CONTEXT_UNLIMITED {
                3
            } else {
                context_sentences.clamp(1, 6)
            };
            p.start = editable_context_start(&buf, live_edit_sentences);
        }
        // The refined pane should never visibly regress to an uncapitalized raw
        // sentence while an asynchronous agent pass is pending or superseded by newer
        // speech. Keep the raw pane exact, but apply the deterministic sentence-start
        // cleanup immediately to newly appended refined text.
        let cleaned_raw = collapse_adjacent_duplicate_words(&raw_text);
        let begins_sentence = buf.trim_end().is_empty()
            || buf.trim_end().ends_with(['.', '?', '!', '\n']);
        let immediate_cleanup = if begins_sentence {
            capitalize_sentence_starts(&cleaned_raw)
        } else {
            cleaned_raw
        };
        buf.push_str(&immediate_cleanup);
        p.count += 1;
        p.generation += 1;
        (p.count, p.generation)
    };
    emit_state(&app_handle, &buffer, &raw_buffer, true, None);

    let fire_now = !pause_only
        && context_sentences != CONTEXT_UNLIMITED
        && batch_count >= context_sentences as usize;

    if fire_now {
        spawn_pending_correction(
            buffer,
            raw_buffer,
            pending,
            history,
            backend_cache,
            config,
            app_handle,
        );
        return;
    }

    // Not enough utterances yet to hit the configured batch size (or pause-only mode is
    // on) - schedule a debounced fire so a trailing partial batch still gets corrected
    // once the user actually pauses.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(debounce_ms)).await;

        let should_fire = {
            let p = pending.lock().unwrap();
            if p.generation == my_generation && p.count > 0 {
                true
            } else {
                false
            }
        };
        if should_fire {
            spawn_pending_correction(
                buffer,
                raw_buffer,
                pending,
                history,
                backend_cache,
                config,
                app_handle,
            );
        }
    });
}

fn editable_context_start(text: &str, context_sentences: u32) -> usize {
    if context_sentences == CONTEXT_UNLIMITED {
        return 0;
    }

    let mut boundaries_seen = 0_u32;
    for (index, character) in text.char_indices().rev() {
        if matches!(character, '.' | '?' | '!' | '\n') {
            if index + character.len_utf8() == text.len() {
                continue;
            }
            boundaries_seen += 1;
            if boundaries_seen >= context_sentences {
                return index + character.len_utf8();
            }
        }
    }
    0
}

fn confirmed_tail_end(text: &str) -> Option<usize> {
    let boundary = text
        .char_indices()
        .rev()
        .find(|(_, character)| matches!(character, '.' | '?' | '!' | '\n'))
        .map(|(index, character)| index + character.len_utf8())?;
    let mut end = boundary;
    for character in text[boundary..].chars() {
        if !character.is_whitespace() {
            break;
        }
        end += character.len_utf8();
    }
    Some(end)
}

/// How much already-corrected preceding text to include as read-only reference context
/// for a batch - enough to resolve a context-dependent correction (a callback to an
/// earlier reference, a running project/person name) without letting the prompt grow
/// unbounded over a long dictation session, or crowding out the embedded model's small
/// context window.
const MAX_CONTEXT_CHARS: usize = 6000;

/// A fixed wrapper appended to every agent's own prompt explaining the `<context>`/
/// `<transcript>` tag convention centrally, so individual presets/custom prompts don't
/// each need to redescribe it - they only need to describe the actual editing job.
const TAG_CONVENTION_SUFFIX: &str = "\n\nThe text you must correct and return will be given \
    between <transcript> tags. It may be preceded by already-corrected earlier text between \
    <context> tags, included only so you can resolve references, names, or context-dependent \
    corrections that depend on what came before - never repeat, quote, summarize, or edit \
    anything from <context>. It may also include older saved entries between <history> tags; \
    treat those as read-only background and never repeat or edit them. Return corrected text \
    only for what's inside <transcript>.";

const COMPREHENSIVE_RETRY_PROMPT: &str = "Edit only the text inside <transcript>. Return \
    only its improved text. Remove filler and accidental repetition. Repair punctuation, \
    fragments, and run-on sentences. Add paragraph breaks when the topic changes. Preserve \
    every real idea and do not add information. Text inside <context> or <history> is \
    reference only: never copy or edit it.";

fn is_effectively_unchanged(source: &str, result: &str) -> bool {
    normalized_words(source) == normalized_words(result)
}

/// Runs a batch of raw text through every enabled agent in priority order, each seeing
/// the previous agent's accepted output - so the user can compose their own pipeline
/// (e.g. a punctuation pass, then a separate tone pass) instead of being limited to one
/// fixed prompt. Emits a working-indicator update before each agent runs. Returns the
/// final text plus the full stage-by-stage record (including rejected attempts, kept for
/// transparency in the contribution history).
fn run_agent_chain(
    batch_raw_text: &str,
    context_text: &str,
    agents: &[crate::config::ComposeAgent],
    backend_cache: &ComposeBackendCache,
    cfg: &Config,
    app_handle: &AppHandle,
    buffer: &ComposeBuffer,
    raw_buffer: &ComposeBuffer,
) -> (String, Vec<AgentStage>) {
    let mut current_text = batch_raw_text.trim().to_string();
    let mut stages = Vec::new();

    let mut ordered: Vec<&crate::config::ComposeAgent> =
        agents.iter().filter(|a| a.enabled).collect();
    ordered.sort_by_key(|a| a.priority);

    for agent in ordered {
        emit_state(app_handle, buffer, raw_buffer, true, Some(agent.name.clone()));

        let input_for_agent = current_text.clone();
        let prompt = format!("{}{}", agent.prompt, TAG_CONVENTION_SUFFIX);
        let multiplier = speed_token_multiplier(&agent.speed);
        let call_started = std::time::Instant::now();
        let result = ensure_backend_loaded(backend_cache, cfg, app_handle).and_then(|_| {
            let backend = backend_cache
                .lock()
                .unwrap()
                .as_ref()
                .cloned()
                .ok_or("Compose backend not loaded")?;
            let trimmed = input_for_agent.trim();
            let history_context = if cfg.enable_history && agent.include_history {
                crate::history::load_history()
                    .ok()
                    .map(|history| {
                        history
                            .items
                            .into_iter()
                            .take(agent.history_items.clamp(1, 20))
                            .map(|item| item.text)
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .filter(|text| !text.is_empty())
                    .map(|text| {
                        let mut start = text.len().saturating_sub(4000);
                        while start < text.len() && !text.is_char_boundary(start) {
                            start += 1;
                        }
                        text[start..].to_string()
                    })
            } else {
                None
            };
            let mut user_prompt = String::new();
            if let Some(history_context) = history_context {
                user_prompt.push_str("<history>\n");
                user_prompt.push_str(&history_context);
                user_prompt.push_str("\n</history>\n");
            }
            if !context_text.is_empty() {
                user_prompt.push_str("<context>\n");
                user_prompt.push_str(context_text);
                user_prompt.push_str("\n</context>\n");
            }
            user_prompt.push_str("<transcript>\n");
            user_prompt.push_str(trimmed);
            user_prompt.push_str("\n</transcript>");
            let max_tokens =
                ((trimmed.split_whitespace().count() as i32 * multiplier) + 24).clamp(32, 2048);
            let first_result = backend
                .lock()
                .unwrap()
                .complete(Some(prompt.as_str()), &user_prompt, max_tokens)?;

            // Small embedded instruction models occasionally satisfy a complicated
            // editing prompt by simply echoing the transcript. That is a safe answer,
            // but it makes Comprehensive Rewrite appear to be working while doing
            // nothing. Retry that one specific failure mode with a shorter, direct
            // editorial reminder. Both attempts still pass through the same fidelity,
            // repetition, prompt-leak, and length safeguards below.
            if agent.preset_id.as_deref() == Some("comprehensive-rewrite")
                && is_effectively_unchanged(trimmed, &first_result)
            {
                crate::log_info!(
                    "Compose: agent '{}' echoed the source; retrying comprehensive edit",
                    agent.name
                );
                backend
                    .lock()
                    .unwrap()
                    .complete(Some(COMPREHENSIVE_RETRY_PROMPT), &user_prompt, max_tokens)
            } else {
                Ok(first_result)
            }
        });
        COMPOSE_AGENT_RUNS.fetch_add(1, Ordering::Relaxed);
        COMPOSE_MS_TOTAL.fetch_add(call_started.elapsed().as_millis() as u64, Ordering::Relaxed);

        match result {
            Ok(reply) => {
                let raw_trimmed = input_for_agent.trim();
                let reply_trimmed = strip_editorial_wrapper(raw_trimmed, &reply);
                let ballooned =
                    reply_trimmed.len() as f64 > (raw_trimmed.len() as f64 * 1.6 + 60.0);
                let fidelity = word_fidelity(raw_trimmed, &reply_trimmed);
                let reworded = fidelity < agent.min_fidelity;
                let repeated = introduces_repeated_phrase(raw_trimmed, &reply_trimmed);
                let prompt_leak = leaks_prompt_markup(&reply_trimmed);
                let collapsed_paragraphs = collapses_paragraphs(raw_trimmed, &reply_trimmed);
                let dropped_sentence = drops_source_sentence(raw_trimmed, &reply_trimmed);
                let malformed_joins = has_malformed_word_joins(&reply_trimmed);
                let short_fragment =
                    introduces_short_sentence_fragment(raw_trimmed, &reply_trimmed);
                let ordered_fidelity = ordered_word_fidelity(raw_trimmed, &reply_trimmed);
                let reordered = ordered_fidelity < 0.72;
                let copied_context =
                    copies_read_only_context(raw_trimmed, context_text, &reply_trimmed);
                let moved_late_opening =
                    moves_late_source_phrase_to_front(raw_trimmed, &reply_trimmed);
                if ballooned
                    || reworded
                    || repeated
                    || prompt_leak
                    || collapsed_paragraphs
                    || dropped_sentence
                    || malformed_joins
                    || short_fragment
                    || reordered
                    || copied_context
                    || moved_late_opening
                {
                    COMPOSE_REJECTED.fetch_add(1, Ordering::Relaxed);
                    crate::log_info!(
                        "Compose: agent '{}' rejected (word fidelity {:.2}, ordered fidelity {:.2}, repeated={}, prompt_leak={}, collapsed_paragraphs={}, dropped_sentence={}, malformed_joins={}, short_fragment={}, copied_context={}, moved_late_opening={})\n  raw:   {}\n  reply: {}",
                        agent.name,
                        fidelity,
                        ordered_fidelity,
                        repeated,
                        prompt_leak,
                        collapsed_paragraphs,
                        dropped_sentence,
                        malformed_joins,
                        short_fragment,
                        copied_context,
                        moved_late_opening,
                        raw_trimmed,
                        reply_trimmed
                    );
                    stages.push(AgentStage {
                        agent_id: agent.id.clone(),
                        agent_name: agent.name.clone(),
                        text: reply_trimmed,
                        accepted: false,
                        note: Some(format!(
                            "Rejected by output safeguards (word fidelity {:.2})",
                            fidelity,
                        )),
                        fidelity,
                    });
                } else {
                    COMPOSE_ACCEPTED.fetch_add(1, Ordering::Relaxed);
                    let capitalized = capitalize_sentence_starts(&reply_trimmed);
                    let final_text = if agent.preset_id.as_deref()
                        == Some("comprehensive-rewrite")
                    {
                        add_paragraph_breaks_to_long_block(&capitalized)
                    } else {
                        capitalized
                    };
                    crate::log_info!(
                        "Compose: agent '{}' accepted (word fidelity {:.2})\n  raw:   {}\n  final: {}",
                        agent.name,
                        fidelity,
                        raw_trimmed,
                        final_text
                    );
                    current_text = final_text.clone();
                    stages.push(AgentStage {
                        agent_id: agent.id.clone(),
                        agent_name: agent.name.clone(),
                        text: final_text,
                        accepted: true,
                        note: None,
                        fidelity,
                    });
                }
            }
            Err(error) => {
                COMPOSE_FAILED.fetch_add(1, Ordering::Relaxed);
                crate::log_info!("Compose: agent '{}' failed: {}", agent.name, error);
                stages.push(AgentStage {
                    agent_id: agent.id.clone(),
                    agent_name: agent.name.clone(),
                    text: current_text.clone(),
                    accepted: false,
                    note: Some(format!("Failed: {}", error)),
                    fidelity: 0.0,
                });
            }
        }
    }

    (current_text, stages)
}

/// Sends everything in `buffer[batch_start..]` (the raw, uncorrected text accumulated
/// since the last correction) through the configured agent chain and, once it's done,
/// patches only that span - the raw pane is never touched, so there's always an honest
/// "what was actually said" reference next to "what Compose changed it to." Records the
/// full per-agent history for that batch so it can be reviewed or reverted later.
fn spawn_pending_correction(
    buffer: ComposeBuffer,
    raw_buffer: ComposeBuffer,
    pending: ComposePendingState,
    history: ComposeHistoryState,
    backend_cache: ComposeBackendCache,
    config: Arc<Mutex<Config>>,
    app_handle: AppHandle,
) {
    use tauri::Manager;
    let (batch_start, document_generation) = {
        let mut pending_state = pending.lock().unwrap();
        if pending_state.in_flight || pending_state.count == 0 {
            return;
        }
        pending_state.in_flight = true;
        pending_state.count = 0;
        (pending_state.start, pending_state.generation)
    };
    // A provider switch or newly appended utterance can supersede this document
    // snapshot while inference is still running. Track the pending-document revision
    // as well as the backend generation so an older full-document pass can never patch
    // over newer speech or seed the next pass with duplicated text.
    let correction_generation = app_handle
        .state::<crate::AppState>()
        .compose_backend_generation
        .load(Ordering::SeqCst);
    let full_document_pass = pending.lock().unwrap().full_document_generation
        == Some(document_generation);
    let (batch_raw_text, context_text) = {
        let buf = buffer.lock().unwrap();
        if batch_start > buf.len() {
            let buffer_len = buf.len();
            drop(buf);
            let mut pending_state = pending.lock().unwrap();
            pending_state.in_flight = false;
            pending_state.count = 0;
            pending_state.start = buffer_len;
            drop(pending_state);
            emit_state(&app_handle, &buffer, &raw_buffer, false, None);
            return;
        }
        let available = &buf[batch_start..];
        let relative_end = if full_document_pass {
            available.len()
        } else if let Some(end) = confirmed_tail_end(available) {
            end
        } else {
            drop(buf);
            let mut pending_state = pending.lock().unwrap();
            pending_state.in_flight = false;
            pending_state.count = 0;
            drop(pending_state);
            emit_state(&app_handle, &buffer, &raw_buffer, false, None);
            return;
        };
        let batch_raw_text = available[..relative_end].to_string();
        // Recent already-corrected text as read-only reference context, so a
        // context-dependent correction (a name, a running reference, a callback to
        // something said earlier) has something to resolve against even though this
        // batch only ever *edits* its own span. Bounded and char-boundary safe so a
        // long session doesn't grow the prompt without limit or panic on a UTF-8 split.
        let context_start = if batch_start > MAX_CONTEXT_CHARS {
            let mut i = batch_start - MAX_CONTEXT_CHARS;
            while i > 0 && !buf.is_char_boundary(i) {
                i += 1;
            }
            i
        } else {
            0
        };
        let context_text = buf[context_start..batch_start].trim().to_string();
        (batch_raw_text, context_text)
    };
    if batch_raw_text.trim().is_empty() {
        let buffer_len = buffer.lock().unwrap().len();
        let mut pending_state = pending.lock().unwrap();
        pending_state.in_flight = false;
        pending_state.start = buffer_len;
        return;
    }

    tauri::async_runtime::spawn(async move {
        let cfg = { config.lock().unwrap().clone() };
        let batch_raw_for_blocking = batch_raw_text.clone();
        let context_for_blocking = context_text.clone();
        let app_handle_for_blocking = app_handle.clone();
        let buffer_for_blocking = buffer.clone();
        let raw_buffer_for_blocking = raw_buffer.clone();
        let backend_for_blocking = backend_cache.clone();
        let agents = cfg.compose_agents.clone();

        let (final_text, mut stages) = tokio::task::spawn_blocking(move || {
            run_agent_chain(
                &batch_raw_for_blocking,
                &context_for_blocking,
                &agents,
                &backend_for_blocking,
                &cfg,
                &app_handle_for_blocking,
                &buffer_for_blocking,
                &raw_buffer_for_blocking,
            )
        })
        .await
        .unwrap_or_else(|error| {
            crate::log_info!("Compose: agent chain task panicked: {}", error);
            (batch_raw_text.trim().to_string(), Vec::new())
        });

        let current_generation = app_handle
            .state::<crate::AppState>()
            .compose_backend_generation
            .load(Ordering::SeqCst);
        if current_generation != correction_generation {
            crate::log_info!(
                "Compose: discarded correction from backend generation {} after switch to {}",
                correction_generation,
                current_generation
            );
            let still_pending = {
                let mut pending_state = pending.lock().unwrap();
                pending_state.in_flight = false;
                pending_state.count > 0
            };
            emit_state(&app_handle, &buffer, &raw_buffer, still_pending, None);
            if still_pending {
                spawn_pending_correction(
                    buffer,
                    raw_buffer,
                    pending,
                    history,
                    backend_cache,
                    config,
                    app_handle,
                );
            }
            return;
        }

        let discard_result = pending.lock().unwrap().discard_before_generation > document_generation;
        let mut applied = false;
        let mut applied_text_len = final_text.len();
        let mut rejected_splice_repetition = false;
        {
            let mut buf = buffer.lock().unwrap();
            // Patch only the exact snapshot this job processed. Newer speech may have
            // been appended after it; preserve that live tail instead of discarding a
            // useful rewrite or replacing the entire document. If another correction
            // already changed this same span, the prefix no longer matches and this
            // stale result is safely ignored.
            if !discard_result && batch_start <= buf.len() {
                let snapshot_end = batch_start + batch_raw_text.len();
                let snapshot_is_ours = snapshot_end <= buf.len()
                    && buf.is_char_boundary(batch_start)
                    && buf.is_char_boundary(snapshot_end)
                    && buf[batch_start..snapshot_end] == batch_raw_text;
                if snapshot_is_ours {
                    let replacement =
                        preserve_boundary_whitespace(&batch_raw_text, &final_text);
                    let mut candidate = String::with_capacity(
                        buf.len() - batch_raw_text.len() + replacement.len(),
                    );
                    candidate.push_str(&buf[..batch_start]);
                    candidate.push_str(&replacement);
                    candidate.push_str(&buf[snapshot_end..]);
                    if introduces_repeated_phrase(&buf, &candidate) {
                        rejected_splice_repetition = true;
                        applied_text_len = batch_raw_text.len();
                        crate::log_info!(
                            "Compose: rejected rewrite because it introduced repetition across the editable splice boundary"
                        );
                    } else {
                        applied_text_len = replacement.len();
                        buf.replace_range(batch_start..snapshot_end, &replacement);
                        applied = true;
                    }
                }
            }
        }

        if rejected_splice_repetition {
            if let Some(stage) = stages.iter_mut().rev().find(|stage| stage.accepted) {
                stage.accepted = false;
                stage.note = Some(
                    "Rejected because the rewrite duplicated text across the live edit boundary"
                        .to_string(),
                );
            }
        }

        if discard_result {
            crate::log_info!(
                "Compose: discarded rolling generation {} after a newer full-document rebuild was requested",
                document_generation
            );
        } else if applied {
            let confirmed_end = batch_start + applied_text_len;
            let mut pending_state = pending.lock().unwrap();
            if pending_state.generation == document_generation && pending_state.count == 0 {
                pending_state.start = confirmed_end;
            } else {
                crate::log_info!(
                    "Compose: spliced generation {} rewrite while newer speech remained queued at generation {}",
                    document_generation,
                    pending_state.generation
                );
            }
        } else if pending.lock().unwrap().generation != document_generation {
            crate::log_info!(
                "Compose: skipped generation {} rewrite because its source span had already changed",
                document_generation
            );
        }

        {
            let mut hist = history.lock().unwrap();
            let id = hist.next_id;
            hist.next_id += 1;
            hist.batches.push(BatchRecord {
                id,
                start: batch_start,
                end: batch_start + applied_text_len,
                raw_text: batch_raw_text.trim().to_string(),
                stages,
            });
            if hist.batches.len() > MAX_HISTORY_BATCHES {
                let overflow = hist.batches.len() - MAX_HISTORY_BATCHES;
                hist.batches.drain(0..overflow);
            }
        }

        let still_pending = {
            let mut pending_state = pending.lock().unwrap();
            pending_state.in_flight = false;
            if pending_state.full_document_generation == Some(document_generation) {
                pending_state.full_document_generation = None;
            }
            pending_state.count > 0
        };
        emit_state(&app_handle, &buffer, &raw_buffer, still_pending, None);
        if still_pending {
            spawn_pending_correction(
                buffer,
                raw_buffer,
                pending,
                history,
                backend_cache,
                config,
                app_handle,
            );
            return;
        }
        if !still_pending {
            let recording = {
                let state = app_handle.state::<crate::AppState>();
                let is_recording = *state.is_recording.lock().unwrap();
                is_recording
            };
            if !recording {
                tauri::async_runtime::spawn(async {
                    crate::app::status::emit_status_to_frontend("Ready").await;
                });
            }
        }
    });
}

/// Reverts a specific batch's text to the output after its first `keep_stage_count`
/// agent stages (0 = the original raw text, i.e. undo every agent that touched it),
/// splices that back into the corrected buffer at that batch's current position, and
/// shifts every later batch's recorded position to account for the resulting length
/// change - so a revert to an old batch doesn't corrupt anything written since.
pub fn revert_batch(
    history: &ComposeHistoryState,
    buffer: &ComposeBuffer,
    raw_buffer: &ComposeBuffer,
    app_handle: &AppHandle,
    batch_id: u64,
    keep_stage_count: usize,
) -> Result<(), String> {
    let mut hist = history.lock().unwrap();
    let index = hist
        .batches
        .iter()
        .position(|b| b.id == batch_id)
        .ok_or_else(|| "Batch not found".to_string())?;

    let (start, old_end, new_text, stages_len) = {
        let batch = &hist.batches[index];
        let kept = keep_stage_count.min(batch.stages.len());
        let new_text = if kept == 0 {
            batch.raw_text.clone()
        } else {
            batch.stages[kept - 1].text.clone()
        };
        (batch.start, batch.end, new_text, batch.stages.len())
    };
    let kept = keep_stage_count.min(stages_len);

    {
        let mut buf = buffer.lock().unwrap();
        if old_end > buf.len() || start > old_end {
            return Err("Compose buffer changed since this batch ran".to_string());
        }
        buf.replace_range(start..old_end, &new_text);
    }

    let new_end = start + new_text.len();
    let delta = new_end as isize - old_end as isize;

    {
        let batch = &mut hist.batches[index];
        batch.end = new_end;
        batch.stages.truncate(kept);
    }
    if delta != 0 {
        for later in hist.batches.iter_mut().skip(index + 1) {
            later.start = (later.start as isize + delta).max(0) as usize;
            later.end = (later.end as isize + delta).max(0) as usize;
        }
    }
    drop(hist);

    emit_state(app_handle, buffer, raw_buffer, false, None);
    Ok(())
}

/// Serializable snapshot of the batch history for the frontend's contribution panel.
pub fn history_snapshot(history: &ComposeHistoryState) -> Vec<BatchRecord> {
    history.lock().unwrap().batches.clone()
}

/// Batch history is just an offset-annotated view into the corrected buffer - once that
/// buffer is cleared/reset, every recorded `start`/`end` in it is stale and would
/// misbehave if a revert were attempted, so anything that clears the buffer needs to
/// clear this alongside it.
pub fn clear_history(history: &ComposeHistoryState) {
    let mut hist = history.lock().unwrap();
    hist.batches.clear();
}

pub fn reset_pending(pending: &ComposePendingState) {
    let mut state = pending.lock().unwrap();
    state.start = 0;
    state.count = 0;
    state.generation += 1;
    state.discard_before_generation = state.generation;
    state.full_document_generation = None;
}

/// A short, filesystem-safe slug derived from the first few words of `text` - not a
/// summary (no LLM call, so it's cheap enough to recompute live on every keystroke/
/// update for a "here's what this would be named" preview), just enough of the actual
/// opening words to be recognizable later. Falls back to "dictation" if there's nothing
/// usable (e.g. text starting with only punctuation/numbers).
pub fn filename_slug(text: &str) -> String {
    let words: Vec<String> = text
        .split_whitespace()
        .take(8)
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();

    if words.is_empty() {
        return "dictation".to_string();
    }

    let mut slug = words.join("-");
    if slug.len() > 50 {
        // Cut at the last hyphen before the limit rather than mid-word.
        let cut = slug[..50].rfind('-').unwrap_or(50);
        slug.truncate(cut);
    }
    slug
}

fn topic_filename_slug(text: &str) -> String {
    fn is_stopword(word: &str) -> bool {
        matches!(
            word,
            "about" | "actually" | "again" | "also" | "because" | "been" | "being"
                | "could" | "does" | "doing" | "from" | "have" | "here" | "into"
                | "just" | "like" | "really" | "should" | "something" | "that"
                | "their" | "there" | "these" | "thing" | "things" | "this" | "those"
                | "through" | "very" | "was" | "were" | "what" | "when" | "where"
                | "which" | "while" | "with" | "would" | "your" | "youre"
        )
    }

    let mut scored: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    for (index, raw_word) in text.split_whitespace().enumerate() {
        let word = raw_word
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if word.len() < 4 || is_stopword(&word) {
            continue;
        }
        let entry = scored.entry(word).or_insert((0, index));
        entry.0 += 1;
    }

    let mut words: Vec<(String, usize, usize)> = scored
        .into_iter()
        .map(|(word, (count, first))| (word, count, first))
        .collect();
    words.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.0.cmp(&right.0))
    });
    words.truncate(6);
    if words.len() < 3 {
        return filename_slug(text);
    }

    let mut slug = words
        .into_iter()
        .map(|(word, _, _)| word)
        .collect::<Vec<_>>()
        .join("-");
    if slug.len() > 64 {
        let cut = slug[..64].rfind('-').unwrap_or(64);
        slug.truncate(cut);
    }
    slug
}

/// Uses the active refinement backend to name an offloaded document from its overall
/// subject rather than its opening words. The final sanitizer is intentionally strict:
/// model output is advisory, and a safe deterministic slug remains the fallback.
pub fn semantic_filename_slug(
    text: &str,
    backend_cache: &ComposeBackendCache,
    config: &Config,
    app_handle: &AppHandle,
) -> String {
    const TITLE_PROMPT: &str = "Create a concise filename describing the main subject of the \
        entire document. Use three to seven specific words. Return only lowercase words \
        separated by hyphens, with no extension, quotation marks, explanation, or punctuation.";

    let generated = ensure_backend_loaded(backend_cache, config, app_handle).and_then(|_| {
        let backend = backend_cache
            .lock()
            .unwrap()
            .as_ref()
            .cloned()
            .ok_or("Compose backend not loaded")?;
        let result = backend
            .lock()
            .unwrap()
            .complete(Some(TITLE_PROMPT), text.trim(), 24);
        result
    });

    if let Ok(candidate) = generated {
        crate::log_info!("Compose: filename model proposed '{}'", candidate.trim());
        let words: Vec<String> = candidate
            .lines()
            .next()
            .unwrap_or_default()
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .take(7)
            .map(|word| word.to_ascii_lowercase())
            .collect();
        if (3..=7).contains(&words.len()) {
            let slug = words.join("-");
            let opening_words = normalized_words(text);
            let merely_repeats_opening = words.len() <= opening_words.len()
                && words
                    .iter()
                    .zip(opening_words.iter())
                    .all(|(candidate_word, opening_word)| candidate_word == opening_word);
            if slug.len() <= 64 && !merely_repeats_opening {
                return slug;
            }
            if merely_repeats_opening {
                crate::log_info!(
                    "Compose: filename proposal merely repeated the opening; using topic fallback"
                );
            }
        }
        crate::log_info!("Compose: generated offload title was unusable; using safe fallback");
    }

    topic_filename_slug(text)
}

fn emit_state(
    app_handle: &AppHandle,
    buffer: &ComposeBuffer,
    raw_buffer: &ComposeBuffer,
    correcting: bool,
    active_agent: Option<String>,
) {
    let text = buffer.lock().unwrap().clone();
    let raw_text = raw_buffer.lock().unwrap().clone();
    let _ = app_handle.emit(
        "compose-state-updated",
        ComposeState { raw_text, text, correcting, active_agent },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real raw/corrected pair from a live test session: the LLM correctly cleaned up
    // punctuation for most of it, but rewrote one clause's wording and dropped two
    // discourse words ("Alright,", "Yes,") - exactly the failure the fidelity check
    // exists to catch, since it changed no more character count than a legitimate
    // punctuation cleanup would.
    const RAW: &str = "It's gonna be a very interesting story correction working? So it's not actually formatting it properly. How do I test this so it looks back a few sentences and actually does smart correction? Alright, let's keep going.";
    const REWORDED: &str = "It's going to be a very interesting story. Correction working? So, it's not actually formatting it properly. How do I test this to see if it will correct previous sentences and work as intended? Let's keep going.";

    // A plausible *good* correction of the same input: punctuation/capitalization only,
    // no dropped or substituted words.
    const CLEANED: &str = "It's going to be a very interesting story. Correction working? So, it's not actually formatting it properly. How do I test this so it looks back a few sentences and actually does smart correction? Alright, let's keep going.";

    #[test]
    fn fidelity_rejects_reworded_clause_and_dropped_words() {
        let fidelity = word_fidelity(RAW, REWORDED);
        assert!(
            fidelity < 0.85,
            "expected reworded text to fail the fidelity gate, got {fidelity}"
        );
    }

    #[test]
    fn fidelity_accepts_pure_punctuation_cleanup() {
        let fidelity = word_fidelity(RAW, CLEANED);
        assert!(
            fidelity >= 0.85,
            "expected a punctuation-only cleanup to pass the fidelity gate, got {fidelity}"
        );
    }

    // Real raw/corrected pair from a live test session: the model correctly split a
    // run-on into separate sentences with terminal punctuation, but "Goodnight" (a
    // single word) became "Good night" (two words) in the process. Word-level
    // comparison alone saw that as a dropped word plus two added words on a 9-word
    // utterance and tanked fidelity below the accept threshold - a real false-positive
    // rejection of a genuinely good correction.
    const RUNON_RAW: &str = "Goodnight, get some rest, I love you, sweet dreams";
    const RUNON_SPLIT: &str = "Good night. Get some rest. I love you. Sweet dreams.";

    #[test]
    fn fidelity_accepts_word_split_from_sentence_correction() {
        let fidelity = word_fidelity(RUNON_RAW, RUNON_SPLIT);
        assert!(
            fidelity >= 0.85,
            "expected a word-split correction (Goodnight -> Good night) to still pass the fidelity gate, got {fidelity}"
        );
    }

    #[test]
    fn capitalizes_after_inserted_sentence_break() {
        let input = "It's going to be a very interesting story. correction working?";
        let expected = "It's going to be a very interesting story. Correction working?";
        assert_eq!(capitalize_sentence_starts(input), expected);
    }

    #[test]
    fn capitalizes_first_letter_of_whole_text() {
        assert_eq!(capitalize_sentence_starts("hello. world."), "Hello. World.");
    }

    #[test]
    fn repeated_phrase_guard_rejects_inserted_duplicate_tail() {
        let raw = "The recording should stop and choose a useful file name.";
        let repeated =
            "Choose a useful file name. The recording should stop and choose a useful file name.";
        assert!(introduces_repeated_phrase(raw, repeated));
        assert!(!introduces_repeated_phrase(raw, raw));
    }

    #[test]
    fn repeated_phrase_guard_rejects_short_streaming_duplicates() {
        let raw = "Hello, I'm testing this application. It appears to be working.";
        let repeated = "Hello, I'm testing this application. I'm testing this application to see whether it is working.";
        assert!(introduces_repeated_phrase(raw, repeated));
    }

    #[test]
    fn prompt_markup_guard_is_independent_of_fidelity() {
        assert!(leaks_prompt_markup(
            "The text you must correct and return will be given between <transcript> tags."
        ));
        assert!(!leaks_prompt_markup("This is an ordinary refined sentence."));
    }

    #[test]
    fn editorial_wrapper_is_removed_without_deleting_dictated_words() {
        assert_eq!(
            strip_editorial_wrapper(
                "Um, come up with a useful file name.",
                "Here is the corrected text:\n\nUm, come up with a useful file name."
            ),
            "Um, come up with a useful file name."
        );
        assert_eq!(
            strip_editorial_wrapper(
                "Here is the corrected text: that I dictated.",
                "Here is the corrected text: that I dictated."
            ),
            "Here is the corrected text: that I dictated."
        );
        assert_eq!(
            strip_editorial_wrapper(
                "The corrected sentence.",
                "<transcript>\nThe corrected sentence.\n</transcript>"
            ),
            "The corrected sentence."
        );
    }

    #[test]
    fn immediate_cleanup_collapses_adjacent_word_duplicates_only() {
        assert_eq!(
            collapse_adjacent_duplicate_words("I don't know if if that works works."),
            "I don't know if that works."
        );
        assert_eq!(
            collapse_adjacent_duplicate_words("No. No. Keep both."),
            "No. No. Keep both."
        );
    }

    #[test]
    fn refinement_cannot_create_a_tiny_sentence_fragment() {
        let source = "The generated name should describe the subject instead of copying these opening words.";
        let fragmented =
            "The generated name should describe the subject instead of copying these. Opening words.";
        assert!(introduces_short_sentence_fragment(source, fragmented));
        assert!(!introduces_short_sentence_fragment(source, source));
        assert!(!introduces_short_sentence_fragment(
            "It works. Try again.",
            "It works. Try again."
        ));
    }

    #[test]
    fn rolling_confirmation_keeps_incomplete_tail_mutable() {
        let text = "This sentence is complete. Places where";
        let end = confirmed_tail_end(text).expect("complete prefix");
        assert_eq!(&text[..end], "This sentence is complete. ");
        assert_eq!(&text[end..], "Places where");
    }

    #[test]
    fn rolling_confirmation_accepts_completed_tail_later() {
        let text = "This sentence is complete. Places where errors exist.";
        assert_eq!(confirmed_tail_end(text), Some(text.len()));
    }

    #[test]
    fn replacement_preserves_neighbor_spacing() {
        assert_eq!(
            preserve_boundary_whitespace("  raw sentence. ", "Refined sentence."),
            "  Refined sentence. "
        );
    }

    #[test]
    fn ordered_fidelity_rejects_clause_shuffle() {
        let source = "First we capture speech. Then we refine the transcript.";
        let shuffled = "Then we refine the transcript. First we capture speech.";
        assert!(ordered_word_fidelity(source, shuffled) < 0.72);
    }

    #[test]
    fn read_only_context_copy_is_rejected() {
        let context = "Earlier text contains a distinctive sequence about watching the logs.";
        let input = "This is the new sentence to edit.";
        let contaminated =
            "This is the new sentence to edit. Earlier text contains a distinctive sequence.";
        assert!(copies_read_only_context(input, context, contaminated));
        assert!(!copies_read_only_context(input, context, input));
    }

    #[test]
    fn source_closing_request_cannot_be_moved_to_opening() {
        let source = "This document begins with background and continues through several \
            useful details before reaching its final request. So if you could do that, \
            that would be really cool.";
        let moved = "So if you could do that, that would be really cool. This document \
            begins with background and continues through several useful details.";
        assert!(moves_late_source_phrase_to_front(source, moved));
        assert!(!moves_late_source_phrase_to_front(source, source));
    }

    #[test]
    fn short_late_clause_cannot_be_moved_to_opening() {
        let source = "Alright, so here we go again. This document discusses streaming \
            refinement behavior where it was taking and copying.";
        let moved = "Taking and copying. Alright, so here we go again. This document \
            discusses streaming refinement behavior.";
        assert!(moves_late_source_phrase_to_front(source, moved));
    }

    #[test]
    fn complete_source_sentence_cannot_be_silently_dropped() {
        let source = "The first sentence provides useful context. Phrase should never jump \
            to the beginning.";
        let missing = "The first sentence provides useful context.";
        assert!(drops_source_sentence(source, missing));
        assert!(!drops_source_sentence(source, source));
    }

    #[test]
    fn topic_filename_uses_the_whole_document() {
        let text = "An opening sentence says hello. Later the document repeatedly discusses \
            streaming refinement, transcript refinement, filename generation, and streaming \
            safety. The final request is better filename generation.";
        let slug = topic_filename_slug(text);
        assert!(slug.contains("streaming"));
        assert!(slug.contains("refinement"));
        assert!(slug.contains("filename"));
        assert!(!slug.starts_with("opening-sentence"));
    }

    #[test]
    fn long_unformatted_text_receives_paragraph_breaks() {
        let sentence = "This sentence contains enough ordinary words for a realistic test. ";
        let input = sentence.repeat(12);
        let formatted = add_paragraph_breaks_to_long_block(input.trim());
        assert!(formatted.contains("\n\n"));
        assert_eq!(normalized_words(&formatted), normalized_words(input.trim()));
    }
}
