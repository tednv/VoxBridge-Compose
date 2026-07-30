#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { logError, run } from "@tauri-apps/cli/main.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

if (process.platform === "win32") {
  process.env.CARGO_TARGET_DIR = "C:\\voxbridge-compose-build";
}

// Build the VoxBridge engine variants (see docs/ROADMAP.local.md and ../voxbridge/) before a
// real release build, so they're present in src-tauri/engines-dist/ for
// tauri.conf.json's bundle.resources to pick up. Skipped for `tauri dev` - engine
// builds take a few minutes (a full whisper.cpp/ggml CMake build per variant) and dev
// iteration doesn't need them; the app still falls back to whisper-rs when no engine
// DLLs are present. `voxbridge/` is a self-contained crate/build - this is the only place
// the application tells it where to put its output.
if (process.argv[2] === "build") {
  const voxbridgeScript = path.join(__dirname, "..", "voxbridge", "scripts", "build-engines.mjs");
  const outDir = path.join(__dirname, "..", "src-tauri", "engines-dist");
  const result = spawnSync("node", [voxbridgeScript, "--out-dir", outDir], {
    stdio: "inherit",
  });
  if (result.status !== 0) {
    console.error("VoxBridge engine build failed - aborting before the main Tauri build.");
    process.exit(result.status ?? 1);
  }
}

// Deliberately generic, not tuned to the build machine's exact CPU: a release binary
// gets built once and run on many different users' CPUs, so it must not bake in
// instructions (e.g. AVX-512) that the build machine happens to have but an end user's
// CPU doesn't — that mismatch is what originally caused Windows transcription crashes on
// hybrid Intel CPUs (see v1.3.18 release notes). Investigated removing this during local
// debugging on one specific machine; confirmed via an isolated reproduction that it was
// NOT actually the cause of that session's encode/decode failures (an unrelated
// whisper-rs abort_callback bug was) — so it stays off, matching upstream, rather than
// trading crash-safety on other users' hardware for an fix that wasn't needed here.
process.env.GGML_NATIVE = "OFF";

// Attempted GGML_BACKEND_DL + GGML_CPU_ALL_VARIANTS (proper runtime CPU-ISA dispatch)
// here — reverted. Real architectural wall, not a config problem: with
// GGML_BACKEND_DL, ggml builds *every* backend (including the baseline ggml-cpu/
// ggml-vulkan, not just the extra ISA variants) as a CMake MODULE library, which on
// Windows produces no import library by design (MODULE = dlopen-only, never linked
// directly). whisper-rs's own Rust bindings call functions like ggml_cpu_has_avx()
// expecting normal linking, so this breaks at link time (LNK1181: cannot open input
// file 'ggml-cpu.lib'). Fixing it would require patching whisper-rs itself (not just
// whisper-rs-sys) to route those calls through ggml's dynamic backend registry API
// instead of direct symbols. See vendor/whisper-rs-sys (kept for reference / a
// future attempt, not currently used — see the commented-out
// [patch.crates-io] in Cargo.toml).

try {
  await run(process.argv.slice(2), "tauri");
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  if (typeof logError === "function") logError(message);
  console.error(error);
  process.exit(1);
}
