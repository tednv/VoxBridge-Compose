# Third-party notices

VoxBridge Compose depends on open-source software. This overview is provided for
convenience and does not replace license files distributed with dependencies and
source submodules.

| Project | Role | License reference |
| --- | --- | --- |
| [FOSS Voquill](https://github.com/jackbrumley/voquill) | Original application foundation | AGPL-3.0 |
| [VoxBridge](https://github.com/tednv/VoxBridge) | Speech-recognition and embedded-refinement runtime | MIT; see `voxbridge/LICENSE` |
| [whisper.cpp](https://github.com/ggerganov/whisper.cpp) | Native speech inference | MIT; see `voxbridge/native/whisper.cpp/LICENSE` |
| [llama.cpp](https://github.com/ggml-org/llama.cpp) | Native language-model inference | MIT; see `voxbridge/native/llama.cpp/LICENSE` |
| [Tauri](https://tauri.app/) | Desktop application framework | Apache-2.0 OR MIT |
| [Preact](https://preactjs.com/) | User-interface library | MIT |
| [Tabler Icons](https://tabler.io/icons) | Interface icons | MIT |
| [Qwen3 4B Instruct 2507](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507) ([GGUF](https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF)) | Optional downloaded refinement model | Apache-2.0 |

The application can download Qwen3 model weights or connect to Ollama. Those
models and services are not relicensed by VoxBridge Compose; review their own
terms before use or redistribution. Model weights are downloaded on demand and
are not included in the application source repository.
