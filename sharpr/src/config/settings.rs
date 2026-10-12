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
    /// Optional custom folder for resized copies.
    pub resize_output_dir: Option<PathBuf>,
    /// Maximum thumbnail entries to retain in memory.
    pub thumbnail_cache_max: i32,
    settings: gio::Settings,
}

fn settings_for_app() -> gio::Settings {
    if gio::SettingsSchemaSource::default()
        .as_ref()
        .and_then(|source| source.lookup("io.github.hebbihebb.Sharpr", true))
        .is_some()
    {
        return gio::Settings::new("io.github.hebbihebb.Sharpr");
    }

    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data");
    if let Ok(source) = gio::SettingsSchemaSource::from_directory(
        &data_dir,
        gio::SettingsSchemaSource::default().as_ref(),
        false,
    ) {
        if let Some(schema) = source.lookup("io.github.hebbihebb.Sharpr", true) {
            return gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None::<&str>);
        }
    }

    gio::Settings::new("io.github.hebbihebb.Sharpr")
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
            .field("resize_output_dir", &self.resize_output_dir)
            .field("thumbnail_cache_max", &self.thumbnail_cache_max)
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
            resize_output_dir: None,
            thumbnail_cache_max: 500,
            settings: settings_for_app(),
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        Self::load_from_settings(settings_for_app())
    }

    fn load_from_settings(settings: gio::Settings) -> Self {
        let last_folder = string_path(&settings, "last-folder");
        let library_root = string_path(&settings, "library-root");
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
            window_width: settings.int("window-width").clamp(400, 7680),
            window_height: settings.int("window-height").clamp(300, 4320),
            library_root,
            libraries,
            active_library_id,
            resize_output_dir: string_path(&settings, "resize-output-dir"),
            thumbnail_cache_max: settings.int("thumbnail-cache-max").clamp(100, 2000),
            settings,
        }
    }

    pub fn save(&self) {
        let path_string = |path: Option<&PathBuf>| {
            path.map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let _ = self
            .settings
            .set_string("last-folder", &path_string(self.last_folder.as_ref()));
        let _ = self
            .settings
            .set_boolean("metadata-visible", self.metadata_visible);
        let _ = self
            .settings
            .set_int("window-width", self.window_width.clamp(400, 7680));
        let _ = self
            .settings
            .set_int("window-height", self.window_height.clamp(300, 4320));
        let _ = self
            .settings
            .set_string("library-root", &path_string(self.library_root.as_ref()));
        let _ = self
            .settings
            .set_string("libraries-json", &libraries_json(&self.libraries));
        let _ = self.settings.set_string(
            "active-library-id",
            self.active_library_id.as_deref().unwrap_or_default(),
        );
        let _ = self.settings.set_string(
            "resize-output-dir",
            &path_string(self.resize_output_dir.as_ref()),
        );
        let _ = self.settings.set_int(
            "thumbnail-cache-max",
            self.thumbnail_cache_max.clamp(100, 2000),
        );
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

    pub fn set_thumbnail_cache_max(&mut self, value: i32) {
        self.thumbnail_cache_max = value.clamp(100, 2000);
        let _ = self
            .settings
            .set_int("thumbnail-cache-max", self.thumbnail_cache_max);
    }

    pub fn set_resize_output_dir(&mut self, path: Option<PathBuf>) {
        self.resize_output_dir = path;
        let value = self
            .resize_output_dir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = self.settings.set_string("resize-output-dir", &value);
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
            resize_output_dir: Some(PathBuf::from("/tmp/resized")),
            thumbnail_cache_max: -5,
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
            settings.string("resize-output-dir").as_str(),
            "/tmp/resized"
        );
        assert_eq!(settings.int("thumbnail-cache-max"), 100);
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
