use std::path::PathBuf;

use gio::prelude::*;

/// Application settings persisted via `gio::Settings`.
#[derive(Clone)]
pub struct AppSettings {
    /// Last folder the user had open.
    pub last_folder: Option<PathBuf>,
    /// Whether the metadata overlay is visible by default.
    pub metadata_visible: bool,
    /// Restored main window width in logical pixels.
    pub window_width: i32,
    /// Restored main window height in logical pixels.
    pub window_height: i32,
    /// Optional custom root shown in preferences for future library scans.
    pub library_root: Option<PathBuf>,
    /// Optional custom path to the upscale binary.
    pub upscaler_binary_path: Option<PathBuf>,
    /// Default AI upscale model, stored as `"standard"` or `"anime"`.
    pub upscaler_default_model: String,
    /// Preferred output format: "auto", "jpeg", "png", or "webp".
    pub upscaler_output_format: String,
    /// Compression mode: "auto", "lossy", or "lossless".
    pub upscaler_compression_mode: String,
    /// Lossy quality target for JPEG-based output decisions.
    pub upscaler_quality: i32,
    /// Vulkan tile size override; 0 means auto.
    pub upscaler_tile_size: i32,
    /// Vulkan GPU device override; -1 means auto.
    pub upscaler_gpu_id: i32,
    /// Maximum thumbnail entries to retain in memory.
    pub thumbnail_cache_max: i32,
    /// The smart tagger ONNX model to use.
    pub smart_tagger_model: String,
    /// Whether the AI upscale entry point is exposed in the primary UI.
    pub show_upscale_ui: bool,
    /// Active upscale backend: "cli", "onnx", or "comfyui".
    pub upscale_backend: String,
    /// ONNX upscale model: "swin2sr-compressed-x4" or "swin2sr-real-x4".
    pub onnx_upscale_model: String,
    /// Base URL of the local ComfyUI server.
    pub comfyui_url: String,
    /// Whether the ComfyUI backend option is shown in the upscale dialog.
    pub comfyui_enabled: bool,
    settings: gio::Settings,
}

impl std::fmt::Debug for AppSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppSettings")
            .field("last_folder", &self.last_folder)
            .field("metadata_visible", &self.metadata_visible)
            .field("window_width", &self.window_width)
            .field("window_height", &self.window_height)
            .field("library_root", &self.library_root)
            .field("upscaler_binary_path", &self.upscaler_binary_path)
            .field("upscaler_default_model", &self.upscaler_default_model)
            .field("upscaler_output_format", &self.upscaler_output_format)
            .field("upscaler_compression_mode", &self.upscaler_compression_mode)
            .field("upscaler_quality", &self.upscaler_quality)
            .field("upscaler_tile_size", &self.upscaler_tile_size)
            .field("upscaler_gpu_id", &self.upscaler_gpu_id)
            .field("thumbnail_cache_max", &self.thumbnail_cache_max)
            .field("show_upscale_ui", &self.show_upscale_ui)
            .field("upscale_backend", &self.upscale_backend)
            .field("onnx_upscale_model", &self.onnx_upscale_model)
            .field("comfyui_url", &self.comfyui_url)
            .field("comfyui_enabled", &self.comfyui_enabled)
            .finish()
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            last_folder: None,
            metadata_visible: true,
            window_width: 1400,
            window_height: 900,
            library_root: None,
            upscaler_binary_path: None,
            upscaler_default_model: "standard".into(),
            upscaler_output_format: "auto".into(),
            upscaler_compression_mode: "auto".into(),
            upscaler_quality: 90,
            upscaler_tile_size: 0,
            upscaler_gpu_id: -1,
            thumbnail_cache_max: 500,
            smart_tagger_model: "resnet50-v1-7".into(),
            show_upscale_ui: false,
            upscale_backend: "cli".into(),
            onnx_upscale_model: "swin2sr-lightweight-x2".into(),
            comfyui_url: "http://127.0.0.1:8188".into(),
            comfyui_enabled: false,
            settings: gio::Settings::new("io.github.hebbihebb.Sharpr"),
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        let settings = gio::Settings::new("io.github.hebbihebb.Sharpr");
        let last_folder = string_path(&settings, "last-folder");
        let window_width = settings.int("window-width").clamp(400, 7680);
        let window_height = settings.int("window-height").clamp(300, 4320);
        let library_root = string_path(&settings, "library-root");
        let upscaler_binary_path = string_path(&settings, "upscaler-binary-path");
        let upscaler_default_model = match settings.string("upscaler-default-model").as_str() {
            "anime" => "anime".to_string(),
            _ => "standard".to_string(),
        };
        let upscaler_output_format = match settings.string("upscaler-output-format").as_str() {
            "jpeg" => "jpeg".to_string(),
            "png" => "png".to_string(),
            "webp" => "webp".to_string(),
            _ => "auto".to_string(),
        };
        let upscaler_compression_mode = match settings.string("upscaler-compression-mode").as_str()
        {
            "lossy" => "lossy".to_string(),
            "lossless" => "lossless".to_string(),
            _ => "auto".to_string(),
        };
        let upscaler_quality = settings.int("upscaler-quality").clamp(50, 100);
        let upscaler_tile_size = settings.int("upscaler-tile-size").clamp(0, 4096);
        let upscaler_gpu_id = settings.int("upscaler-gpu-id").clamp(-1, 16);
        let thumbnail_cache_max = settings.int("thumbnail-cache-max").clamp(100, 2000);
        let smart_tagger_model = match settings.string("smart-tagger-model").as_str() {
            "resnet18-v1-7" => "resnet18-v1-7".to_string(),
            "resnet152-v1-7" => "resnet152-v1-7".to_string(),
            _ => "resnet50-v1-7".to_string(),
        };
        let show_upscale_ui = settings.boolean("show-upscale-ui");
        let upscale_backend = match settings.string("upscale-backend").as_str() {
            "onnx" => "onnx".to_string(),
            _ => "cli".to_string(),
        };
        let onnx_upscale_model = match settings.string("onnx-upscale-model").as_str() {
            "swin2sr-compressed-x4" => "swin2sr-compressed-x4".to_string(),
            "swin2sr-real-x4" => "swin2sr-real-x4".to_string(),
            _ => "swin2sr-lightweight-x2".to_string(),
        };
        let comfyui_url = {
            let val = settings.string("comfyui-url");
            if val.is_empty() {
                "http://127.0.0.1:8188".to_string()
            } else {
                val.to_string()
            }
        };
        let comfyui_enabled = settings.boolean("comfyui-enabled");

        Self {
            last_folder,
            metadata_visible: settings.boolean("metadata-visible"),
            window_width,
            window_height,
            library_root,
            upscaler_binary_path,
            upscaler_default_model,
            upscaler_output_format,
            upscaler_compression_mode,
            upscaler_quality,
            upscaler_tile_size,
            upscaler_gpu_id,
            thumbnail_cache_max,
            smart_tagger_model,
            show_upscale_ui,
            upscale_backend,
            onnx_upscale_model,
            comfyui_url,
            comfyui_enabled,
            settings,
        }
    }

    pub fn save(&self) {
        let last_folder = self
            .last_folder
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = self.settings.set_string("last-folder", &last_folder);
        let _ = self
            .settings
            .set_boolean("metadata-visible", self.metadata_visible);
        let _ = self
            .settings
            .set_int("window-width", self.window_width.clamp(400, 7680));
        let _ = self
            .settings
            .set_int("window-height", self.window_height.clamp(300, 4320));
        let library_root = self
            .library_root
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = self.settings.set_string("library-root", &library_root);
        let upscaler_binary_path = self
            .upscaler_binary_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = self
            .settings
            .set_string("upscaler-binary-path", &upscaler_binary_path);
        let model = if self.upscaler_default_model == "anime" {
            "anime"
        } else {
            "standard"
        };
        let _ = self.settings.set_string("upscaler-default-model", model);
        let output_format = match self.upscaler_output_format.as_str() {
            "jpeg" | "png" | "webp" => self.upscaler_output_format.as_str(),
            _ => "auto",
        };
        let _ = self
            .settings
            .set_string("upscaler-output-format", output_format);
        let compression_mode = match self.upscaler_compression_mode.as_str() {
            "lossy" | "lossless" => self.upscaler_compression_mode.as_str(),
            _ => "auto",
        };
        let _ = self
            .settings
            .set_string("upscaler-compression-mode", compression_mode);
        let _ = self
            .settings
            .set_int("upscaler-quality", self.upscaler_quality.clamp(50, 100));
        let _ = self
            .settings
            .set_int("upscaler-tile-size", self.upscaler_tile_size.clamp(0, 4096));
        let _ = self
            .settings
            .set_int("upscaler-gpu-id", self.upscaler_gpu_id.clamp(-1, 16));
        let _ = self.settings.set_int(
            "thumbnail-cache-max",
            self.thumbnail_cache_max.clamp(100, 2000),
        );
        let model = match self.smart_tagger_model.as_str() {
            "resnet18-v1-7" | "resnet152-v1-7" => self.smart_tagger_model.as_str(),
            _ => "resnet50-v1-7",
        };
        let _ = self.settings.set_string("smart-tagger-model", model);
        let _ = self
            .settings
            .set_boolean("show-upscale-ui", self.show_upscale_ui);
        let backend = match self.upscale_backend.as_str() {
            "onnx" => "onnx",
            "comfyui" => "comfyui",
            _ => "cli",
        };
        let _ = self.settings.set_string("upscale-backend", backend);
        let onnx_model = match self.onnx_upscale_model.as_str() {
            "swin2sr-compressed-x4" => "swin2sr-compressed-x4",
            "swin2sr-real-x4" => "swin2sr-real-x4",
            _ => "swin2sr-lightweight-x2",
        };
        let _ = self.settings.set_string("onnx-upscale-model", onnx_model);
        let _ = self.settings.set_string("comfyui-url", &self.comfyui_url);
        let _ = self
            .settings
            .set_boolean("comfyui-enabled", self.comfyui_enabled);
    }

    pub fn set_upscale_backend(&mut self, value: &str) {
        self.upscale_backend = match value {
            "onnx" => "onnx".to_string(),
            "comfyui" => "comfyui".to_string(),
            _ => "cli".to_string(),
        };
        let _ = self
            .settings
            .set_string("upscale-backend", &self.upscale_backend);
    }

    pub fn set_comfyui_url(&mut self, value: &str) {
        self.comfyui_url = value.to_string();
        let _ = self.settings.set_string("comfyui-url", &self.comfyui_url);
    }

    pub fn set_comfyui_enabled(&mut self, value: bool) {
        self.comfyui_enabled = value;
        let _ = self.settings.set_boolean("comfyui-enabled", value);
    }

    pub fn set_onnx_upscale_model(&mut self, value: &str) {
        self.onnx_upscale_model = match value {
            "swin2sr-compressed-x4" => "swin2sr-compressed-x4".to_string(),
            "swin2sr-real-x4" => "swin2sr-real-x4".to_string(),
            _ => "swin2sr-lightweight-x2".to_string(),
        };
        let _ = self
            .settings
            .set_string("onnx-upscale-model", &self.onnx_upscale_model);
    }

    pub fn set_metadata_visible(&mut self, visible: bool) {
        self.metadata_visible = visible;
        let _ = self.settings.set_boolean("metadata-visible", visible);
    }

    pub fn set_library_root(&mut self, path: Option<PathBuf>) {
        self.library_root = path;
        let value = self
            .library_root
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = self.settings.set_string("library-root", &value);
    }

    pub fn set_upscaler_binary_path(&mut self, path: Option<PathBuf>) {
        self.upscaler_binary_path = path;
        let value = self
            .upscaler_binary_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = self.settings.set_string("upscaler-binary-path", &value);
    }

    pub fn set_upscaler_default_model(&mut self, model: &str) {
        self.upscaler_default_model = if model == "anime" {
            "anime".to_string()
        } else {
            "standard".to_string()
        };
        let _ = self
            .settings
            .set_string("upscaler-default-model", &self.upscaler_default_model);
    }

    pub fn set_upscaler_output_format(&mut self, value: &str) {
        self.upscaler_output_format = match value {
            "jpeg" => "jpeg".to_string(),
            "png" => "png".to_string(),
            "webp" => "webp".to_string(),
            _ => "auto".to_string(),
        };
        let _ = self
            .settings
            .set_string("upscaler-output-format", &self.upscaler_output_format);
    }

    pub fn set_upscaler_compression_mode(&mut self, value: &str) {
        self.upscaler_compression_mode = match value {
            "lossy" => "lossy".to_string(),
            "lossless" => "lossless".to_string(),
            _ => "auto".to_string(),
        };
        let _ = self
            .settings
            .set_string("upscaler-compression-mode", &self.upscaler_compression_mode);
    }

    pub fn set_upscaler_quality(&mut self, value: i32) {
        self.upscaler_quality = value.clamp(50, 100);
        let _ = self
            .settings
            .set_int("upscaler-quality", self.upscaler_quality);
    }

    pub fn set_upscaler_tile_size(&mut self, value: i32) {
        self.upscaler_tile_size = value.clamp(0, 4096);
        let _ = self
            .settings
            .set_int("upscaler-tile-size", self.upscaler_tile_size);
    }

    pub fn set_upscaler_gpu_id(&mut self, value: i32) {
        self.upscaler_gpu_id = value.clamp(-1, 16);
        let _ = self
            .settings
            .set_int("upscaler-gpu-id", self.upscaler_gpu_id);
    }

    pub fn set_thumbnail_cache_max(&mut self, value: i32) {
        self.thumbnail_cache_max = value.clamp(100, 2000);
        let _ = self
            .settings
            .set_int("thumbnail-cache-max", self.thumbnail_cache_max);
    }

    pub fn set_smart_tagger_model(&mut self, value: &str) {
        self.smart_tagger_model = match value {
            "resnet18-v1-7" => "resnet18-v1-7".to_string(),
            "resnet152-v1-7" => "resnet152-v1-7".to_string(),
            _ => "resnet50-v1-7".to_string(),
        };
        let _ = self
            .settings
            .set_string("smart-tagger-model", &self.smart_tagger_model);
    }

    pub fn set_show_upscale_ui(&mut self, value: bool) {
        self.show_upscale_ui = value;
        let _ = self.settings.set_boolean("show-upscale-ui", value);
    }
}

fn string_path(settings: &gio::Settings, key: &str) -> Option<PathBuf> {
    let value = settings.string(key);
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value.as_str()))
    }
}
