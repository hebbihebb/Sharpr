use std::path::PathBuf;

use gio::prelude::*;

/// Application settings persisted via `gio::Settings`.
#[derive(Clone)]
pub struct AppSettings {
    /// Last folder the user had open.
    pub last_folder: Option<PathBuf>,
    /// Whether the metadata overlay is visible by default.
    pub metadata_visible: bool,
    /// Optional custom root shown in preferences for future library scans.
    pub library_root: Option<PathBuf>,
    /// Optional custom path to the upscale binary.
    pub upscaler_binary_path: Option<PathBuf>,
    /// Default AI upscale model, stored as `"standard"` or `"anime"`.
    pub upscaler_default_model: String,
    /// Maximum thumbnail entries to retain in memory.
    pub thumbnail_cache_max: i32,
    settings: gio::Settings,
}

impl std::fmt::Debug for AppSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppSettings")
            .field("last_folder", &self.last_folder)
            .field("metadata_visible", &self.metadata_visible)
            .field("library_root", &self.library_root)
            .field("upscaler_binary_path", &self.upscaler_binary_path)
            .field("upscaler_default_model", &self.upscaler_default_model)
            .field("thumbnail_cache_max", &self.thumbnail_cache_max)
            .finish()
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            last_folder: None,
            metadata_visible: true,
            library_root: None,
            upscaler_binary_path: None,
            upscaler_default_model: "standard".into(),
            thumbnail_cache_max: 500,
            settings: gio::Settings::new("io.github.hebbihebb.Sharpr"),
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        let settings = gio::Settings::new("io.github.hebbihebb.Sharpr");
        let last_folder = string_path(&settings, "last-folder");
        let library_root = string_path(&settings, "library-root");
        let upscaler_binary_path = string_path(&settings, "upscaler-binary-path");
        let upscaler_default_model = match settings.string("upscaler-default-model").as_str() {
            "anime" => "anime".to_string(),
            _ => "standard".to_string(),
        };
        let thumbnail_cache_max = settings.int("thumbnail-cache-max").clamp(100, 2000);

        Self {
            last_folder,
            metadata_visible: settings.boolean("metadata-visible"),
            library_root,
            upscaler_binary_path,
            upscaler_default_model,
            thumbnail_cache_max,
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
        let _ = self.settings.set_int(
            "thumbnail-cache-max",
            self.thumbnail_cache_max.clamp(100, 2000),
        );
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

    pub fn set_thumbnail_cache_max(&mut self, value: i32) {
        self.thumbnail_cache_max = value.clamp(100, 2000);
        let _ = self
            .settings
            .set_int("thumbnail-cache-max", self.thumbnail_cache_max);
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
