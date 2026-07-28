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
pub type ComposeBackendCache = Arc<Mutex<Option<voxbridge::LlmBackend>>>;
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
#[derive(Default)]
pub struct PendingBatch {
    /// Byte offset into the corrected buffer where this batch's uncorrected text begins.
    start: usize,
    /// How many utterances have been folded into this batch so far.
    count: usize,
    /// Bumped on every utterance; lets a scheduled debounce fire only if nothing newer
    /// has arrived to supersede it, without needing to explicitly cancel a tokio task.
    generation: u64,
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
        return;
    }

    let backend_cache = state.compose_backend.clone();
    {
        let guard = backend_cache.lock().unwrap();
        if guard.is_some() {
            return; // already loaded
        }
    }

    let compose_loading = state.compose_loading.clone();
    let compose_preload_error = state.compose_preload_error.clone();
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
            if prepared_config.compose_model_path.trim().is_empty() || !configured_path.exists() {
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
                    crate::app::status::emit_status_to_frontend("Warming refinement model").await;
                    match tokio::task::spawn_blocking(move || {
                        ensure_backend_loaded(
                            &backend_cache,
                            &prepared_config,
                            &app_handle_for_blocking,
                        )?;
                        let guard = backend_cache.lock().unwrap();
                        let backend = guard.as_ref().ok_or("Compose backend not loaded")?;
                        // Throwaway completion to force the model resident and any one-time
                        // GPU shader compilation to happen now, not on the user's first real use.
                        backend.complete(Some(WARMUP_PROMPT), "warm up.", 8)
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

        let error = match result {
            Ok(_) => {
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
            serde_json::json!({ "loading": false, "error": error }),
        );
        crate::app::status::emit_status_to_frontend("Ready").await;
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

    *cache.lock().unwrap() = Some(backend);
    Ok(())
}

/// Drops any cached backend so the next call reloads from current config - used when
/// Compose-relevant settings change (backend, model, GPU toggle) while the app is running.
pub fn invalidate_backend(cache: &ComposeBackendCache) {
    *cache.lock().unwrap() = None;
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
    {
        let mut raw_buf = raw_buffer.lock().unwrap();
        if !raw_buf.is_empty() && !raw_buf.ends_with(' ') {
            raw_buf.push(' ');
        }
        raw_buf.push_str(raw_text.trim());
    }

    let (context_sentences, pause_only, debounce_ms) = {
        let cfg = config.lock().unwrap();
        (
            cfg.compose_context_sentences,
            cfg.compose_pause_only,
            latency_debounce_ms(&cfg.compose_edit_latency),
        )
    };
    let (batch_start, batch_count, my_generation) = {
        let mut buf = buffer.lock().unwrap();
        if !buf.is_empty() && !buf.ends_with(' ') {
            buf.push(' ');
        }
        let mut p = pending.lock().unwrap();
        if p.count == 0 {
            // Include editable overlap from the already-composed text. Later speech
            // often explicitly corrects an earlier recognition error ("I said test,
            // not dust"); read-only reference context cannot repair that earlier word.
            // Everything revisits the full current document, while finite settings
            // revisit the requested number of recent sentence boundaries.
            p.start = editable_context_start(&buf, context_sentences);
        }
        buf.push_str(raw_text.trim());
        p.count += 1;
        p.generation += 1;
        (p.start, p.count, p.generation)
    };
    emit_state(&app_handle, &buffer, &raw_buffer, true, None);

    let fire_now = !pause_only
        && context_sentences != CONTEXT_UNLIMITED
        && batch_count >= context_sentences as usize;

    if fire_now {
        {
            let mut p = pending.lock().unwrap();
            p.count = 0;
        }
        spawn_correction(batch_start, buffer, raw_buffer, pending, history, backend_cache, config, app_handle);
        return;
    }

    // Not enough utterances yet to hit the configured batch size (or pause-only mode is
    // on) - schedule a debounced fire so a trailing partial batch still gets corrected
    // once the user actually pauses.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(debounce_ms)).await;

        let should_fire = {
            let mut p = pending.lock().unwrap();
            if p.generation == my_generation && p.count > 0 {
                p.count = 0;
                true
            } else {
                false
            }
        };
        if should_fire {
            spawn_correction(batch_start, buffer, raw_buffer, pending, history, backend_cache, config, app_handle);
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

/// How much already-corrected preceding text to include as read-only reference context
/// for a batch - enough to resolve a context-dependent correction (a callback to an
/// earlier reference, a running project/person name) without letting the prompt grow
/// unbounded over a long dictation session, or crowding out the embedded model's small
/// context window.
const MAX_CONTEXT_CHARS: usize = 1500;

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
            let guard = backend_cache.lock().unwrap();
            let backend = guard.as_ref().ok_or("Compose backend not loaded")?;
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
            backend.complete(Some(prompt.as_str()), &user_prompt, max_tokens)
        });
        COMPOSE_AGENT_RUNS.fetch_add(1, Ordering::Relaxed);
        COMPOSE_MS_TOTAL.fetch_add(call_started.elapsed().as_millis() as u64, Ordering::Relaxed);

        match result {
            Ok(reply) => {
                let reply_trimmed = reply.trim().to_string();
                let raw_trimmed = input_for_agent.trim();
                let ballooned =
                    reply_trimmed.len() as f64 > (raw_trimmed.len() as f64 * 1.6 + 60.0);
                let fidelity = word_fidelity(raw_trimmed, &reply_trimmed);
                let reworded = fidelity < agent.min_fidelity;
                if ballooned || reworded {
                    COMPOSE_REJECTED.fetch_add(1, Ordering::Relaxed);
                    crate::log_info!(
                        "Compose: agent '{}' rejected (word fidelity {:.2})\n  raw:   {}\n  reply: {}",
                        agent.name,
                        fidelity,
                        raw_trimmed,
                        reply_trimmed
                    );
                    stages.push(AgentStage {
                        agent_id: agent.id.clone(),
                        agent_name: agent.name.clone(),
                        text: reply_trimmed,
                        accepted: false,
                        note: Some(format!(
                            "Rejected: drifted too far from the input (word fidelity {:.2})",
                            fidelity
                        )),
                        fidelity,
                    });
                } else {
                    COMPOSE_ACCEPTED.fetch_add(1, Ordering::Relaxed);
                    let final_text = capitalize_sentence_starts(&reply_trimmed);
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
fn spawn_correction(
    batch_start: usize,
    buffer: ComposeBuffer,
    raw_buffer: ComposeBuffer,
    pending: ComposePendingState,
    history: ComposeHistoryState,
    backend_cache: ComposeBackendCache,
    config: Arc<Mutex<Config>>,
    app_handle: AppHandle,
) {
    let (batch_raw_text, context_text) = {
        let buf = buffer.lock().unwrap();
        if batch_start > buf.len() {
            return;
        }
        let batch_raw_text = buf[batch_start..].to_string();
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
        return;
    }

    tauri::async_runtime::spawn(async move {
        let cfg = { config.lock().unwrap().clone() };
        let batch_raw_for_blocking = batch_raw_text.clone();
        let context_for_blocking = context_text.clone();
        let app_handle_for_blocking = app_handle.clone();
        let buffer_for_blocking = buffer.clone();
        let raw_buffer_for_blocking = raw_buffer.clone();
        let agents = cfg.compose_agents.clone();

        let (final_text, stages) = tokio::task::spawn_blocking(move || {
            run_agent_chain(
                &batch_raw_for_blocking,
                &context_for_blocking,
                &agents,
                &backend_cache,
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

        {
            let mut buf = buffer.lock().unwrap();
            // Only patch if this batch's span is still exactly what we sent (no newer
            // batch already started past it while we were correcting) - otherwise leave
            // the raw text alone rather than corrupt a later edit.
            if batch_start <= buf.len() {
                let after_is_ours = buf[batch_start..].trim() == batch_raw_text.trim();
                if after_is_ours {
                    buf.replace_range(batch_start.., &final_text);
                }
            }
        }

        {
            let mut hist = history.lock().unwrap();
            let id = hist.next_id;
            hist.next_id += 1;
            hist.batches.push(BatchRecord {
                id,
                start: batch_start,
                end: batch_start + final_text.len(),
                raw_text: batch_raw_text.trim().to_string(),
                stages,
            });
            if hist.batches.len() > MAX_HISTORY_BATCHES {
                let overflow = hist.batches.len() - MAX_HISTORY_BATCHES;
                hist.batches.drain(0..overflow);
            }
        }

        let still_pending = pending.lock().unwrap().count > 0;
        emit_state(&app_handle, &buffer, &raw_buffer, still_pending, None);
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

/// What a batch's text looked like right before the stage at `position` ran - walks the
/// real (not hypothetical) accept/reject decisions up to that point. Used as the actual
/// input handed to a re-run starting partway through the chain.
fn effective_output_before(batch: &BatchRecord, position: usize) -> String {
    let mut current = batch.raw_text.clone();
    for stage in batch.stages.iter().take(position) {
        if stage.accepted {
            current = stage.text.clone();
        }
    }
    current
}

/// Live-tunes the fidelity slider for real: re-runs `agent_id` (with `override_threshold`
/// in place of its configured value) and every agent after it in priority order, for
/// every batch currently in history, against each batch's actual real LLM calls - not
/// just a client-side re-threshold of an already-stored score. This is the accurate but
/// costly path (a real inference call per batch per downstream agent), used because
/// re-thresholding alone can't show what a downstream agent would actually have produced
/// if an upstream agent's accept/reject decision had come out differently.
///
/// Agents *before* `agent_id` are left untouched - their real accepted output is reused
/// as the starting text for this re-run, exactly as it was when the chain first ran.
pub fn recompute_from_agent(
    history: &ComposeHistoryState,
    buffer: &ComposeBuffer,
    raw_buffer: &ComposeBuffer,
    backend_cache: &ComposeBackendCache,
    config: &Config,
    app_handle: &AppHandle,
    agent_id: &str,
    override_threshold: f64,
) {
    let batch_ids: Vec<u64> = { history.lock().unwrap().batches.iter().map(|b| b.id).collect() };

    for batch_id in batch_ids {
        let prepared = {
            let hist = history.lock().unwrap();
            let batch = match hist.batches.iter().find(|b| b.id == batch_id) {
                Some(b) => b,
                None => continue,
            };

            let mut ordered: Vec<crate::config::ComposeAgent> = config
                .compose_agents
                .iter()
                .filter(|a| a.enabled)
                .cloned()
                .collect();
            ordered.sort_by_key(|a| a.priority);
            let position = match ordered.iter().position(|a| a.id == agent_id) {
                Some(p) => p,
                None => {
                    crate::log_info!(
                        "Compose: recompute skipped batch {} - agent '{}' not found among {} enabled agent(s)",
                        batch_id,
                        agent_id,
                        ordered.len()
                    );
                    continue;
                }
            };

            let input_text = effective_output_before(batch, position);
            let mut sub_agents = ordered[position..].to_vec();
            if let Some(first) = sub_agents.first_mut() {
                first.min_fidelity = override_threshold;
            }

            let start = batch.start;
            let context_text = {
                let buf = buffer.lock().unwrap();
                let bounded_start = start.min(buf.len());
                let context_start = if bounded_start > MAX_CONTEXT_CHARS {
                    let mut i = bounded_start - MAX_CONTEXT_CHARS;
                    while i > 0 && !buf.is_char_boundary(i) {
                        i += 1;
                    }
                    i
                } else {
                    0
                };
                buf[context_start..bounded_start].trim().to_string()
            };

            Some((start, input_text, sub_agents, context_text, position))
        };

        let Some((start, input_text, sub_agents, context_text, position)) = prepared else {
            continue;
        };

        let (final_text, new_stages) = run_agent_chain(
            &input_text,
            &context_text,
            &sub_agents,
            backend_cache,
            config,
            app_handle,
            buffer,
            raw_buffer,
        );

        let mut hist = history.lock().unwrap();
        let Some(index) = hist.batches.iter().position(|b| b.id == batch_id) else {
            continue;
        };
        let old_end = hist.batches[index].end;
        if old_end > buffer.lock().unwrap().len() || start > old_end {
            // Buffer has changed shape since we snapshotted (e.g. a live utterance
            // landed mid-recompute) - skip this batch rather than corrupt it.
            continue;
        }
        {
            let mut buf = buffer.lock().unwrap();
            buf.replace_range(start..old_end, &final_text);
        }
        let new_end = start + final_text.len();
        let delta = new_end as isize - old_end as isize;
        {
            let batch = &mut hist.batches[index];
            batch.stages.truncate(position);
            batch.stages.extend(new_stages);
            batch.end = new_end;
        }
        if delta != 0 {
            for later in hist.batches.iter_mut().skip(index + 1) {
                later.start = (later.start as isize + delta).max(0) as usize;
                later.end = (later.end as isize + delta).max(0) as usize;
            }
        }
        drop(hist);
    }

    emit_state(app_handle, buffer, raw_buffer, false, None);
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
}
