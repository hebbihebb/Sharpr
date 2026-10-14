use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::AppSettings;
use crate::library_index::LibraryIndex;
use crate::model::library::SortOrder;
use crate::model::LibraryManager;

pub struct AppState {
    pub library: LibraryManager,
    pub settings: AppSettings,
    pub sort_order: SortOrder,
    pub library_index: Option<Arc<LibraryIndex>>,
    pub library_index_error: Option<String>,
    pub tags: Option<Arc<crate::tags::TagDatabase>>,
    pub tags_error: Option<String>,
    /// Set when moving bucket membership out of tags failed (Phase 4); the
    /// databases were left as they were.
    pub bucket_migration_error: Option<String>,
    pub ops: crate::ops::queue::OpQueue,
    pub selected_paths: HashSet<PathBuf>,
    /// Whether `selected_paths` was chosen on purpose (Ctrl- or
    /// Shift-click, Ctrl+A, Compare's survivors) rather than by a plain
    /// click, so a single chosen image counts as a selection.
    pub selection_explicit: bool,
    pub scope: ViewScope,
    pub current_focused_set: Option<crate::focused_image_set::FocusedImageSet>,
    pub previous_scope: Option<ViewScope>,
    pub previous_content_page: Option<String>,
    pub disabled_folders: Vec<PathBuf>,
    pub collections: Vec<crate::library_index::Collection>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum ViewScope {
    Folder(PathBuf),
    Collection(i64),
    Duplicates,
    #[default]
    Search,
    Compare,
}

impl AppState {
    pub fn new(ops: crate::ops::queue::OpQueue) -> Self {
        let settings = AppSettings::load();
        let mut library = LibraryManager::new();
        library.set_thumbnail_cache_max(settings.thumbnail_cache_max as usize);
        let (library_index, library_index_error) = match LibraryIndex::open() {
            Ok((index, _)) => (Some(Arc::new(index)), None),
            Err(error) => {
                let message = error.to_string();
                crate::bench_error!("index.open.fail", serde_json::json!({ "error": message }),);
                (None, Some(message))
            }
        };
        let disabled_folders = settings
            .active_library()
            .map(|library| library.ignored_folders.clone())
            .unwrap_or_default();
        let (tags, tags_error) = match crate::tags::TagDatabase::open() {
            Ok(tags) => (Some(Arc::new(tags)), None),
            Err(error) => {
                let message = error.to_string();
                crate::bench_error!("tags.open.fail", serde_json::json!({"error": message}));
                (None, Some(message))
            }
        };
        if let (Some(index), Some(tags)) = (&library_index, &tags) {
            let _ = index.migrate_legacy_collections_to_tags(tags);
        }
        if let (Some(index), Some(library)) = (&library_index, settings.active_library()) {
            let _ = index.assign_orphan_collections(&library.id);
        }
        // One-time move of bucket membership out of tags, after a backup.
        let bucket_migration_error = match (&library_index, &tags) {
            (Some(index), Some(tags)) => {
                let backups = dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("sharpr")
                    .join("backups");
                crate::buckets::migrate(index, tags, &backups)
                    .err()
                    .inspect(|error| {
                        crate::bench_error!(
                            "buckets.migrate.fail",
                            serde_json::json!({"error": error})
                        );
                    })
            }
            _ => None,
        };
        Self {
            library,
            settings,
            sort_order: SortOrder::default(),
            library_index,
            library_index_error,
            tags,
            tags_error,
            bucket_migration_error,
            ops,
            selected_paths: HashSet::new(),
            selection_explicit: false,
            scope: ViewScope::default(),
            current_focused_set: None,
            previous_scope: None,
            previous_content_page: None,
            disabled_folders,
            collections: Vec::new(),
        }
    }
}
