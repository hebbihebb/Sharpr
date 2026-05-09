use crate::library_index::{LibraryIndex, Pipeline, StepType};
use crate::upscale::runner::preserved_png_temp_path;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct CompareAssetInfo {
    pub title: String,
    pub path: PathBuf,
    pub format: String,
    pub dimensions: (u32, u32),
    pub file_size: u64,
}

#[derive(Clone, Debug)]
pub struct CompareItem {
    pub pipeline_id: i64,
    pub source_path: PathBuf,
    pub output_path: PathBuf,
    pub original_asset: CompareAssetInfo,
    pub output_asset: CompareAssetInfo,
    pub preserved_png_asset: Option<CompareAssetInfo>,
    pub model: String,
    pub scale: String,
    pub date_added: glib::DateTime,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpscaleStepSettings {
    pub backend: String, // "cli" | "onnx" | "comfyui"
    pub model: String,   // "standard" | "anime"
    #[serde(default)]
    pub onnx_model: Option<String>,
    pub scale: u32, // 0 = smart/auto, 2, 3, 4
    pub compress: bool,
    pub format: String, // "jxl" | "webp" | "jpeg" | "png"
    pub quality: u8,
    #[serde(default)]
    pub keep_png: bool,
    #[serde(default = "default_destination")]
    pub destination: String, // "default" | "source" | "custom"
    #[serde(default)]
    pub custom_path: Option<PathBuf>,
    #[serde(default)]
    pub comfyui_workflow: Option<String>, // "esrgan" | "seedvr2"
}

fn default_destination() -> String {
    "default".to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ExportStepSettings {
    pub format: String,        // "jxl" | "webp" | "png" | "jpeg"
    pub max_edge: Option<u32>, // None = original size
    pub quality: u8,
    #[serde(default = "default_destination")]
    pub destination: String, // "default" | "source" | "custom"
    #[serde(default)]
    pub custom_path: Option<PathBuf>,
}

pub fn build_compare_asset_info(title: &str, path: &Path) -> CompareAssetInfo {
    CompareAssetInfo {
        title: title.to_string(),
        path: path.to_path_buf(),
        format: compare_asset_format(path),
        dimensions: image::image_dimensions(path).unwrap_or((0, 0)),
        file_size: std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0),
    }
}

pub fn compare_output_title(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "Output",
        _ => "Compressed Output",
    }
}

pub fn compare_asset_format(path: &Path) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "JPEG".to_string(),
        Some("png") => "PNG".to_string(),
        Some("webp") => "WebP".to_string(),
        Some("jxl") => "JPEG XL".to_string(),
        Some("avif") => "AVIF".to_string(),
        Some("tif") | Some("tiff") => "TIFF".to_string(),
        Some("bmp") => "BMP".to_string(),
        Some("gif") => "GIF".to_string(),
        Some(other) => other.to_ascii_uppercase(),
        None => "Unknown".to_string(),
    }
}

pub fn build_compare_item_from_pipeline(
    pipeline: &Pipeline,
    idx: &LibraryIndex,
) -> Option<CompareItem> {
    let steps = idx.steps_for_pipeline(pipeline.id).unwrap_or_default();

    // We want the latest completed/failed step that has an output
    let display_step = steps
        .iter()
        .rev()
        .find(|s| s.output_path.as_ref().map(|p| p.exists()).unwrap_or(false))?;

    let mut model = "Unknown".to_string();
    let mut scale = "-".to_string();

    match display_step.step_type {
        StepType::Upscale => {
            if let Ok(settings) =
                serde_json::from_str::<UpscaleStepSettings>(&display_step.settings_json)
            {
                model = settings.model.clone();
                scale = match settings.scale {
                    0 => "Smart scale".to_string(),
                    value => format!("{value}×"),
                };
            }
        }
        StepType::Export => {
            if let Ok(settings) =
                serde_json::from_str::<ExportStepSettings>(&display_step.settings_json)
            {
                model = "None (Export)".to_string();
                scale = settings
                    .max_edge
                    .map(|e| format!("Max {}px", e))
                    .unwrap_or_else(|| "Original".to_string());
            }
        }
    }

    let date_added = glib::DateTime::from_unix_local(pipeline.created_at)
        .unwrap_or_else(|_| glib::DateTime::now_local().unwrap());

    let output = display_step.output_path.as_ref().unwrap();
    let preserved_png = preserved_png_temp_path(output);

    Some(CompareItem {
        pipeline_id: pipeline.id,
        source_path: pipeline.source_path.clone(),
        output_path: output.clone(),
        original_asset: build_compare_asset_info("Original", &pipeline.source_path),
        output_asset: build_compare_asset_info(compare_output_title(output), output),
        preserved_png_asset: preserved_png
            .exists()
            .then(|| build_compare_asset_info("PNG Sidecar", &preserved_png)),
        model,
        scale,
        date_added,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_output_title_uses_plain_output_for_png() {
        assert_eq!(
            compare_output_title(Path::new("/tmp/example.png")),
            "Output"
        );
        assert_eq!(
            compare_output_title(Path::new("/tmp/example.jxl")),
            "Compressed Output"
        );
    }

    #[test]
    fn compare_asset_format_normalizes_common_extensions() {
        assert_eq!(compare_asset_format(Path::new("/tmp/example.jpg")), "JPEG");
        assert_eq!(
            compare_asset_format(Path::new("/tmp/example.jxl")),
            "JPEG XL"
        );
        assert_eq!(compare_asset_format(Path::new("/tmp/example.webp")), "WebP");
    }
}
