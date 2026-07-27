# VoxBridge Compose

GPU-first local transcription and composition infrastructure.

VoxBridge Compose is being built around reliable hardware detection, accelerated
local transcription, automatic runtime selection, safe CPU fallback, and an
optional local composition pass.

The project is under active extraction and is not yet ready for general use.

## Principles

- Local-first processing
- GPU acceleration as the normal operating mode
- Automatic selection of compatible runtime components
- Predictable fallback when acceleration is unavailable
- Clear diagnostics instead of silent failure
- No account or hosted-service requirement

## Project provenance

This project originated from FOSS Voquill and may retain portions of its
AGPLv3-licensed code. It has since been independently maintained and
substantially re-engineered, particularly its transcription runtime, hardware
acceleration, fallback behavior, diagnostics, and packaging.

Original Voquill code remains copyright its respective authors. Subsequent
modifications and original components remain copyright their respective
contributors. This project is not affiliated with or endorsed by the original
Voquill maintainers.

See [NOTICE.md](NOTICE.md) for attribution details.

## License

VoxBridge Compose is licensed under the GNU Affero General Public License
version 3. See [LICENSE](LICENSE).
