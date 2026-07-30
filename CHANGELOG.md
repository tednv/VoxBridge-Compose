# Changelog

## 0.3.0

### Live refinement

- Reworked streaming refinement around a confirmed prefix and revisable recent
  tail, with serialized model work and exact-span replacement.
- Coalesces speech received during an active pass and rejects stale work rather
  than duplicating, moving, or overwriting newer dictation.
- Waits for outstanding recognition when recording stops, then performs one
  authoritative full-document reconciliation.
- Adds safeguards for repeated phrases, prompt or context leakage, dropped and
  reordered sentences, paragraph collapse, malformed word joins, moved closing
  clauses, editorial wrappers, and newly introduced tiny sentence fragments.
- Preserves paragraph structure and applies deterministic cleanup to newly
  recognized text without capitalizing arbitrary streaming chunk boundaries.
- Rebuilds the current document once after a fidelity change instead of replaying
  overlapping historical batches.

### Models and providers

- Makes Faster Whisper/CTranslate2 the default speech-recognition path, with
  CUDA FP16 acceleration, a CPU INT8 fallback, and whisper.cpp/Vulkan retained
  as the broad-hardware compatibility option.
- Adds managed Faster Whisper model selection and preparation, including
  complete-download validation and accurate download, load, and warmup status.
- Makes Qwen3 4B Instruct 2507 Q4_K_M the recommended embedded refinement model.
- Adds a managed embedded-model selector with a lighter Qwen2.5 1.5B option and
  an advanced custom-GGUF picker.
- Shows installation state, download size, intended hardware tier, and estimated
  combined graphics-memory requirements before downloading a managed model.
- Migrates only the former default model path; user-selected custom model paths
  remain untouched and existing model files are not deleted.
- Makes embedded/Ollama switching generation-aware so slow initialization cannot
  restore an obsolete provider or apply its stale result.
- Downloads missing managed models, preloads and warms the selected provider, and
  reconciles an existing document once the replacement provider is ready.

### Status and workflow

- Adds a compact global Start/Stop control, a vertical live microphone meter,
  backend/model details on status hover, and a confirmed workspace-clear action.
- Places refinement history and copy actions beside Apply, consolidates the
  offload controls, and gives both transcript panes the primary working area.
- Restores system-memory reporting, adds live processor activity, and reports
  graphics-memory totals and available allocation where the operating system
  exposes them.
- Separates speech-recognition and text-refinement memory estimates, reads local
  Ollama's reported model allocation when available, and does not pretend to
  measure a network Ollama server.
- Adds topic-based filename suggestions using the whole document, with a local
  keyword fallback when model output is unsuitable.
- Improves progress reporting for download, initialization, preload, recognition,
  refinement, provider switching, and final reconciliation.
- Guards provider and model switching with generation checks so recording can
  continue without stale results restoring an obsolete backend.

### Credits

- Documents the Qwen3 and Unsloth model distribution used by the recommended
  embedded option.
- Credits the LocalAgreement research, Whisper-Streaming, and SimulStreaming that
  informed the independently implemented streaming architecture, along with the
  earlier incremental word-stability work and bounded-revision research.

## 0.1.1

- Corrected Linux package identity.

## 0.1.0

- Initial independent VoxBridge Compose release.
