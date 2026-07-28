//! Exercises the real, compiled VoxBridge build pipeline end-to-end for this app:
//! resolves `engines-dist/` (as written by `scripts/build-engines.mjs`), picks the best
//! variant for this CPU, loads a real model, and transcribes a real recording.
//!
//! This lives in `src-tauri/tests/` (not the `voxbridge` crate itself) because it assumes
//! this app's own config/model/debug-recording directory conventions - the `voxbridge`
//! crate itself has no opinion on where those live.
//!
//! Requires `node ../voxbridge/scripts/build-engines.mjs` to have been run first, and a
//! downloaded model + a debug WAV to exist - skips itself with a clear message if
//! either is missing, rather than failing on a machine that hasn't built engines yet.

use std::path::Path;

#[test]
fn voxbridge_transcribes_real_audio() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let engines_dir = match voxbridge::resolve_engines_dir(&[manifest_dir]) {
        Some(dir) => dir,
        None => {
            eprintln!(
                "SKIPPED: no engines-dist/ found under {:?} - run `node ../voxbridge/scripts/build-engines.mjs --out-dir engines-dist` first",
                manifest_dir
            );
            return;
        }
    };

    let model_path = dirs::config_dir()
        .unwrap()
        .join("foss-voquill")
        .join("models")
        .join("ggml-distil-small.en.bin");
    if !model_path.exists() {
        eprintln!("SKIPPED: no test model at {:?}", model_path);
        return;
    }

    let debug_dir = dirs::config_dir().unwrap().join("foss-voquill").join("debug");
    let wav_path = std::fs::read_dir(&debug_dir).ok().and_then(|entries| {
        entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().map(|ext| ext == "wav").unwrap_or(false))
    });
    let wav_path = match wav_path {
        Some(path) => path,
        None => {
            eprintln!("SKIPPED: no test WAV found under {:?}", debug_dir);
            return;
        }
    };

    let engine = voxbridge::Engine::load_best(&engines_dir).expect("failed to load an engine variant");
    println!("Loaded VoxBridge engine variant: {}", engine.variant_name());

    let mut reader = hound::WavReader::open(&wav_path).expect("failed to open test wav");
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect();

    let model = engine
        .load_model(model_path.to_str().unwrap())
        .expect("failed to load model");
    let text = model
        .transcribe(&samples, Some("en"), None)
        .expect("transcription failed");

    println!("RESULT [{}]: \"{}\"", engine.variant_name(), text);
    assert!(!text.is_empty(), "transcription result should not be empty");
}
