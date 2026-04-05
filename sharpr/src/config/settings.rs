use std::path::PathBuf;

use gio::prelude::*;

/// Application settings persisted via `gio::Settings`.
pub struct AppSettings {
    /// Last folder the user had open.
    pub last_folder: Option<PathBuf>,
    /// Whether the metadata overlay is visible by default.
    pub metadata_visible: bool,
    settings: gio::Settings,
}

impl std::fmt::Debug for AppSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppSettings")
            .field("last_folder", &self.last_folder)
            .field("metadata_visible", &self.metadata_visible)
            .finish()
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            last_folder: None,
            metadata_visible: true,
            settings: gio::Settings::new("io.github.hebbihebb.Sharpr"),
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        let settings = gio::Settings::new("io.github.hebbihebb.Sharpr");
        let last_folder = settings.string("last-folder");
        let last_folder = if last_folder.is_empty() {
            None
        } else {
            Some(PathBuf::from(last_folder.as_str()))
        };

        Self {
            last_folder,
            metadata_visible: settings.boolean("metadata-visible"),
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
    }
}
