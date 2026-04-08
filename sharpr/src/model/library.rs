use std::collections::{HashMap, HashSet};
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
/// Holds a `GListStore<ImageEntry>` bound directly to the filmstrip `GtkListView`.
/// Maintains an O(1) path→index lookup map and an LRU-bounded thumbnail cache.
pub struct LibraryManager {
    pub store: gio::ListStore,
    pub current_folder: Option<PathBuf>,
    pub selected_index: Option<u32>,
    folder_history: HashMap<PathBuf, u32>,
    /// O(1) path → list index lookup, kept in sync with `store`.
    path_to_index: HashMap<PathBuf, u32>,
    /// Set of all image paths encountered during the session, for cross-folder duplicates.
    all_known_paths: HashSet<PathBuf>,
    hash_store: HashMap<PathBuf, u64>,
    thumbnail_cache: HashMap<PathBuf, Texture>,
    /// Insertion order for LRU eviction.
    cache_order: Vec<PathBuf>,
    prefetch_cache: HashMap<PathBuf, (Vec<u8>, u32, u32)>,
    prefetch_order: Vec<PathBuf>,
    prefetch_in_flight: HashSet<PathBuf>,
    preview_cache: HashMap<PathBuf, (Vec<u8>, u32, u32)>,
    preview_order: Vec<PathBuf>,
    thumbnail_cache_max: usize,
}

impl LibraryManager {
    const MAX_PREVIEW_CACHE: usize = 30;

    pub fn new() -> Self {
        Self {
            store: gio::ListStore::new::<ImageEntry>(),
            current_folder: None,
            selected_index: None,
            folder_history: HashMap::new(),
            path_to_index: HashMap::new(),
            all_known_paths: HashSet::new(),
            hash_store: HashMap::new(),
            thumbnail_cache: HashMap::new(),
            cache_order: Vec::new(),
            prefetch_cache: HashMap::new(),
            prefetch_order: Vec::new(),
            prefetch_in_flight: HashSet::new(),
            preview_cache: HashMap::new(),
            preview_order: Vec::new(),
            thumbnail_cache_max: 500,
        }
    }

    /// Scan `folder` for images and populate the store.
    /// Clears all existing entries and rebuilds the index map.
    pub fn scan_folder(&mut self, folder: &Path) {
        self.store.remove_all();
        self.path_to_index.clear();
        self.hash_store.clear();
        self.prefetch_cache.clear();
        self.prefetch_order.clear();
        self.prefetch_in_flight.clear();
        self.preview_cache.clear();
        self.preview_order.clear();
        if let (Some(folder), Some(idx)) = (self.current_folder.as_ref(), self.selected_index) {
            self.folder_history.insert(folder.clone(), idx);
        }
        self.selected_index = None;
        self.current_folder = Some(folder.to_path_buf());

        let Ok(dir) = std::fs::read_dir(folder) else {
            return;
        };

        let mut paths: Vec<PathBuf> = dir
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.path())
            .filter(|p| Self::is_image(p))
            .collect();

        // Natural-ish sort by filename (case-insensitive).
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

        for (index, path) in paths.iter().enumerate() {
            let entry = self.build_entry(path.clone());
            self.store.append(&entry);
            self.path_to_index.insert(path.clone(), index as u32);
            self.all_known_paths.insert(path.clone());
        }
    }

    /// Insert `path` into the current folder's store in filename order.
    /// Returns the inserted or existing index, or `None` if the path is outside
    /// the active folder or isn't a supported image type.
    pub fn insert_path(&mut self, path: PathBuf) -> Option<u32> {
        if !Self::is_image(&path) {
            return None;
        }
        let current_folder = self.current_folder.as_ref()?;
        if path.parent() != Some(current_folder.as_path()) {
            return None;
        }
        if let Some(index) = self.path_to_index.get(&path).copied() {
            return Some(index);
        }

        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let insert_at = (0..self.store.n_items())
            .find(|&index| {
                self.entry_at(index)
                    .map(|entry| entry.filename().to_lowercase() > filename)
                    .unwrap_or(false)
            })
            .unwrap_or(self.store.n_items());

        let entry = self.build_entry(path.clone());
        self.store.insert(insert_at, &entry);
        self.reindex_from(insert_at);
        self.all_known_paths.insert(path.clone());
        Some(insert_at)
    }

    /// Returns the list index for `path`, or `None` if not in the current folder.
    pub fn index_of_path(&self, path: &Path) -> Option<u32> {
        self.path_to_index.get(path).copied()
    }

    /// Returns the `ImageEntry` at `index`, if present.
    pub fn entry_at(&self, index: u32) -> Option<ImageEntry> {
        self.store.item(index).and_then(|o| o.downcast().ok())
    }

    /// Returns the currently selected `ImageEntry`.
    pub fn selected_entry(&self) -> Option<ImageEntry> {
        self.selected_index.and_then(|i| self.entry_at(i))
    }

    pub fn restore_index_for(&self, folder: &Path) -> Option<u32> {
        self.folder_history.get(folder).copied()
    }

    /// Advance the selection by `delta` steps, wrapping at folder boundaries.
    /// Updates `selected_index` and returns the new index, or `None` if the
    /// store is empty.
    pub fn navigate(&mut self, delta: i32) -> Option<u32> {
        let count = self.store.n_items();
        if count == 0 {
            return None;
        }
        let current = self.selected_index.unwrap_or(0) as i32;
        let next = (current + delta).rem_euclid(count as i32) as u32;
        self.selected_index = Some(next);
        Some(next)
    }

    pub fn image_count(&self) -> u32 {
        self.store.n_items()
    }

    /// Remove the entry for `path` from the store and all internal maps.
    /// Reindexes entries that shifted down. Does nothing if not present.
    pub fn remove_path(&mut self, path: &Path) {
        let Some(index) = self.path_to_index.remove(path) else {
            return;
        };
        self.store.remove(index);
        self.thumbnail_cache.remove(path);
        if let Some(pos) = self.cache_order.iter().position(|p| p == path) {
            self.cache_order.remove(pos);
        }
        self.preview_cache.remove(path);
        if let Some(pos) = self.preview_order.iter().position(|p| p == path) {
            self.preview_order.remove(pos);
        }
        if self.selected_index == Some(index) {
            self.selected_index = None;
        }
        self.reindex_from(index);
    }

    // -----------------------------------------------------------------------
    // Thumbnail cache
    // -----------------------------------------------------------------------

    pub fn cached_thumbnail(&self, path: &Path) -> Option<Texture> {
        self.thumbnail_cache.get(path).cloned()
    }

    /// Insert into the LRU cache. No-op if already cached.
    pub fn insert_thumbnail(&mut self, path: PathBuf, texture: Texture) {
        if self.thumbnail_cache.contains_key(&path) {
            return;
        }
        while self.cache_order.len() >= self.thumbnail_cache_max {
            let oldest = self.cache_order.remove(0);
            self.thumbnail_cache.remove(&oldest);
        }
        self.thumbnail_cache.insert(path.clone(), texture);
        self.cache_order.push(path);
    }

    pub fn set_thumbnail_cache_max(&mut self, max_entries: usize) {
        self.thumbnail_cache_max = max_entries.max(100);
        while self.cache_order.len() > self.thumbnail_cache_max {
            let oldest = self.cache_order.remove(0);
            self.thumbnail_cache.remove(&oldest);
        }
    }

    /// Returns true if `path` is in the cache or currently being decoded.
    pub fn prefetch_pending(&self, path: &Path) -> bool {
        self.prefetch_cache.contains_key(path) || self.prefetch_in_flight.contains(path)
    }

    /// Mark a path as having a prefetch decode in flight.
    pub fn mark_prefetch_in_flight(&mut self, path: PathBuf) {
        self.prefetch_in_flight.insert(path);
    }

    /// Store completed prefetch bytes. Evicts oldest if cache exceeds 8 entries.
    pub fn insert_prefetch(&mut self, path: PathBuf, bytes: Vec<u8>, width: u32, height: u32) {
        self.prefetch_in_flight.remove(&path);
        if self.prefetch_cache.contains_key(&path) {
            return;
        }
        if self.prefetch_order.len() >= 8 {
            let oldest = self.prefetch_order.remove(0);
            self.prefetch_cache.remove(&oldest);
        }
        self.prefetch_cache
            .insert(path.clone(), (bytes, width, height));
        self.prefetch_order.push(path);
    }

    /// Consume pre-decoded bytes for `path`, removing from cache.
    /// Returns `None` if not cached.
    pub fn take_prefetch(&mut self, path: &Path) -> Option<(Vec<u8>, u32, u32)> {
        if let Some(pos) = self.prefetch_order.iter().position(|p| p == path) {
            self.prefetch_order.remove(pos);
        }
        self.prefetch_cache.remove(path)
    }

    /// Return a cloned decoded preview buffer, if present.
    pub fn cached_preview(&self, path: &Path) -> Option<(Vec<u8>, u32, u32)> {
        self.preview_cache.get(path).cloned()
    }

    /// Insert decoded preview bytes into the preview LRU cache.
    pub fn insert_preview(&mut self, path: PathBuf, bytes: Vec<u8>, width: u32, height: u32) {
        if self.preview_cache.contains_key(&path) {
            if let Some(pos) = self.preview_order.iter().position(|p| p == &path) {
                self.preview_order.remove(pos);
            }
        } else if self.preview_order.len() >= Self::MAX_PREVIEW_CACHE {
            let oldest = self.preview_order.remove(0);
            self.preview_cache.remove(&oldest);
        }

        self.preview_cache
            .insert(path.clone(), (bytes, width, height));
        self.preview_order.push(path);
    }

    /// Populate the store from an arbitrary list of paths (virtual view).
    /// Does not touch the filesystem or `current_folder`; sets it to `None`
    /// so callers can distinguish a virtual view from a real folder scan.
    pub fn load_virtual(&mut self, paths: &[PathBuf]) {
        self.store.remove_all();
        self.path_to_index.clear();
        self.hash_store.clear();
        self.prefetch_cache.clear();
        self.prefetch_order.clear();
        self.prefetch_in_flight.clear();
        self.preview_cache.clear();
        self.preview_order.clear();
        self.selected_index = None;
        self.current_folder = None;
        for (index, path) in paths.iter().enumerate() {
            let entry = self.build_entry(path.clone());
            self.store.append(&entry);
            self.path_to_index.insert(path.clone(), index as u32);
        }
    }

    pub fn insert_hash(&mut self, path: PathBuf, hash: u64) {
        self.hash_store.insert(path, hash);
    }

    /// Returns all (path, hash) pairs for the current folder, in store order.
    pub fn hashes_snapshot(&self) -> Vec<(PathBuf, u64)> {
        (0..self.store.n_items())
            .filter_map(|i| self.entry_at(i))
            .filter_map(|e| {
                let p = e.path();
                self.hash_store.get(&p).map(|&h| (p, h))
            })
            .collect()
    }

    /// Returns ALL accumulated (path, hash) pairs regardless of the current view.
    /// Use this for whole-library duplicate detection.
    pub fn all_hashes_snapshot(&self) -> Vec<(PathBuf, u64)> {
        self.hash_store
            .iter()
            .map(|(path, &hash)| (path.clone(), hash))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    pub fn find_duplicate_filenames(&self) -> Vec<PathBuf> {
        let mut groups: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for path in &self.all_known_paths {
            if let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_lowercase()) {
                groups.entry(name).or_default().push(path.clone());
            }
        }

        let mut duplicates = Vec::new();
        for mut paths in groups.into_values() {
            if paths.len() > 1 {
                paths.sort();
                duplicates.extend(paths);
            }
        }
        duplicates
    }

    fn is_image(path: &Path) -> bool {
        path.extension()
            .map(|ext| {
                let low = ext.to_string_lossy().to_lowercase();
                IMAGE_EXTENSIONS.contains(&low.as_str())
            })
            .unwrap_or(false)
    }

    fn build_entry(&self, path: PathBuf) -> ImageEntry {
        let entry = ImageEntry::new(path.clone());
        if let Ok(meta) = std::fs::metadata(&path) {
            entry.set_file_size(meta.len());
        }
        if let Ok((width, height)) = image::image_dimensions(&path) {
            entry.set_dimensions(width, height);
        }
        if let Some(texture) = self.thumbnail_cache.get(&path) {
            entry.set_thumbnail(Some(texture.clone()));
        }
        entry
    }

    fn reindex_from(&mut self, start: u32) {
        for index in start..self.store.n_items() {
            if let Some(entry) = self.entry_at(index) {
                self.path_to_index.insert(entry.path(), index);
            }
        }
    }
}

impl Default for LibraryManager {
    fn default() -> Self {
        Self::new()
    }
}
