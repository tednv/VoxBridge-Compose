# VoxBridge Compose architecture

## Product boundary

VoxBridge Compose is a local-first recording, transcription, and document-refinement
workspace for Windows and Linux. The application and the VoxBridge runtime are
separate projects with separate responsibilities and licenses:

- **VoxBridge Compose** is the AGPL-licensed desktop product. It owns microphone
  capture, continuous-recording policy, utterance ordering, documents, agents,
  history, privacy, offloading, status presentation, global shortcuts, and desktop
  integration.
- **VoxBridge** is the separately maintained MIT-licensed runtime adapter. It hides
  backend-specific native libraries, optional workers, model formats, hardware
  probes, model lifecycle, warmup, cancellation, caching, switching, fallback, and
  result normalization behind a stable Rust-facing API.

VoxBridge is intentionally not a universal AI gateway. It does not aim to duplicate
LiteLLM, LocalAI, Speaches, or other general provider/proxy systems. It integrates
only the established inference backends required by the Compose pipeline.

## Pipeline

```text
Microphone and continuous recording
                  |
                  v
       Ordered utterance boundary
                  |
                  v
        VoxBridge runtime API
          |               |
          |               +----------------------------+
          v                                            v
Speech-recognition adapter                    Refinement adapter
  - whisper.cpp CPU/Vulkan                      - embedded llama.cpp
  - Faster Whisper/CTranslate2                  - local/network Ollama
    optional CUDA/processor
          |                                            |
          v                                            v
       Raw transcript ----------------> Compose agent pipeline
          |                                            |
          +------------------> Raw and refined document
```

Audio remains in the speech-recognition path. Ollama receives text only when the
user selects it for refinement. A network Ollama connection is user-configured and
must be protected appropriately for the local network.

## Speech recognition

### whisper.cpp

The compact default path uses whisper.cpp through VoxBridge's runtime-selected
engine variants:

- a conservative processor baseline
- an AVX2/FMA processor variant where supported
- a Vulkan graphics variant for broad NVIDIA and AMD coverage

VoxBridge selects and loads the appropriate self-contained native library without
exposing dynamic-library details to the application.

### Faster Whisper and CTranslate2

The default recognition path uses the MIT-licensed Faster Whisper pipeline and
CTranslate2 inference engine through a persistent worker owned by VoxBridge.
Compose does not directly manage Python or CTranslate2.

This backend is optional because its runtime, CUDA libraries, and model artifacts
are substantially larger than the compact default. It supports:

- CTranslate2 CUDA/FP16 on compatible NVIDIA systems
- optimized CTranslate2 processor inference as fallback
- separate CTranslate2 model directories rather than whisper.cpp GGML files
- model preload and actual-inference warmup before reporting Ready
- a normalized text result compatible with the existing Compose pipeline

Portable packaging, download progress, cancellation, and cross-platform runtime
installation remain active implementation work.

## Text refinement

VoxBridge presents two refinement adapters:

- **Embedded:** an in-process GGUF model through llama.cpp engine variants.
- **Ollama:** a text-only request to a user-selected Ollama server on the same
  computer or another machine on the local network.

The runtime owns backend connection/model mechanics. Compose owns agent prompts,
ordering, fidelity checks, retries, acceptance, document reconciliation, and user
controls.

## Streaming document model

Whisper-family recognition operates on ordered audio utterances; it is not used as
an ever-growing document editor. Compose maintains the live document with a
confirmed-prefix/revisable-tail design:

1. Completed text remains stable.
2. A bounded recent tail can be refined.
3. New speech is coalesced for the next serialized pass.
4. Every result is tied to the exact source revision that produced it.
5. Stale results from a superseded backend or document generation are discarded.
6. Stop-time reconciliation considers the complete document while preserving
   established paragraphs and ordering.

Safeguards reject duplicated or missing sentences, reordered content, context
leakage, malformed joins, paragraph collapse, and unsupported rewrites. The
research and reference systems that informed this independently implemented design
are credited in [NOTICE.md](../NOTICE.md).

## Backend switching

Recording capture must remain independent from model replacement. A backend or
model change:

1. claims a new generation;
2. prepares and warms the requested backend;
3. prevents older in-flight work from updating current readiness or text;
4. activates at a safe utterance boundary; and
5. preserves the raw transcript and document already accepted.

Cached backends may be reused when their model, device request, and runtime remain
compatible. A failed graphics warmup may fall back to processor inference, but the
actual device must be reported accurately.

## Status and privacy

The Status workspace combines application state with runtime capability and model
lifecycle information. Estimates must be labeled as estimates; remote Ollama memory
must not be presented as locally measured.

Private mode suppresses new persistent history, recording files, and diagnostic
session logs. Bug reports are prepared for review, redact local paths where
possible, and are never submitted automatically.

## Non-goals

The current architecture does not include:

- a broad catalog of cloud AI providers
- a generic OpenAI-compatible proxy
- custom speech or language-model inference engines
- text-to-speech, image generation, vision, or embeddings
- backend-specific model lifecycle duplicated in the Compose frontend

Those capabilities should be supplied by established projects if a concrete future
product requirement justifies an adapter.
