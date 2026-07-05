use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::AppSettings;
use crate::library_index::LibraryIndex;
use crate::model::library::SortOrder;
use crate::model::LibraryManager;
use crate::ui::compare_item::CompareItem;

pub struct AppState {
    pub library: LibraryManager,
    pub settings: AppSettings,
    pub sort_order: SortOrder,
    pub library_index: Option<Arc<LibraryIndex>>,
    pub library_index_error: Option<String>,
    pub tags: Option<Arc<crate::tags::TagDatabase>>,
    pub ops: crate::ops::queue::OpQueue,
    pub selected_paths: HashSet<PathBuf>,
    pub scope: ViewScope,
    pub current_focused_set: Option<crate::focused_image_set::FocusedImageSet>,
    pub previous_scope: Option<ViewScope>,
    pub previous_content_page: Option<String>,
    pub comparison: Option<CompareItem>,
    pub disabled_folders: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum ViewScope {
    Folder(PathBuf),
    Collection(i64),
    Duplicates,
    #[default]
    Search,
    Quality(crate::quality::QualityClass),
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
        let tags = crate::tags::TagDatabase::open().ok().map(Arc::new);
        if let (Some(index), Some(tags)) = (&library_index, &tags) {
            let _ = index.migrate_legacy_collections_to_tags(tags);
        }
        if let (Some(index), Some(library)) = (&library_index, settings.active_library()) {
            let _ = index.assign_orphan_collections(&library.id);
        }
        Self {
            library,
            settings,
            sort_order: SortOrder::default(),
            library_index,
            library_index_error,
            tags,
            ops,
            selected_paths: HashSet::new(),
            scope: ViewScope::default(),
            current_focused_set: None,
            previous_scope: None,
            previous_content_page: None,
            comparison: None,
            disabled_folders,
        }
    }
}
