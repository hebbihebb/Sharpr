use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::upscale::{
    backend::UpscaleBackend, runner::UpscaleEvent, ComfyUiWorkflow, UpscaleJobConfig,
};

/// Thin HTTP client for a local ComfyUI server.
pub struct ComfyUiClient {
    pub base_url: String,
}

impl ComfyUiClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let url = base_url.into();
        Self {
            base_url: url.trim_end_matches('/').to_string(),
        }
    }

    /// Check that a ComfyUI server is reachable by hitting `/system_stats`.
    pub fn health_check(&self) -> Result<(), String> {
        let url = format!("{}/system_stats", self.base_url);
        ureq::get(&url)
            .timeout(Duration::from_secs(5))
            .call()
            .map(|_| ())
            .map_err(|e| format!("ComfyUI health check failed: {e}"))
    }

    /// Upload an image to ComfyUI's `/upload/image` endpoint.
    /// Returns the filename on the server.
    pub fn upload_image(&self, path: &Path) -> Result<String, String> {
        let mut file =
            std::fs::File::open(path).map_err(|e| format!("Failed to open input: {e}"))?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .map_err(|e| format!("Failed to read input: {e}"))?;

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("input.png");

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let boundary = format!("SharprBoundary{ts}");
        let mut body = Vec::new();

        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"image\"; filename=\"{filename}\"\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
        body.extend_from_slice(&buffer);
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

        let url = format!("{}/upload/image", self.base_url);
        let resp = ureq::post(&url)
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send_bytes(&body)
            .map_err(|e| format!("Upload failed: {e}"))?;

        let json: Value = resp
            .into_json()
            .map_err(|e| format!("Invalid upload response: {e}"))?;

        json["name"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "Missing 'name' in upload response".into())
    }

    /// Queue a prompt workflow on the ComfyUI server.
    /// Returns the prompt ID.
    pub fn queue_prompt(&self, workflow: Value) -> Result<String, String> {
        let url = format!("{}/prompt", self.base_url);
        let resp = ureq::post(&url)
            .send_json(json!({ "prompt": workflow }))
            .map_err(|e| format!("Queue failed: {e}"))?;

        let json: Value = resp
            .into_json()
            .map_err(|e| format!("Invalid prompt response: {e}"))?;

        json["prompt_id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "Missing 'prompt_id' in response".into())
    }

    /// Poll for a completed result by prompt ID.
    /// Returns the output filename if completed.
    pub fn poll_result(&self, prompt_id: &str) -> Result<Option<String>, String> {
        let url = format!("{}/history/{}", self.base_url, prompt_id);
        let resp = ureq::get(&url)
            .call()
            .map_err(|e| format!("History poll failed: {e}"))?;

        let json: Value = resp
            .into_json()
            .map_err(|e| format!("Invalid history response: {e}"))?;

        let job = &json[prompt_id];
        if job.is_null() {
            return Ok(None);
        }

        // Search for output images in the history
        if let Some(outputs) = job["outputs"].as_object() {
            for (_, out) in outputs {
                if let Some(images) = out["images"].as_array() {
                    if let Some(img) = images.first() {
                        if let Some(filename) = img["filename"].as_str() {
                            return Ok(Some(filename.to_string()));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Download the output image from the ComfyUI server.
    pub fn download_output(&self, filename: &str, dest: &Path) -> Result<(), String> {
        let url = format!(
            "{}/view?filename={}",
            self.base_url,
            urlencoding::encode(filename)
        );
        let resp = ureq::get(&url)
            .call()
            .map_err(|e| format!("Download failed: {e}"))?;

        let mut reader = resp.into_reader();
        let mut file =
            std::fs::File::create(dest).map_err(|e| format!("Failed to create output: {e}"))?;
        std::io::copy(&mut reader, &mut file).map_err(|e| format!("Failed to save output: {e}"))?;

        Ok(())
    }
}

/// Upscale backend that delegates to a local ComfyUI server.
pub struct ComfyUiBackend {
    pub client: ComfyUiClient,
    pub workflow: ComfyUiWorkflow,
}

impl ComfyUiBackend {
    pub fn new(base_url: impl Into<String>, workflow: ComfyUiWorkflow) -> Self {
        Self {
            client: ComfyUiClient::new(base_url),
            workflow,
        }
    }
}

impl UpscaleBackend for ComfyUiBackend {
    fn run(
        self: Box<Self>,
        input: PathBuf,
        output: PathBuf,
        config: UpscaleJobConfig,
    ) -> async_channel::Receiver<UpscaleEvent> {
        let (tx, rx) = async_channel::bounded(4);
        let client = self.client;
        let workflow_mode = self.workflow;

        std::thread::spawn(move || {
            let _ = tx.send_blocking(UpscaleEvent::Progress(Some(0.1)));

            // 1. Upload
            let remote_filename = match client.upload_image(&input) {
                Ok(f) => f,
                Err(e) => {
                    let _ = tx.send_blocking(UpscaleEvent::Failed(e));
                    return;
                }
            };
            let _ = tx.send_blocking(UpscaleEvent::Progress(Some(0.3)));

            // 2. Prepare workflow
            let workflow_str = workflow_template(workflow_mode);
            let mut workflow: Value = match serde_json::from_str(workflow_str) {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send_blocking(UpscaleEvent::Failed(format!("Invalid preset: {e}")));
                    return;
                }
            };

            // Patch input image
            if let Some(node) = workflow.get_mut("1") {
                if let Some(inputs) = node.get_mut("inputs") {
                    inputs["image"] = json!(remote_filename);
                }
            }

            if workflow_mode.uses_sharpr_model_picker() {
                if let Some(node) = workflow.get_mut("3") {
                    if let Some(inputs) = node.get_mut("inputs") {
                        let model_name = match config.model {
                            crate::upscale::UpscaleModel::Standard => "RealESRGAN_x4plus.pth",
                            crate::upscale::UpscaleModel::Anime => {
                                "RealESRGAN_x4plus_anime.pth"
                            }
                        };
                        inputs["model_name"] = json!(model_name);
                    }
                }
            }

            // 3. Queue
            let prompt_id = match client.queue_prompt(workflow) {
                Ok(id) => id,
                Err(e) => {
                    let _ = tx.send_blocking(UpscaleEvent::Failed(e));
                    return;
                }
            };
            let _ = tx.send_blocking(UpscaleEvent::Progress(Some(0.5)));

            // 4. Poll
            let mut output_filename = None;
            for _ in 0..300 {
                // 5 minutes timeout
                match client.poll_result(&prompt_id) {
                    Ok(Some(f)) => {
                        output_filename = Some(f);
                        break;
                    }
                    Ok(None) => {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    Err(e) => {
                        let _ = tx.send_blocking(UpscaleEvent::Failed(e));
                        return;
                    }
                }
            }

            let Some(f) = output_filename else {
                let _ =
                    tx.send_blocking(UpscaleEvent::Failed("Timeout waiting for ComfyUI".into()));
                return;
            };
            let _ = tx.send_blocking(UpscaleEvent::Progress(Some(0.8)));

            // 5. Download
            if let Err(e) = client.download_output(&f, &output) {
                let _ = tx.send_blocking(UpscaleEvent::Failed(e));
                return;
            }

            let _ = tx.send_blocking(UpscaleEvent::Progress(Some(1.0)));
            let _ = tx.send_blocking(UpscaleEvent::Done(output));
        });

        rx
    }
}

fn workflow_template(workflow: ComfyUiWorkflow) -> &'static str {
    match workflow {
        ComfyUiWorkflow::Esrgan => include_str!("comfy_preset.json"),
        ComfyUiWorkflow::SeedVr2 => include_str!("comfy_seedvr2.json"),
    }
}
