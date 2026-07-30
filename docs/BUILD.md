# VoxBridge Compose Build Guide

VoxBridge Compose uses npm scripts with the Tauri CLI to build across platforms.

## Quick Start

The main build command will check for dependencies, build the frontend, and then build the Tauri application.

```bash
# Start desktop development
npm run tauri:dev

# Standard Tauri bundle build
npm run tauri:build
```

### When to use each command:
- **npm run tauri:build**: Use this for the final app you intend to share or use daily. It produces optimized, small executables and bundled installers.
- **npm run tauri:dev**: Use this for active development. It provides hot-reloading for both the frontend and backend.

## Requirements

- **Rust** with Cargo
- **Node.js** and **npm** (for frontend dependencies)

### Platform-Specific Requirements

#### Linux (Ubuntu/Debian)
The build script checks for these packages:
- `libpulse-dev`
- `libgtk-layer-shell-dev`
- `cmake`
- `pkg-config`
- `libclang-dev`
- `build-essential`

Additional Tauri requirements:
- `libwebkit2gtk-4.1-dev`
- `libgtk-3-dev`
- `libayatana-appindicator3-dev`
- `librsvg2-dev`

#### Windows

`npm run deps:check` only verifies Clang/LLVM, CMake, Rust, and Node.js are present — it does **not** check two things a from-source build also needs, because whisper.cpp's Vulkan backend and the MSVC linker aren't things a simple `where.exe` lookup can validate cleanly. Install these yourself before building:

- **Visual Studio Build Tools with the "Desktop development with C++" workload.** A bare Visual Studio/Build Tools install (or the VS Code C/C++ extension alone) does *not* include this — check for `VC\Tools\MSVC\<version>\bin\Hostx64\x64\cl.exe` under your VS install path. Without it there's no MSVC linker (`link.exe`), and `cargo build` fails at the link step.
  ```
  winget install -e --id Microsoft.VisualStudio.2022.BuildTools --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  ```
- **Vulkan SDK.** `Cargo.toml` builds `whisper-rs` with the `vulkan` feature (used for GPU/"Turbo Mode" transcription), and `whisper-rs-sys`'s build script hard-fails with `Please install Vulkan SDK and ensure that VULKAN_SDK env variable is set` if it's missing. The official SDK is published by LunarG (Khronos' designated SDK maintainer) at `sdk.lunarg.com`:
  ```
  winget install -e --id KhronosGroup.VulkanSDK
  ```
  The installer sets the machine-level `VULKAN_SDK` env var, but a shell opened *before* installing won't see it — open a fresh shell (or re-derive it from the registry, see below).
- **Ninja**, to avoid a known MSBuild bug (see Troubleshooting below):
  ```
  winget install -e --id Ninja-build.Ninja
  ```

Two more things that aren't obvious from a stock PowerShell/terminal session:

1. **LLVM installed via winget does not add itself to PATH.** `clang.exe` ends up at `C:\Program Files\LLVM\bin\clang.exe` but isn't runnable as just `clang` until you add that directory to `PATH` yourself.
2. **Build from an MSVC developer environment**, not a plain shell — run `vcvarsall.bat x64` first (or use a "Developer PowerShell/Command Prompt for VS" shortcut). Without it, `clang`/`bindgen` can't find the MSVC/UCRT standard headers (`stdio.h`, `vcruntime.h`, etc.) when generating whisper.cpp's FFI bindings. It degrades gracefully (falls back to a bundled, possibly-stale `bindings.rs`) rather than hard-failing, so this is easy to miss:
   ```
   "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvarsall.bat" x64
   ```
   (Adjust the path for a Community/Professional/Enterprise install — Build Tools installs under `...\2022\BuildTools\...`, not the `...\18\Community\...`-style path a full Visual Studio installer uses.)

A full from-scratch Windows build session looks roughly like:

```powershell
$machPath = [System.Environment]::GetEnvironmentVariable("Path","Machine")
$userPath = [System.Environment]::GetEnvironmentVariable("Path","User")
$env:Path = "$machPath;$userPath;C:\Program Files\LLVM\bin"
$env:VULKAN_SDK = [System.Environment]::GetEnvironmentVariable("VULKAN_SDK","Machine")
$env:CMAKE_GENERATOR = "Ninja"

cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvarsall.bat" x64 && npm run tauri:build'
```

## Known Runtime Warnings

### libayatana-appindicator Deprecation (Linux)

When running VoxBridge Compose on Linux, you may see a warning in the terminal:
`libayatana-appindicator-WARNING: libayatana-appindicator is deprecated. Please use libayatana-appindicator-glib in newly written code.`

**Status:** This is a cosmetic warning that affects all Tauri v2 applications using tray icons on Linux. It does not affect functionality.

**Cause:** Tauri's tray implementation currently depends on the older `libayatana-appindicator3` library. The upstream project has released a newer `-glib` variant, but the Rust ecosystem bindings haven't migrated yet. No action is required from users or developers.

## What the Build Process Does

1. **Checks Dependencies** - Runs `npm run deps:check` and verifies required system libraries and toolchains.
2. **Builds Frontend** - Runs `npm run build` (type-check + Vite build).
3. **Builds Application** - Runs `tauri build` to create the final executable and installers.

## Optional Faster Whisper runtime

Faster Whisper/CTranslate2 is the default recognition backend owned by the
separate VoxBridge runtime. Portable installation and release packaging are
still being completed; source builds must prepare its managed runtime separately.
The compact whisper.cpp engine variants remain available as the compatibility
fallback.

Development copies of the optional backend require a managed Python runtime,
Faster Whisper, CTranslate2, a separate CTranslate2-format model directory, and
the appropriate CUDA runtime libraries for NVIDIA graphics execution. Do not
point a distributable build at a developer-machine Python installation or an
absolute local path. Portable runtime installation, verification, progress,
uninstallation, and Windows/Linux packaging must be completed before this
backend is release-ready.

## Output

After a successful build, you can find the artifacts in:
- **Linux**: `src-tauri/target/release/bundle/` (contains `.deb`, `.rpm`, `.AppImage`)
- **Windows**: `src-tauri/target/release/bundle/` (contains `.msi`, `.exe`)

## Troubleshooting

- **Missing dependencies on Linux**: If the build fails with missing library errors, follow the instructions provided by the script to install the necessary `apt` packages.
- **Frontend build issues**: If the UI fails to build, try clearing `node_modules` and running the build again.
- **Rust compilation errors**: Ensure your Rust toolchain is up to date with `rustup update`.
- **Fedora AppImage bundling**: Some Fedora toolchains ship RELR-enabled libraries that fail when stripped by the linuxdeploy binary bundled with Tauri. On Fedora, use `npm run tauri -- build --bundles deb,rpm` for distro packages, and build AppImage on Ubuntu/Mint/Kubuntu.
- **Windows: `Please install Vulkan SDK and ensure that VULKAN_SDK env variable is set`**: The Vulkan SDK isn't installed, or it was installed in a shell session that started before the install (so it doesn't have the env var yet). See the Windows section above.
- **Windows: `fatal error: 'stdio.h' file not found` (during bindgen)**: You're not in an MSVC developer environment. Run `vcvarsall.bat x64` first. This one only degrades to a (possibly stale) bundled `bindings.rs` rather than failing the build outright, so it's worth fixing even if the build "succeeds."
- **Windows: `Check for working C compiler: ...cl.exe - broken` / MSBuild `error MSB4018: The "GetOutOfDateItems" task failed unexpectedly`**: This is a known bug in MSBuild's `TryCompile` handling for the `vulkan-shaders-gen` sub-build inside whisper.cpp's CMake project, not a problem with your toolchain. Install Ninja and set `CMAKE_GENERATOR=Ninja` before building — CMake will use Ninja instead of generating MSBuild `.vcxproj` files for that check, which avoids the bug entirely.
- **Windows: build works the first time but fails after switching generators/toolchains**: Stale CMake cache. Delete the affected `whisper-rs-sys-<hash>` directory under `%CARGO_TARGET_DIR%\release\build\` (or the whole `build\` directory) to force a clean reconfigure, then rebuild.
- **Windows: `npm : The term 'npm' is not recognized...` inside a background/CI shell**: A freshly-installed toolchain (via winget/rustup/etc.) updates the registry's `PATH`, but processes already running — including a shell your build tooling spawned earlier in the same session — won't pick it up. Rebuild `PATH` explicitly from the registry (`[System.Environment]::GetEnvironmentVariable("Path","Machine")` + `"...","User"`) rather than assuming `$env:Path` is current.
- **Windows: builds are extremely slow to fail, or fail deep inside a native sub-build with an unhelpful path-related error**: Check `CARGO_TARGET_DIR`. The project runner uses a short local build path because native engine builds can otherwise exceed Windows path-length limits. If you invoke Cargo directly, choose another short local target path.
