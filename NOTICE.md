# Notices and attribution

## FOSS Voquill

VoxBridge Compose is an independently maintained derivative of
[FOSS Voquill](https://github.com/jackbrumley/voquill), created by Jack Brumley
with contributions from the Voquill community. The original project supplied the
AGPL-licensed application foundation, including substantial desktop,
cross-platform, audio, configuration, and user-interface work.

We are grateful to the original author and contributors for making that work
available as free software. Visit [voquill.org](https://voquill.org/) to learn
about the original project, and consider
[supporting its development](https://voquill.org/donate.html).

Original Voquill contributions remain copyright their respective authors and
contributors and remain subject to their applicable license notices. VoxBridge
Compose is not affiliated with or endorsed by the original maintainers.

## VoxBridge and inference projects

The application uses the separately maintained, MIT-licensed
[VoxBridge](https://github.com/tednv/VoxBridge) runtime. VoxBridge incorporates
the MIT-licensed [whisper.cpp](https://github.com/ggerganov/whisper.cpp) and
[llama.cpp](https://github.com/ggml-org/llama.cpp) projects as submodules.

The default embedded refinement model is the Apache-2.0-licensed
[Qwen3 4B Instruct 2507 model](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507),
distributed in GGUF form by
[Unsloth](https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF).
Users are responsible for reviewing the terms supplied with models they choose to
download or configure.

Additional acknowledgements and license references are listed in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). License files shipped inside
dependencies and submodules remain authoritative for those components.

## Incremental streaming research

VoxBridge Compose's confirmed-prefix, revisable-tail, and bounded-revision design
was informed by the following research and reference implementations:

- Ethan Selfridge, Iker Arizmendi, Peter Heeman, and Jason Williams,
  [*Stability and Accuracy in Incremental Speech Recognition*](https://aclanthology.org/W11-2014/)
  (SIGDIAL 2011), for treating stability and accuracy as explicit properties of
  incremental hypotheses.
- Ian McGraw and Alexander Gruenstein,
  [*Estimating Word-Stability During Incremental Speech Recognition*](https://research.google/pubs/estimating-word-stability-during-incremental-speech-recognition/)
  (Interspeech 2012), for the longest-stable-prefix framing and its
  stability-versus-latency tradeoff.
- Dominik Macháček, Raj Dabre, and Ondřej Bojar,
  [*Turning Whisper into a Real-Time Transcription System*](https://aclanthology.org/2023.ijcnlp-demo.3/)
  (IJCNLP-AACL 2023), and its MIT-licensed
  [Whisper-Streaming](https://github.com/ufal/whisper_streaming) reference
  implementation, for LocalAgreement and adaptive confirmation.
- Junkun Chen, Jian Xue, Peidong Wang, Jing Pan, and Jinyu Li,
  [*Improving Stability in Simultaneous Speech Translation: A Revision-Controllable Decoding Approach*](https://arxiv.org/abs/2310.04399)
  (2023), for explicitly bounding the portion of a live result that may be
  revised.
- Dominik Macháček and Peter Polák,
  [*Simultaneous Translation with Offline Speech and LLM Models in CUNI Submission to IWSLT 2025*](https://aclanthology.org/2025.iwslt-1.41/)
  and the MIT-licensed
  [SimulStreaming](https://github.com/ufal/SimulStreaming) project, for later
  practical work on incremental Whisper transcription and context.

VoxBridge Compose does not incorporate source code from these projects. Its
streaming refinement implementation is independent; these citations credit the
research concepts and reference systems that informed its architecture.

## Subsequent work

New VoxBridge Compose components and subsequent modifications remain copyright
their respective contributors and are distributed under the GNU Affero General
Public License version 3.
