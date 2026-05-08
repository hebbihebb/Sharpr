use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use image::imageops::FilterType;
use serde_json::{json, Value};

use crate::upscale::{
    backend::UpscaleBackend,
    runner::{finalize_output, UpscaleEvent},
    ComfyUiWorkflow, UpscaleJobConfig,
};

/// Thin HTTP client for a local ComfyUI server.
pub struct ComfyUiClient {
    pub base_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComfyUiOutputRef {
    pub filename: String,
    pub subfolder: String,
    pub folder_type: String,
}

enum ComfyUiPollState {
    Pending,
    Completed(ComfyUiOutputRef),
    Failed(String),
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
        body.extend_from_slice(
            format!("Content-Type: {}\r\n\r\n", content_type_for_path(path)).as_bytes(),
        );
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

        if let Some(prompt_id) = json["prompt_id"].as_str() {
            return Ok(prompt_id.to_string());
        }

        if let Some(error) = json.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
                .unwrap_or("ComfyUI rejected the prompt");
            let details = json
                .get("node_errors")
                .and_then(Value::as_object)
                .map(|errors| {
                    errors
                        .iter()
                        .map(|(node, err)| format!("{node}: {err}"))
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .filter(|details| !details.is_empty());
            return Err(match details {
                Some(details) => format!("{message} ({details})"),
                None => message.to_string(),
            });
        }

        Err("Missing 'prompt_id' in response".into())
    }

    /// Poll for a completed result by prompt ID.
    /// Returns the output filename if completed.
    pub fn poll_result(&self, prompt_id: &str) -> Result<Option<ComfyUiOutputRef>, String> {
        let url = format!("{}/history/{}", self.base_url, prompt_id);
        let resp = ureq::get(&url)
            .call()
            .map_err(|e| format!("History poll failed: {e}"))?;

        let json: Value = resp
            .into_json()
            .map_err(|e| format!("Invalid history response: {e}"))?;

        match parse_history_response(&json, prompt_id)? {
            ComfyUiPollState::Pending => Ok(None),
            ComfyUiPollState::Completed(output) => Ok(Some(output)),
            ComfyUiPollState::Failed(message) => Err(message),
        }
    }

    /// Download the output image from the ComfyUI server.
    pub fn download_output(&self, output: &ComfyUiOutputRef, dest: &Path) -> Result<(), String> {
        let mut query = vec![format!(
            "filename={}",
            urlencoding::encode(&output.filename)
        )];
        if !output.subfolder.is_empty() {
            query.push(format!(
                "subfolder={}",
                urlencoding::encode(&output.subfolder)
            ));
        }
        if !output.folder_type.is_empty() {
            query.push(format!("type={}", urlencoding::encode(&output.folder_type)));
        }
        let url = format!("{}/view?{}", self.base_url, query.join("&"));
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

            let prepared_input = match prepare_input_for_workflow(&input, &config, workflow_mode) {
                Ok(path) => path,
                Err(e) => {
                    let _ = tx.send_blocking(UpscaleEvent::Failed(e));
                    return;
                }
            };

            // 1. Upload
            let remote_filename = match client.upload_image(&prepared_input) {
                Ok(f) => f,
                Err(e) => {
                    cleanup_prepared_input(&prepared_input, &input);
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
                            crate::upscale::UpscaleModel::Anime => "RealESRGAN_x4plus_anime.pth",
                        };
                        inputs["model_name"] = json!(model_name);
                    }
                }
            }

            // 3. Queue
            let prompt_id = match client.queue_prompt(workflow) {
                Ok(id) => id,
                Err(e) => {
                    cleanup_prepared_input(&prepared_input, &input);
                    let _ = tx.send_blocking(UpscaleEvent::Failed(e));
                    return;
                }
            };
            let _ = tx.send_blocking(UpscaleEvent::Progress(Some(0.5)));

            // 4. Poll
            let mut output_ref = None;
            for _ in 0..300 {
                // 5 minutes timeout
                match client.poll_result(&prompt_id) {
                    Ok(Some(output)) => {
                        output_ref = Some(output);
                        break;
                    }
                    Ok(None) => {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    Err(e) => {
                        cleanup_prepared_input(&prepared_input, &input);
                        let _ = tx.send_blocking(UpscaleEvent::Failed(e));
                        return;
                    }
                }
            }

            cleanup_prepared_input(&prepared_input, &input);

            let Some(output_ref) = output_ref else {
                let _ =
                    tx.send_blocking(UpscaleEvent::Failed("Timeout waiting for ComfyUI".into()));
                return;
            };
            let _ = tx.send_blocking(UpscaleEvent::Progress(Some(0.8)));

            // 5. Download to a temporary PNG, then re-encode through Sharpr's
            // normal output pipeline so ComfyUI results respect the selected
            // format and compression settings.
            let downloaded = comfy_download_path(&output);
            if let Err(e) = client.download_output(&output_ref, &downloaded) {
                let _ = tx.send_blocking(UpscaleEvent::Failed(e));
                return;
            }

            if let Err(e) = finalize_output(&downloaded, &output, config) {
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

fn comfy_download_path(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("upscaled");
    output.with_file_name(format!("{stem}.comfy-download.png"))
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("gif") => "image/gif",
        Some("tif") | Some("tiff") => "image/tiff",
        _ => "image/png",
    }
}

fn parse_history_response(json: &Value, prompt_id: &str) -> Result<ComfyUiPollState, String> {
    let job = &json[prompt_id];
    if job.is_null() {
        return Ok(ComfyUiPollState::Pending);
    }

    if let Some(status_str) = job["status"]["status_str"].as_str() {
        if status_str.eq_ignore_ascii_case("error") {
            let message = job["status"]["messages"]
                .as_array()
                .and_then(|messages| messages.last())
                .map(|message| {
                    if let Some(arr) = message.as_array() {
                        if arr.get(0).and_then(|v| v.as_str()) == Some("execution_error") {
                            if let Some(details) = arr.get(1).and_then(|v| v.as_object()) {
                                let node_id = details
                                    .get("node_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                let exception_message = details
                                    .get("exception_message")
                                    .and_then(|v| v.as_str())
                                    .or_else(|| details.get("exception_type").and_then(|v| v.as_str()))
                                    .unwrap_or("Execution error");
                                return format!("Node {node_id}: {exception_message}");
                            }
                        }
                    }
                    message.to_string()
                })
                .unwrap_or_else(|| "ComfyUI job failed".to_string());
            return Ok(ComfyUiPollState::Failed(message));
        }
    }

    if let Some(outputs) = job["outputs"].as_object() {
        for out in outputs.values() {
            if let Some(images) = out["images"].as_array() {
                if let Some(img) = images.first() {
                    if let Some(filename) = img["filename"].as_str() {
                        return Ok(ComfyUiPollState::Completed(ComfyUiOutputRef {
                            filename: filename.to_string(),
                            subfolder: img["subfolder"].as_str().unwrap_or_default().to_string(),
                            folder_type: img["type"].as_str().unwrap_or("output").to_string(),
                        }));
                    }
                }
            }
        }
    }

    Ok(ComfyUiPollState::Pending)
}

fn prepare_input_for_workflow(
    input: &Path,
    config: &UpscaleJobConfig,
    workflow: ComfyUiWorkflow,
) -> Result<PathBuf, String> {
    if workflow != ComfyUiWorkflow::Esrgan {
        return Ok(input.to_path_buf());
    }

    let Some((target_width, target_height)) = desired_output_dimensions(config) else {
        return Ok(input.to_path_buf());
    };
    let (input_width, input_height) = config.source_dimensions;
    if input_width == 0 || input_height == 0 {
        return Ok(input.to_path_buf());
    }

    let native_scale = config.execution_scale.max(1);
    let upload_width = ((target_width as f32) / native_scale as f32)
        .round()
        .max(1.0) as u32;
    let upload_height = ((target_height as f32) / native_scale as f32)
        .round()
        .max(1.0) as u32;

    if upload_width == input_width && upload_height == input_height {
        return Ok(input.to_path_buf());
    }

    let image = image::ImageReader::open(input)
        .map_err(|e| format!("Failed to open input for ComfyUI resize: {e}"))?
        .decode()
        .map_err(|e| format!("Failed to decode input for ComfyUI resize: {e}"))?;
    let resized = image.resize_exact(upload_width, upload_height, FilterType::Lanczos3);
    let prepared_path = comfy_prepared_input_path(input);
    resized
        .save(&prepared_path)
        .map_err(|e| format!("Failed to save ComfyUI resized input: {e}"))?;
    Ok(prepared_path)
}

fn comfy_prepared_input_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("input");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{stem}.sharpr-comfy-input-{nanos}.png"))
}

fn cleanup_prepared_input(prepared_input: &Path, original_input: &Path) {
    if prepared_input != original_input {
        let _ = std::fs::remove_file(prepared_input);
    }
}

fn desired_output_dimensions(config: &UpscaleJobConfig) -> Option<(u32, u32)> {
    if let Some(target_dimensions) = config.target_dimensions {
        return Some(target_dimensions);
    }

    if config.requested_scale == 0 {
        return None;
    }

    let (width, height) = config.source_dimensions;
    if width == 0 || height == 0 {
        return None;
    }

    Some((
        width.saturating_mul(config.requested_scale),
        height.saturating_mul(config.requested_scale),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_history_response_reads_output_metadata() {
        let json = json!({
            "abc123": {
                "outputs": {
                    "4": {
                        "images": [{
                            "filename": "SharprUpscale_0001.png",
                            "subfolder": "Sharpr",
                            "type": "output"
                        }]
                    }
                }
            }
        });

        let state = parse_history_response(&json, "abc123").unwrap();
        match state {
            ComfyUiPollState::Completed(output) => {
                assert_eq!(
                    output,
                    ComfyUiOutputRef {
                        filename: "SharprUpscale_0001.png".to_string(),
                        subfolder: "Sharpr".to_string(),
                        folder_type: "output".to_string(),
                    }
                );
            }
            _ => panic!("expected completed state"),
        }
    }

    #[test]
    fn desired_output_dimensions_prefers_explicit_target() {
        let config = UpscaleJobConfig {
            source_dimensions: (1600, 900),
            requested_scale: 0,
            execution_scale: 4,
            target_dimensions: Some((3840, 2160)),
            model: crate::upscale::UpscaleModel::Standard,
            compress_output: false,
            compressed_format: crate::upscale::UpscaleOutputFormat::Png,
            keep_raw_png_sidecar: false,
            compression_mode: crate::upscale::UpscaleCompressionMode::Auto,
            quality: 90,
            tile_size: None,
            gpu_id: None,
        };

        assert_eq!(desired_output_dimensions(&config), Some((3840, 2160)));
    }

    #[test]
    fn parse_history_response_handles_execution_error() {
        let json = json!({
            "abc123": {
                "status": {
                    "status_str": "error",
                    "messages": [
                        [
                            "execution_error",
                            {
                                "node_id": "4",
                                "exception_message": "Some Python error",
                                "exception_type": "TypeError",
                                "current_inputs": {
                                    "batch_size": [1],
                                    "dit": ["{'model': 'some_model.gguf'}"]
                                }
                            }
                        ]
                    ]
                }
            }
        });

        let state = parse_history_response(&json, "abc123").unwrap();
        match state {
            ComfyUiPollState::Failed(msg) => {
                assert_eq!(msg, "Node 4: Some Python error");
            }
            _ => panic!("expected failed state"),
        }
    }
}
