use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gdk4::Texture;
use gio::prelude::*;

use crate::model::image_entry::ImageEntry;

/// Known image file extensions (lower-case).
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "tiff", "tif", "bmp", "ico", "avif", "heic", "heif",
];

// ---------------------------------------------------------------------------
// LibraryManager
// ---------------------------------------------------------------------------

/// Central data store for the currently open folder's image list.
///
/// Holds a `GListStore<ImageEntry>` that is bound directly to the filmstrip
/// `GtkListView`. Also maintains an LRU-bounded thumbnail cache.
pub struct LibraryManager {
    pub store: gio::ListStore,
    pub current_folder: Option<PathBuf>,
    pub selected_index: Option<u32>,
    thumbnail_cache: HashMap<PathBuf, Texture>,
    /// Order of insertion for LRU eviction.
    cache_order: Vec<PathBuf>,
}

impl LibraryManager {
    const MAX_CACHE: usize = 500;

    pub fn new() -> Self {
        Self {
            store: gio::ListStore::new::<ImageEntry>(),
            current_folder: None,
            selected_index: None,
            thumbnail_cache: HashMap::new(),
            cache_order: Vec::new(),
        }
    }

    /// Scan `folder` for images and populate the store.
    /// Clears any existing entries first.
    pub fn scan_folder(&mut self, folder: &Path) {
        self.store.remove_all();
        self.selected_index = None;
        self.current_folder = Some(folder.to_path_buf());

        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };

        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.path())
            .filter(|p| Self::is_image(p))
            .collect();

        // Natural sort by filename.
        paths.sort_by(|a, b| {
            a.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
                .cmp(
                    &b.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase(),
                )
        });

        for path in paths {
            // Populate file_size eagerly (cheap stat).
            let entry = ImageEntry::new(path.clone());
            if let Ok(meta) = std::fs::metadata(&path) {
                entry.set_file_size(meta.len());
            }
            self.store.append(&entry);
        }
    }

    /// Returns the `ImageEntry` at `index`, if present.
    pub fn entry_at(&self, index: u32) -> Option<ImageEntry> {
        self.store.item(index).and_then(|o| o.downcast().ok())
    }

    /// Returns the currently selected `ImageEntry`.
    pub fn selected_entry(&self) -> Option<ImageEntry> {
        self.selected_index.and_then(|i| self.entry_at(i))
    }

    pub fn image_count(&self) -> u32 {
        self.store.n_items()
    }

    // -----------------------------------------------------------------------
    // Thumbnail cache
    // -----------------------------------------------------------------------

    pub fn cached_thumbnail(&self, path: &Path) -> Option<Texture> {
        self.thumbnail_cache.get(path).cloned()
    }

    pub fn insert_thumbnail(&mut self, path: PathBuf, texture: Texture) {
        if self.thumbnail_cache.contains_key(&path) {
            return;
        }
        if self.cache_order.len() >= Self::MAX_CACHE {
            // Evict the oldest entry.
            let oldest = self.cache_order.remove(0);
            self.thumbnail_cache.remove(&oldest);
        }
        self.thumbnail_cache.insert(path.clone(), texture);
        self.cache_order.push(path);
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn is_image(path: &Path) -> bool {
        path.extension()
            .map(|ext| {
                let low = ext.to_string_lossy().to_lowercase();
                IMAGE_EXTENSIONS.contains(&low.as_str())
            })
            .unwrap_or(false)
    }
}

impl Default for LibraryManager {
    fn default() -> Self {
        Self::new()
    }
}
