use dirs;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub engine: String,
    pub size: String,
    pub file_size: u64,
    pub download_url: String,
    pub sha256: String,
    pub label: String,
    pub description: String,
    pub recommended: bool,
    pub managed: bool,
}

pub struct ModelManager {
    pub models_dir: PathBuf,
}

impl ModelManager {
    fn contains_file_named(root: &std::path::Path, file_name: &str) -> bool {
        let Ok(entries) = std::fs::read_dir(root) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            if path.is_dir() {
                Self::contains_file_named(&path, file_name)
            } else {
                path.file_name().and_then(|name| name.to_str()) == Some(file_name)
            }
        })
    }

    fn file_size_named(root: &std::path::Path, file_name: &str) -> Option<u64> {
        let entries = std::fs::read_dir(root).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(size) = Self::file_size_named(&path, file_name) {
                    return Some(size);
                }
            } else if path.file_name().and_then(|name| name.to_str()) == Some(file_name) {
                return path.metadata().ok().map(|metadata| metadata.len());
            }
        }
        None
    }

    pub fn new() -> Result<Self, String> {
        let models_dir = dirs::config_dir()
            .ok_or("Could not find config directory")?
            .join("foss-voquill")
            .join("models");

        if !models_dir.exists() {
            std::fs::create_dir_all(&models_dir).map_err(|e| e.to_string())?;
        }

        Ok(Self { models_dir })
    }

    pub fn get_available_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                engine: "VoxBridge".to_string(),
                size: "tiny.en".to_string(),
                label: "Tiny (English)".to_string(),
                file_size: 77_600_000,
                download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin".to_string(),
                sha256: "be07098a4cc50130a511ca096303ad371c513297a7d4a093047d9ca4378f8776".to_string(),
                description: "Lightning fast, best for simple commands.".to_string(),
                recommended: false,
                managed: false,
            },
            ModelInfo {
                engine: "VoxBridge".to_string(),
                size: "distil-small.en".to_string(),
                label: "Distil-Small (English)".to_string(),
                file_size: 175_000_000,
                download_url: "https://huggingface.co/distil-whisper/distil-small.en/resolve/main/ggml-distil-small.en.bin".to_string(),
                sha256: "e8a676964fd3f78b021a385f078a18863712ca10fdc907a685eee9c0e71d7a62".to_string(),
                description: "Perfect balance of speed and high accuracy.".to_string(),
                recommended: true,
                managed: false,
            },
            ModelInfo {
                engine: "VoxBridge".to_string(),
                size: "base.en".to_string(),
                label: "Base (English)".to_string(),
                file_size: 147_000_000,
                download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin".to_string(),
                sha256: "60ed30914c83ad34005b63359d992f802773d57864f7df26e95261895697d74d".to_string(),
                description: "Standard choice for general dictation.".to_string(),
                recommended: false,
                managed: false,
            },
            ModelInfo {
                engine: "VoxBridge".to_string(),
                size: "small.en".to_string(),
                label: "Small (English)".to_string(),
                file_size: 483_000_000,
                download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin".to_string(),
                sha256: "1be3a305f560a8cc0937f268b7ca67270b240561570d55e09d949cf94edb54d1".to_string(),
                description: "Great accuracy for complex vocabulary.".to_string(),
                recommended: false,
                managed: false,
            },
            ModelInfo {
                engine: "VoxBridge".to_string(),
                size: "medium.en".to_string(),
                label: "Medium (English)".to_string(),
                file_size: 1_500_000_000,
                download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin".to_string(),
                sha256: "1be3a305f560a8cc0937f268b7ca67270b240561570d55e09d949cf94edb54d1".to_string(),
                description: "Highest accuracy. Needs a powerful computer or GPU.".to_string(),
                recommended: false,
                managed: false,
            },
            ModelInfo {
                engine: "VoxBridge Faster Whisper".to_string(),
                size: "fw-distil-small.en".to_string(),
                label: "Distil-Small (English)".to_string(),
                file_size: 330_000_000,
                download_url: String::new(),
                sha256: String::new(),
                description: "Default CTranslate2 model. Fast and efficient for English dictation.".to_string(),
                recommended: true,
                managed: true,
            },
            ModelInfo {
                engine: "VoxBridge Faster Whisper".to_string(),
                size: "fw-small.en".to_string(),
                label: "Small (English)".to_string(),
                file_size: 500_000_000,
                download_url: String::new(),
                sha256: String::new(),
                description: "CTranslate2 model with higher accuracy and memory use.".to_string(),
                recommended: false,
                managed: true,
            },
            ModelInfo {
                engine: "VoxBridge Faster Whisper".to_string(),
                size: "fw-medium.en".to_string(),
                label: "Medium (English)".to_string(),
                file_size: 1_500_000_000,
                download_url: String::new(),
                sha256: String::new(),
                description: "High-accuracy CTranslate2 model for capable systems.".to_string(),
                recommended: false,
                managed: true,
            },
        ]
    }

    pub fn get_available_engines() -> Vec<String> {
        let mut engines: Vec<String> = Self::get_available_models()
            .iter()
            .map(|m| m.engine.clone())
            .collect();
        engines.sort();
        engines.dedup();
        engines
    }

    pub fn get_model_path(&self, model_size: &str) -> PathBuf {
        self.models_dir.join(format!("ggml-{}.bin", model_size))
    }

    pub fn is_model_downloaded(&self, model_size: &str) -> bool {
        if let Some(model) = model_size.strip_prefix("fw-") {
            let root = self.models_dir.join("faster-whisper").join(model);
            let expected_size = Self::get_available_models()
                .into_iter()
                .find(|candidate| candidate.size == model_size)
                .map(|candidate| candidate.file_size)
                .unwrap_or(1);
            let model_bytes = Self::file_size_named(&root, "model.bin").unwrap_or(0);
            return Self::contains_file_named(&root, "config.json")
                && Self::contains_file_named(&root, "tokenizer.json")
                && model_bytes >= expected_size.saturating_mul(85) / 100;
        }
        self.get_model_path(model_size).exists()
    }

    pub async fn download_model<F>(
        &self,
        model_size: &str,
        progress_callback: F,
    ) -> Result<PathBuf, String>
    where
        F: Fn(f64) + Send + 'static,
    {
        let models = Self::get_available_models();
        let model_info = models
            .iter()
            .find(|m| m.size == model_size)
            .ok_or_else(|| format!("Model size {} not found", model_size))?;
        if model_info.managed {
            return Err(
                "Faster Whisper models are downloaded and prepared by the VoxBridge runtime."
                    .to_string(),
            );
        }

        let path = self.get_model_path(model_size);

        let client = reqwest::Client::new();
        let mut response = client
            .get(&model_info.download_url)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let total_size = response.content_length().unwrap_or(model_info.file_size);
        let mut downloaded: u64 = 0;
        let mut last_reported_progress: f64 = -1.0;

        let mut file = tokio::fs::File::create(&path)
            .await
            .map_err(|e| e.to_string())?;

        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            downloaded += chunk.len() as u64;

            let progress = (downloaded as f64 / total_size as f64) * 100.0;

            // Only report progress if it has increased by at least 0.5%
            // to prevent saturating the Tauri IPC bridge and freezing the UI
            if progress - last_reported_progress >= 0.5 || progress >= 100.0 {
                progress_callback(progress);
                last_reported_progress = progress;
            }
        }

        file.flush().await.map_err(|e| e.to_string())?;
        Ok(path)
    }
}
