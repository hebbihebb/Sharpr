use std::path::{Path, PathBuf};

use gio::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FolderMode {
    #[default]
    TopLevel,
    DrillDown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryConfig {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    #[serde(default)]
    pub folder_mode: FolderMode,
    #[serde(default)]
    pub last_folder: Option<PathBuf>,
    #[serde(default)]
    pub ignored_folders: Vec<PathBuf>,
}

impl LibraryConfig {
    fn new(name: String, root: PathBuf, folder_mode: FolderMode) -> Self {
        Self {
            id: new_library_id(),
            name,
            root,
            folder_mode,
            last_folder: None,
            ignored_folders: Vec::new(),
        }
    }
}

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
    /// Legacy single-library root kept for migration compatibility.
    pub library_root: Option<PathBuf>,
    /// Persisted library definitions.
    pub libraries: Vec<LibraryConfig>,
    /// Active library id.
    pub active_library_id: Option<String>,
    /// Optional custom path to the upscale binary.
    pub upscaler_binary_path: Option<PathBuf>,
    /// Optional custom folder for saved upscaled images.
    pub upscaled_output_dir: Option<PathBuf>,
    /// Optional custom folder for saved downscaled/exported images.
    pub export_output_dir: Option<PathBuf>,
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
    /// Active ComfyUI workflow preset: "esrgan" or "seedvr2".
    pub comfyui_workflow: String,
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
            .field("libraries", &self.libraries)
            .field("active_library_id", &self.active_library_id)
            .field("upscaler_binary_path", &self.upscaler_binary_path)
            .field("upscaled_output_dir", &self.upscaled_output_dir)
            .field("export_output_dir", &self.export_output_dir)
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
            .field("comfyui_workflow", &self.comfyui_workflow)
            .field("comfyui_enabled", &self.comfyui_enabled)
            .finish()
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        let root = default_library_root();
        let name = default_library_name(&root);
        let library = LibraryConfig::new(name, root, FolderMode::TopLevel);
        Self {
            last_folder: None,
            metadata_visible: true,
            window_width: 1400,
            window_height: 900,
            library_root: None,
            active_library_id: Some(library.id.clone()),
            libraries: vec![library],
            upscaler_binary_path: None,
            upscaled_output_dir: None,
            export_output_dir: None,
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
            comfyui_workflow: "esrgan".into(),
            comfyui_enabled: true,
            settings: gio::Settings::new("io.github.hebbihebb.Sharpr"),
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        Self::load_from_settings(gio::Settings::new("io.github.hebbihebb.Sharpr"))
    }

    fn load_from_settings(settings: gio::Settings) -> Self {
        let last_folder = string_path(&settings, "last-folder");
        let window_width = settings.int("window-width").clamp(400, 7680);
        let window_height = settings.int("window-height").clamp(300, 4320);
        let library_root = string_path(&settings, "library-root");
        let upscaler_binary_path = string_path(&settings, "upscaler-binary-path");
        let upscaled_output_dir = string_path(&settings, "upscaled-output-dir");
        let export_output_dir = string_path(&settings, "export-output-dir");
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
            "comfyui" => "comfyui".to_string(),
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
        let comfyui_workflow = match settings.string("comfyui-workflow").as_str() {
            "seedvr2" => "seedvr2".to_string(),
            _ => "esrgan".to_string(),
        };
        let comfyui_enabled = settings.boolean("comfyui-enabled");

        let mut libraries = parse_libraries_json(settings.string("libraries-json").as_str());
        if libraries.is_empty() {
            libraries = migrate_legacy_library(library_root.clone(), last_folder.clone());
        }
        sanitize_libraries(&mut libraries);

        let active_library_id =
            resolve_active_library_id(settings.string("active-library-id").as_str(), &libraries);
        let active_last_folder = active_library_id
            .as_ref()
            .and_then(|id| library_by_id(&libraries, id))
            .and_then(|library| library.last_folder.clone());

        Self {
            last_folder: active_last_folder.or(last_folder),
            metadata_visible: settings.boolean("metadata-visible"),
            window_width,
            window_height,
            library_root,
            libraries,
            active_library_id,
            upscaler_binary_path,
            upscaled_output_dir,
            export_output_dir,
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
            comfyui_workflow,
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
        let _ = self
            .settings
            .set_string("libraries-json", &libraries_json(&self.libraries));
        let _ = self.settings.set_string(
            "active-library-id",
            self.active_library_id.as_deref().unwrap_or_default(),
        );
        let upscaler_binary_path = self
            .upscaler_binary_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = self
            .settings
            .set_string("upscaler-binary-path", &upscaler_binary_path);
        let upscaled_output_dir = self
            .upscaled_output_dir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = self
            .settings
            .set_string("upscaled-output-dir", &upscaled_output_dir);
        let export_output_dir = self
            .export_output_dir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = self
            .settings
            .set_string("export-output-dir", &export_output_dir);
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
        let comfyui_workflow = match self.comfyui_workflow.as_str() {
            "seedvr2" => "seedvr2",
            _ => "esrgan",
        };
        let _ = self
            .settings
            .set_string("comfyui-workflow", comfyui_workflow);
        let _ = self
            .settings
            .set_boolean("comfyui-enabled", self.comfyui_enabled);
    }

    pub fn active_library(&self) -> Option<&LibraryConfig> {
        self.active_library_id
            .as_deref()
            .and_then(|id| library_by_id(&self.libraries, id))
            .or_else(|| self.libraries.first())
    }

    pub fn active_library_mut(&mut self) -> Option<&mut LibraryConfig> {
        let id = self.active_library_id.clone()?;
        self.libraries.iter_mut().find(|library| library.id == id)
    }

    pub fn set_active_library(&mut self, id: &str) {
        if self.libraries.iter().any(|library| library.id == id) {
            self.active_library_id = Some(id.to_string());
            self.last_folder = self
                .libraries
                .iter()
                .find(|library| library.id == id)
                .and_then(|library| library.last_folder.clone());
            self.save();
        }
    }

    pub fn add_library(
        &mut self,
        name: &str,
        root: PathBuf,
        folder_mode: FolderMode,
    ) -> Result<String, String> {
        validate_library_fields(name, &root)?;
        validate_no_root_overlap(&self.libraries, &root, None)?;
        let library = LibraryConfig::new(name.trim().to_string(), root.clone(), folder_mode);
        let id = library.id.clone();
        self.library_root.get_or_insert(root);
        self.active_library_id = Some(id.clone());
        self.last_folder = None;
        self.libraries.push(library);
        self.save();
        Ok(id)
    }

    pub fn update_library(
        &mut self,
        id: &str,
        name: &str,
        root: PathBuf,
        folder_mode: FolderMode,
    ) -> Result<(), String> {
        validate_library_fields(name, &root)?;
        validate_no_root_overlap(&self.libraries, &root, Some(id))?;
        let library_last_folder = {
            let Some(library) = self.libraries.iter_mut().find(|library| library.id == id) else {
                return Err("Library not found".into());
            };
            library.name = name.trim().to_string();
            library.root = root;
            library.folder_mode = folder_mode;
            library
                .ignored_folders
                .retain(|path| path.starts_with(&library.root));
            if library
                .last_folder
                .as_ref()
                .is_some_and(|path| !path.starts_with(&library.root))
            {
                library.last_folder = None;
            }
            library.last_folder.clone()
        };
        if self.active_library_id.as_deref() == Some(id) {
            self.last_folder = library_last_folder;
        }
        self.save();
        Ok(())
    }

    pub fn set_active_library_last_folder(&mut self, folder: Option<PathBuf>) {
        if let Some(library) = self.active_library_mut() {
            library.last_folder = folder.clone();
        }
        self.last_folder = folder;
        self.save();
    }

    pub fn set_active_library_ignored_folders(&mut self, ignored_folders: Vec<PathBuf>) {
        if let Some(library) = self.active_library_mut() {
            library.ignored_folders = ignored_folders;
        }
        self.save();
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

    pub fn set_comfyui_workflow(&mut self, value: &str) {
        self.comfyui_workflow = match value {
            "seedvr2" => "seedvr2".to_string(),
            _ => "esrgan".to_string(),
        };
        let _ = self
            .settings
            .set_string("comfyui-workflow", &self.comfyui_workflow);
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

    pub fn set_upscaled_output_dir(&mut self, path: Option<PathBuf>) {
        self.upscaled_output_dir = path;
        let value = self
            .upscaled_output_dir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = self.settings.set_string("upscaled-output-dir", &value);
    }

    pub fn set_export_output_dir(&mut self, path: Option<PathBuf>) {
        self.export_output_dir = path;
        let value = self
            .export_output_dir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = self.settings.set_string("export-output-dir", &value);
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

fn library_by_id<'a>(libraries: &'a [LibraryConfig], id: &str) -> Option<&'a LibraryConfig> {
    libraries.iter().find(|library| library.id == id)
}

fn string_path(settings: &gio::Settings, key: &str) -> Option<PathBuf> {
    let value = settings.string(key);
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value.as_str()))
    }
}

fn parse_libraries_json(raw: &str) -> Vec<LibraryConfig> {
    serde_json::from_str::<Vec<LibraryConfig>>(raw).unwrap_or_default()
}

fn libraries_json(libraries: &[LibraryConfig]) -> String {
    serde_json::to_string(libraries).unwrap_or_else(|_| "[]".to_string())
}

fn sanitize_libraries(libraries: &mut Vec<LibraryConfig>) {
    let mut out = Vec::new();
    for mut library in libraries.drain(..) {
        if library.id.trim().is_empty() {
            library.id = new_library_id();
        }
        library.name = library.name.trim().to_string();
        if library.name.is_empty() {
            library.name = default_library_name(&library.root);
        }
        library.ignored_folders = library
            .ignored_folders
            .into_iter()
            .filter(|path| path.starts_with(&library.root))
            .collect();
        if library
            .last_folder
            .as_ref()
            .is_some_and(|path| !path.starts_with(&library.root))
        {
            library.last_folder = None;
        }
        if !out.iter().any(|existing: &LibraryConfig| {
            roots_overlap(existing.root.as_path(), library.root.as_path())
        }) {
            out.push(library);
        }
    }
    if out.is_empty() {
        let root = default_library_root();
        out.push(LibraryConfig::new(
            default_library_name(&root),
            root,
            FolderMode::TopLevel,
        ));
    }
    *libraries = out;
}

fn migrate_legacy_library(
    legacy_root: Option<PathBuf>,
    legacy_last_folder: Option<PathBuf>,
) -> Vec<LibraryConfig> {
    let root = legacy_root.unwrap_or_else(default_library_root);
    let mut library = LibraryConfig::new(
        default_library_name(&root),
        root.clone(),
        FolderMode::TopLevel,
    );
    library.last_folder = legacy_last_folder.filter(|path| path.starts_with(&root));
    vec![library]
}

fn default_library_root() -> PathBuf {
    dirs::picture_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_library_name(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Library".to_string())
}

fn resolve_active_library_id(raw: &str, libraries: &[LibraryConfig]) -> Option<String> {
    if libraries.is_empty() {
        return None;
    }
    if libraries.iter().any(|library| library.id == raw) {
        Some(raw.to_string())
    } else {
        Some(libraries[0].id.clone())
    }
}

fn new_library_id() -> String {
    let micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_micros())
        .unwrap_or(0);
    format!("library-{micros}")
}

fn validate_library_fields(name: &str, root: &Path) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Library name is required".into());
    }
    if !root.is_dir() {
        return Err("Library root must be an existing folder".into());
    }
    Ok(())
}

fn validate_no_root_overlap(
    libraries: &[LibraryConfig],
    root: &Path,
    skip_id: Option<&str>,
) -> Result<(), String> {
    for library in libraries {
        if skip_id == Some(library.id.as_str()) {
            continue;
        }
        if roots_overlap(library.root.as_path(), root) {
            return Err(format!(
                "Library root overlaps with existing library “{}”",
                library.name
            ));
        }
    }
    Ok(())
}

fn roots_overlap(a: &Path, b: &Path) -> bool {
    a == b || a.starts_with(b) || b.starts_with(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn settings_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn test_settings() -> gio::Settings {
        let _guard = settings_test_lock().lock().unwrap();
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data");
        let source = gio::SettingsSchemaSource::from_directory(
            &data_dir,
            gio::SettingsSchemaSource::default().as_ref(),
            false,
        )
        .expect("compiled test schema should be available");
        let schema = source
            .lookup("io.github.hebbihebb.Sharpr", true)
            .expect("Sharpr schema should be loadable for tests");
        let backend = gio::memory_settings_backend_new();
        gio::Settings::new_full(&schema, Some(&backend), None::<&str>)
    }

    #[test]
    fn load_normalizes_invalid_and_out_of_range_values() {
        let settings = test_settings();
        let _ = settings.set_int("window-width", 20);
        let _ = settings.set_int("window-height", 99999);
        let _ = settings.set_int("upscaler-quality", 5);
        let _ = settings.set_int("upscaler-tile-size", 50000);
        let _ = settings.set_int("upscaler-gpu-id", 99);
        let _ = settings.set_int("thumbnail-cache-max", 2);
        let _ = settings.set_string("upscaler-default-model", "bogus");
        let _ = settings.set_string("upscaler-output-format", "tiff");
        let _ = settings.set_string("upscaler-compression-mode", "zip");
        let _ = settings.set_string("smart-tagger-model", "resnet34");
        let _ = settings.set_string("upscale-backend", "mystery");
        let _ = settings.set_string("onnx-upscale-model", "unknown");
        let _ = settings.set_string("comfyui-url", "");
        let _ = settings.set_string("comfyui-workflow", "mystery");
        let _ = settings.set_string("library-root", "/tmp/library");
        let _ = settings.set_string("libraries-json", "[]");
        let _ = settings.set_string("upscaler-binary-path", "/tmp/upscaler");
        let _ = settings.set_string("upscaled-output-dir", "/tmp/upscaled");
        let _ = settings.set_string("export-output-dir", "/tmp/export");
        let _ = settings.set_boolean("comfyui-enabled", true);
        let _ = settings.set_boolean("show-upscale-ui", true);

        let loaded = AppSettings::load_from_settings(settings);
        assert_eq!(loaded.window_width, 400);
        assert_eq!(loaded.window_height, 4320);
        assert_eq!(loaded.upscaler_quality, 50);
        assert_eq!(loaded.upscaler_tile_size, 4096);
        assert_eq!(loaded.upscaler_gpu_id, 16);
        assert_eq!(loaded.thumbnail_cache_max, 100);
        assert_eq!(loaded.upscaler_default_model, "standard");
        assert_eq!(loaded.upscaler_output_format, "auto");
        assert_eq!(loaded.upscaler_compression_mode, "auto");
        assert_eq!(loaded.smart_tagger_model, "resnet50-v1-7");
        assert_eq!(loaded.upscale_backend, "cli");
        assert_eq!(loaded.onnx_upscale_model, "swin2sr-lightweight-x2");
        assert_eq!(loaded.comfyui_url, "http://127.0.0.1:8188");
        assert_eq!(loaded.comfyui_workflow, "esrgan");
        assert_eq!(loaded.library_root, Some(PathBuf::from("/tmp/library")));
        assert_eq!(loaded.libraries.len(), 1);
        assert_eq!(loaded.libraries[0].root, PathBuf::from("/tmp/library"));
        assert_eq!(
            loaded.upscaler_binary_path,
            Some(PathBuf::from("/tmp/upscaler"))
        );
        assert_eq!(
            loaded.upscaled_output_dir,
            Some(PathBuf::from("/tmp/upscaled"))
        );
        assert_eq!(loaded.export_output_dir, Some(PathBuf::from("/tmp/export")));
        assert!(loaded.comfyui_enabled);
        assert!(loaded.show_upscale_ui);
    }

    #[test]
    fn load_preserves_comfyui_backend() {
        let settings = test_settings();
        let _ = settings.set_string("upscale-backend", "comfyui");

        let loaded = AppSettings::load_from_settings(settings);

        assert_eq!(loaded.upscale_backend, "comfyui");
    }

    #[test]
    fn save_clamps_and_normalizes_persisted_values() {
        let settings = test_settings();
        let library = LibraryConfig {
            id: "library-1".into(),
            name: "Primary".into(),
            root: PathBuf::from("/tmp/library"),
            folder_mode: FolderMode::DrillDown,
            last_folder: Some(PathBuf::from("/tmp/library/Art")),
            ignored_folders: vec![PathBuf::from("/tmp/library/Ignore")],
        };
        let app = AppSettings {
            last_folder: Some(PathBuf::from("/tmp/library/Art")),
            metadata_visible: false,
            window_width: 10,
            window_height: 50000,
            library_root: Some(PathBuf::from("/tmp/library")),
            libraries: vec![library],
            active_library_id: Some("library-1".into()),
            upscaler_binary_path: Some(PathBuf::from("/tmp/upscaler")),
            upscaled_output_dir: Some(PathBuf::from("/tmp/upscaled")),
            export_output_dir: Some(PathBuf::from("/tmp/export")),
            upscaler_default_model: "weird".into(),
            upscaler_output_format: "bmp".into(),
            upscaler_compression_mode: "strange".into(),
            upscaler_quality: 1000,
            upscaler_tile_size: -7,
            upscaler_gpu_id: 999,
            thumbnail_cache_max: -5,
            smart_tagger_model: "broken".into(),
            show_upscale_ui: true,
            upscale_backend: "mystery".into(),
            onnx_upscale_model: "broken".into(),
            comfyui_url: "http://localhost:9000".into(),
            comfyui_workflow: "unknown".into(),
            comfyui_enabled: true,
            settings: settings.clone(),
        };

        app.save();

        assert_eq!(settings.string("last-folder").as_str(), "/tmp/library/Art");
        assert!(!settings.boolean("metadata-visible"));
        assert_eq!(settings.int("window-width"), 400);
        assert_eq!(settings.int("window-height"), 4320);
        assert_eq!(settings.string("library-root").as_str(), "/tmp/library");
        assert_eq!(settings.string("active-library-id").as_str(), "library-1");
        assert!(settings.string("libraries-json").contains("\"Primary\""));
        assert_eq!(
            settings.string("upscaler-binary-path").as_str(),
            "/tmp/upscaler"
        );
        assert_eq!(
            settings.string("upscaled-output-dir").as_str(),
            "/tmp/upscaled"
        );
        assert_eq!(settings.string("export-output-dir").as_str(), "/tmp/export");
        assert_eq!(
            settings.string("upscaler-default-model").as_str(),
            "standard"
        );
        assert_eq!(settings.string("upscaler-output-format").as_str(), "auto");
        assert_eq!(
            settings.string("upscaler-compression-mode").as_str(),
            "auto"
        );
        assert_eq!(settings.int("upscaler-quality"), 100);
        assert_eq!(settings.int("upscaler-tile-size"), 0);
        assert_eq!(settings.int("upscaler-gpu-id"), 16);
        assert_eq!(settings.int("thumbnail-cache-max"), 100);
        assert_eq!(
            settings.string("smart-tagger-model").as_str(),
            "resnet50-v1-7"
        );
        assert_eq!(settings.string("upscale-backend").as_str(), "cli");
        assert_eq!(
            settings.string("onnx-upscale-model").as_str(),
            "swin2sr-lightweight-x2"
        );
        assert_eq!(
            settings.string("comfyui-url").as_str(),
            "http://localhost:9000"
        );
        assert_eq!(settings.string("comfyui-workflow").as_str(), "esrgan");
        assert!(settings.boolean("comfyui-enabled"));
        assert!(settings.boolean("show-upscale-ui"));
    }

    #[test]
    fn add_library_rejects_overlapping_roots() {
        let settings = test_settings();
        let mut app = AppSettings::load_from_settings(settings);
        app.libraries = vec![LibraryConfig::new(
            "Primary".into(),
            PathBuf::from("/tmp/photos"),
            FolderMode::TopLevel,
        )];
        app.active_library_id = Some(app.libraries[0].id.clone());

        let result = app.add_library(
            "Nested",
            PathBuf::from("/tmp/photos/child"),
            FolderMode::TopLevel,
        );

        assert!(result.is_err());
    }
}
