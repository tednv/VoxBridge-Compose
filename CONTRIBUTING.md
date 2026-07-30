# Contributing

VoxBridge Compose is in an early extraction phase. Before submitting a change:

1. Keep local processing and graceful fallback behavior intact.
2. Add or update validation for behavioral changes.
3. Preserve applicable copyright, license, and third-party notices.
4. Do not commit credentials, private data, generated build output, or local
   working-session records.
5. Keep the product boundary clear: VoxBridge Compose owns recording, documents,
   agents, history, and user workflows; the separate VoxBridge runtime owns
   backend adaptation, capability detection, model lifecycle, and normalized
   speech/refinement results.
6. Do not expand VoxBridge Compose or VoxBridge into a generic cloud-provider
   gateway. Reuse established inference runtimes and add a backend only when it
   directly supports the local transcription-and-refinement pipeline.

By contributing, you agree that your contribution is licensed under the GNU
Affero General Public License version 3.
