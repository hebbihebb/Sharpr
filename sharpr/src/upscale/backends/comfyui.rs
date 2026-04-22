use std::path::PathBuf;

use crate::upscale::{backend::UpscaleBackend, runner::UpscaleEvent, UpscaleJobConfig};

/// Thin HTTP client for a local ComfyUI server.
pub struct ComfyUiClient {
    pub base_url: String,
}

impl ComfyUiClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    /// Check that a ComfyUI server is reachable by hitting `/system_stats`.
    pub fn health_check(&self) -> Result<(), String> {
        let url = format!("{}/system_stats", self.base_url.trim_end_matches('/'));
        ureq::get(&url)
            .call()
            .map(|_| ())
            .map_err(|e| format!("ComfyUI health check failed: {e}"))
    }

    /// Queue a prompt workflow on the ComfyUI server (stub — not yet implemented).
    #[allow(dead_code)]
    pub fn queue_prompt(&self, _workflow_json: &str) -> Result<String, String> {
        Err("not yet implemented".into())
    }

    /// Poll for a completed result by prompt ID (stub — not yet implemented).
    #[allow(dead_code)]
    pub fn poll_result(&self, _prompt_id: &str) -> Result<Option<String>, String> {
        Err("not yet implemented".into())
    }

    /// Download the output image from the ComfyUI server (stub — not yet implemented).
    #[allow(dead_code)]
    pub fn download_output(&self, _filename: &str, _dest: &PathBuf) -> Result<(), String> {
        Err("not yet implemented".into())
    }
}

/// Upscale backend that delegates to a local ComfyUI server.
///
/// This is a stub — it emits `Progress(None)` then `Failed("ComfyUI backend coming soon")`
/// without making any network requests, so the app never crashes when this backend
/// is selected before the real implementation lands.
pub struct ComfyUiBackend {
    pub client: ComfyUiClient,
    pub workflow_path: Option<PathBuf>,
}

impl ComfyUiBackend {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: ComfyUiClient::new(base_url),
            workflow_path: None,
        }
    }
}

impl UpscaleBackend for ComfyUiBackend {
    fn run(
        self: Box<Self>,
        _input: PathBuf,
        _output: PathBuf,
        _config: UpscaleJobConfig,
    ) -> async_channel::Receiver<UpscaleEvent> {
        let (tx, rx) = async_channel::bounded(4);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(UpscaleEvent::Progress(None));
            let _ = tx.send_blocking(UpscaleEvent::Failed(
                "ComfyUI backend coming soon".into(),
            ));
        });
        rx
    }
}
