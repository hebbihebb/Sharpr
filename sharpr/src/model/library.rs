use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use gdk4::Texture;
use gio::prelude::*;
use rustc_hash::FxHashMap;

use crate::library_index::{basic_info_from_path, BasicImageInfo, IndexedImage};
use crate::model::image_entry::ImageEntry;
use crate::quality::{QualityClass, QualityScore};

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum SortOrder {
    #[default]
    Name,
    DateModified,
    FileType,
}

/// Known image file extensions (lower-case).
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "tiff", "tif", "bmp", "ico", "avif", "heic", "heif",
];

#[derive(Clone)]
pub struct CachedImageData {
    pub file_size: u64,
    pub dimensions: Option<(u32, u32)>,
    pub quality: QualityScore,
}

pub struct RawImageEntry {
    pub path: PathBuf,
    pub filename: String,
    pub file_size: u64,
    pub modified: Option<std::time::SystemTime>,
    pub width: u32,
    pub height: u32,
}

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
    folder_history: FxHashMap<PathBuf, u32>,
    /// O(1) path → list index lookup, kept in sync with `store`.
    pub(crate) path_to_index: FxHashMap<PathBuf, u32>,
    /// Set of all image paths encountered during the session, for cross-folder duplicates.
    pub(crate) all_known_paths: HashSet<PathBuf>,
    hash_store: FxHashMap<PathBuf, u64>,
    active_thumbnail_cache: FxHashMap<PathBuf, Texture>,
    thumbnail_cache: FxHashMap<PathBuf, Texture>,
    /// Insertion order for LRU eviction.
    cache_order: Vec<PathBuf>,
    prefetch_cache: FxHashMap<PathBuf, (Vec<u8>, u32, u32)>,
    prefetch_order: Vec<PathBuf>,
    prefetch_cache_bytes: usize,
    prefetch_in_flight: HashSet<PathBuf>,
    preview_cache: FxHashMap<PathBuf, (Vec<u8>, u32, u32)>,
    preview_order: Vec<PathBuf>,
    preview_cache_bytes: usize,
    thumbnail_cache_max: usize,
    metadata_cache: FxHashMap<PathBuf, CachedImageData>,
    indexed_library_paths: Vec<PathBuf>,
}

impl LibraryManager {
    /// Byte budget for the decoded-preview LRU cache (~128 MiB).
    const PREVIEW_CACHE_BUDGET: usize = 128 * 1024 * 1024;
    /// Byte budget for the prefetch LRU cache (~64 MiB).
    const PREFETCH_CACHE_BUDGET: usize = 64 * 1024 * 1024;

    pub fn new() -> Self {
        Self {
            store: gio::ListStore::new::<ImageEntry>(),
            current_folder: None,
            selected_index: None,
            folder_history: FxHashMap::default(),
            path_to_index: FxHashMap::default(),
            all_known_paths: HashSet::new(),
            hash_store: FxHashMap::default(),
            active_thumbnail_cache: FxHashMap::default(),
            thumbnail_cache: FxHashMap::default(),
            cache_order: Vec::new(),
            prefetch_cache: FxHashMap::default(),
            prefetch_order: Vec::new(),
            prefetch_cache_bytes: 0,
            prefetch_in_flight: HashSet::new(),
            preview_cache: FxHashMap::default(),
            preview_order: Vec::new(),
            preview_cache_bytes: 0,
            thumbnail_cache_max: 500,
            metadata_cache: FxHashMap::default(),
            indexed_library_paths: Vec::new(),
        }
    }

    /// Scan `folder` for images and populate the store.
    /// Clears all existing entries and rebuilds the index map.
    pub fn scan_folder(&mut self, folder: &Path) {
        self.reset_for_folder(folder);

        for (index, raw) in Self::scan_folder_raw(folder).into_iter().enumerate() {
            let entry = ImageEntry::new(raw.path.clone());
            entry.set_file_size(raw.file_size);
            entry.set_dimensions(raw.width, raw.height);
            self.store.append(&entry);
            self.path_to_index.insert(raw.path.clone(), index as u32);
            self.all_known_paths.insert(raw.path);
        }
    }

    pub fn reset_for_folder(&mut self, folder: &Path) {
        self.store.remove_all();
        self.path_to_index.clear();
        self.hash_store.clear();
        self.active_thumbnail_cache.clear();
        self.prefetch_cache.clear();
        self.prefetch_order.clear();
        self.prefetch_cache_bytes = 0;
        self.prefetch_in_flight.clear();
        if let (Some(folder), Some(idx)) = (self.current_folder.as_ref(), self.selected_index) {
            self.folder_history.insert(folder.clone(), idx);
        }
        self.selected_index = None;
        self.current_folder = Some(folder.to_path_buf());
    }

    pub fn scan_folder_raw(folder: &Path) -> Vec<RawImageEntry> {
        let Ok(dir) = std::fs::read_dir(folder) else {
            return Vec::new();
        };

        let mut paths: Vec<PathBuf> = dir
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|entry| entry.path())
            .filter(|path| Self::is_image(path))
            .collect();

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

        paths
            .into_iter()
            .map(|path| {
                let filename = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let meta = std::fs::metadata(&path).ok();
                let file_size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified = meta.and_then(|m| m.modified().ok());

                RawImageEntry {
                    path,
                    filename,
                    file_size,
                    modified,
                    width: 0,
                    height: 0,
                }
            })
            .collect()
    }

    pub fn scan_folder_basic(folder: &Path) -> Vec<BasicImageInfo> {
        let Ok(dir) = std::fs::read_dir(folder) else {
            return Vec::new();
        };

        let mut paths: Vec<PathBuf> = dir
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|entry| entry.path())
            .filter(|path| Self::is_image(path))
            .collect();

        paths.sort_by(|a, b| path_sort_key(a, b));
        paths
            .into_iter()
            .map(|path| basic_info_from_path(folder, path))
            .collect()
    }

    pub fn load_indexed_folder(&mut self, folder: &Path, rows: Vec<IndexedImage>) {
        self.reset_for_folder(folder);
        let mut new_entries = Vec::with_capacity(rows.len());
        for (index, row) in rows.into_iter().enumerate() {
            let entry = ImageEntry::new(row.path.clone());
            entry.set_file_size(row.file_size);
            if let (Some(width), Some(height)) = (row.width, row.height) {
                entry.set_dimensions(width, height);
            }
            if let Some(texture) = self.cached_thumbnail(&row.path) {
                entry.set_thumbnail(Some(texture));
            }
            self.path_to_index.insert(row.path.clone(), index as u32);
            self.all_known_paths.insert(row.path.clone());
            if !self.indexed_library_paths.contains(&row.path) {
                self.indexed_library_paths.push(row.path.clone());
            }
            new_entries.push(entry);
        }
        self.store.splice(0, 0, &new_entries);
        self.indexed_library_paths
            .sort_by(|a, b| path_sort_key(a, b));
    }

    pub fn update_entry_metadata(&mut self, path: &Path, width: u32, height: u32) {
        self.metadata_cache.remove(path);
        if let Some(index) = self.index_of_path(path) {
            if let Some(entry) = self.entry_at(index) {
                entry.set_dimensions(width, height);
            }
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

        let entry = self.build_entry(path.clone(), true);
        self.store.insert(insert_at, &entry);
        self.reindex_from(insert_at);
        self.all_known_paths.insert(path.clone());
        if !self.indexed_library_paths.contains(&path) {
            self.indexed_library_paths.push(path.clone());
            self.indexed_library_paths
                .sort_by(|a, b| path_sort_key(a, b));
        }
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
        self.active_thumbnail_cache.remove(path);
        self.thumbnail_cache.remove(path);
        if let Some(pos) = self.cache_order.iter().position(|p| p == path) {
            self.cache_order.remove(pos);
        }
        if let Some((bytes, w, h)) = self.preview_cache.remove(path) {
            self.preview_cache_bytes = self
                .preview_cache_bytes
                .saturating_sub(Self::entry_bytes(w, h));
            drop(bytes);
        }
        if let Some(pos) = self.preview_order.iter().position(|p| p == path) {
            self.preview_order.remove(pos);
        }
        self.metadata_cache.remove(path);
        self.indexed_library_paths.retain(|p| p != path);
        if self.selected_index == Some(index) {
            self.selected_index = None;
        }
        self.reindex_from(index);
    }

    /// Drop all cached decode state for `path` so the next load uses fresh bytes.
    pub fn invalidate_path_caches(&mut self, path: &Path) {
        self.active_thumbnail_cache.remove(path);
        self.thumbnail_cache.remove(path);
        if let Some(pos) = self.cache_order.iter().position(|p| p == path) {
            self.cache_order.remove(pos);
        }

        if let Some((bytes, w, h)) = self.preview_cache.remove(path) {
            self.preview_cache_bytes = self
                .preview_cache_bytes
                .saturating_sub(Self::entry_bytes(w, h));
            drop(bytes);
        }
        if let Some(pos) = self.preview_order.iter().position(|p| p == path) {
            self.preview_order.remove(pos);
        }

        if let Some((bytes, w, h)) = self.prefetch_cache.remove(path) {
            self.prefetch_cache_bytes = self
                .prefetch_cache_bytes
                .saturating_sub(Self::entry_bytes(w, h));
            drop(bytes);
        }
        if let Some(pos) = self.prefetch_order.iter().position(|p| p == path) {
            self.prefetch_order.remove(pos);
        }
        self.prefetch_in_flight.remove(path);
        self.metadata_cache.remove(path);

        if let Some(index) = self.index_of_path(path) {
            if let Some(entry) = self.entry_at(index) {
                entry.set_thumbnail(None::<Texture>);
                let refreshed = self.cached_image_data(path, true);
                entry.set_file_size(refreshed.file_size);
                if let Some((width, height)) = refreshed.dimensions {
                    entry.set_dimensions(width, height);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Thumbnail cache
    // -----------------------------------------------------------------------

    pub fn cached_thumbnail(&self, path: &Path) -> Option<Texture> {
        if let Some(texture) = self.active_thumbnail_cache.get(path) {
            crate::bench_event!(
                "thumbnail.cache_hit",
                serde_json::json!({
                    "path": path.display().to_string(),
                    "cache": "active_view",
                }),
            );
            return Some(texture.clone());
        }
        if let Some(texture) = self.thumbnail_cache.get(path) {
            crate::bench_event!(
                "thumbnail.cache_hit",
                serde_json::json!({
                    "path": path.display().to_string(),
                    "cache": "global_lru",
                }),
            );
            return Some(texture.clone());
        }
        None
    }

    /// Insert into the active view cache and the bounded global LRU cache.
    pub fn insert_thumbnail(&mut self, path: PathBuf, texture: Texture, retain_active: bool) {
        if retain_active {
            self.active_thumbnail_cache
                .insert(path.clone(), texture.clone());
        }
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

    pub fn thumbnail_cache_stats(&self) -> (usize, usize, usize) {
        (
            self.active_thumbnail_cache.len(),
            self.thumbnail_cache.len(),
            self.thumbnail_cache_max,
        )
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

    /// Byte footprint of one RGBA cache entry.
    fn entry_bytes(width: u32, height: u32) -> usize {
        width as usize * height as usize * 4
    }

    /// Mark a path as having a prefetch decode in flight.
    pub fn mark_prefetch_in_flight(&mut self, path: PathBuf) {
        self.prefetch_in_flight.insert(path);
    }

    /// Store completed prefetch bytes. Evicts LRU entries to stay within the
    /// byte budget. Oversized single images are not cached but still decode fine.
    pub fn insert_prefetch(&mut self, path: PathBuf, bytes: Vec<u8>, width: u32, height: u32) {
        self.prefetch_in_flight.remove(&path);
        if self.prefetch_cache.contains_key(&path) {
            return;
        }
        let entry_size = Self::entry_bytes(width, height);
        if entry_size > Self::PREFETCH_CACHE_BUDGET {
            return;
        }
        while self.prefetch_cache_bytes + entry_size > Self::PREFETCH_CACHE_BUDGET {
            let Some(oldest) = self.prefetch_order.first().cloned() else {
                break;
            };
            self.prefetch_order.remove(0);
            if let Some((evicted, ew, eh)) = self.prefetch_cache.remove(&oldest) {
                self.prefetch_cache_bytes = self
                    .prefetch_cache_bytes
                    .saturating_sub(Self::entry_bytes(ew, eh));
                drop(evicted);
            }
        }
        self.prefetch_cache_bytes += entry_size;
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
        if let Some((bytes, w, h)) = self.prefetch_cache.remove(path) {
            self.prefetch_cache_bytes = self
                .prefetch_cache_bytes
                .saturating_sub(Self::entry_bytes(w, h));
            return Some((bytes, w, h));
        }
        None
    }

    /// Return a cloned decoded preview buffer, if present.
    pub fn cached_preview(&self, path: &Path) -> Option<(Vec<u8>, u32, u32)> {
        self.preview_cache.get(path).cloned()
    }

    /// Insert decoded preview bytes into the preview LRU cache.
    /// Evicts LRU entries to stay within the byte budget.
    /// Oversized single images are not cached (they still display as the current image).
    pub fn insert_preview(&mut self, path: PathBuf, bytes: Vec<u8>, width: u32, height: u32) {
        let entry_size = Self::entry_bytes(width, height);
        if entry_size > Self::PREVIEW_CACHE_BUDGET {
            return;
        }
        // Remove existing entry first so its bytes are freed before re-inserting.
        if let Some((old_bytes, ow, oh)) = self.preview_cache.remove(&path) {
            self.preview_cache_bytes = self
                .preview_cache_bytes
                .saturating_sub(Self::entry_bytes(ow, oh));
            if let Some(pos) = self.preview_order.iter().position(|p| p == &path) {
                self.preview_order.remove(pos);
            }
            drop(old_bytes);
        }
        while self.preview_cache_bytes + entry_size > Self::PREVIEW_CACHE_BUDGET {
            let Some(oldest) = self.preview_order.first().cloned() else {
                break;
            };
            self.preview_order.remove(0);
            if let Some((evicted, ew, eh)) = self.preview_cache.remove(&oldest) {
                self.preview_cache_bytes = self
                    .preview_cache_bytes
                    .saturating_sub(Self::entry_bytes(ew, eh));
                drop(evicted);
            }
        }
        self.preview_cache_bytes += entry_size;
        self.preview_cache
            .insert(path.clone(), (bytes, width, height));
        self.preview_order.push(path);
    }

    /// Approximate bytes used by the preview and prefetch caches (for debugging).
    pub fn cache_stats(&self) -> (usize, usize) {
        (self.preview_cache_bytes, self.prefetch_cache_bytes)
    }

    /// Populate the store from an arbitrary list of paths (virtual view).
    /// Does not touch the filesystem or `current_folder`; sets it to `None`
    /// so callers can distinguish a virtual view from a real folder scan.
    /// Returns paths whose metadata was not already cached so callers can
    /// hydrate them asynchronously.
    pub fn load_virtual(&mut self, paths: &[PathBuf]) -> Vec<PathBuf> {
        self.store.remove_all();
        self.path_to_index.clear();
        self.active_thumbnail_cache.clear();
        self.prefetch_cache.clear();
        self.prefetch_order.clear();
        self.prefetch_cache_bytes = 0;
        self.prefetch_in_flight.clear();
        self.preview_cache.clear();
        self.preview_order.clear();
        self.preview_cache_bytes = 0;
        self.selected_index = None;
        self.current_folder = None;
        let mut metadata_pending = Vec::new();
        for (index, path) in paths.iter().enumerate() {
            let entry = ImageEntry::new(path.clone());
            if let Some(cached) = self.metadata_cache.get(path) {
                entry.set_file_size(cached.file_size);
                if let Some((width, height)) = cached.dimensions {
                    entry.set_dimensions(width, height);
                }
            } else {
                metadata_pending.push(path.clone());
            }
            if let Some(texture) = self.thumbnail_cache.get(path) {
                entry.set_thumbnail(Some(texture.clone()));
            }
            self.store.append(&entry);
            self.path_to_index.insert(path.clone(), index as u32);
        }
        metadata_pending
    }

    pub fn apply_cached_image_data(&mut self, path: &Path, cached: CachedImageData) {
        self.metadata_cache
            .insert(path.to_path_buf(), cached.clone());
        if let Some(index) = self.path_to_index.get(path).copied() {
            if let Some(entry) = self.entry_at(index) {
                entry.set_file_size(cached.file_size);
                if let Some((width, height)) = cached.dimensions {
                    entry.set_dimensions(width, height);
                }
            }
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

    pub fn bg_scan_quality_prep(
        &self,
    ) -> (
        Vec<PathBuf>,
        Option<PathBuf>,
        FxHashMap<PathBuf, CachedImageData>,
    ) {
        (
            self.indexed_library_paths.clone(),
            self.current_folder.clone(),
            self.metadata_cache.clone(),
        )
    }

    pub fn bg_scan_quality_finish(
        &mut self,
        paths: Vec<PathBuf>,
        cache: FxHashMap<PathBuf, CachedImageData>,
    ) {
        self.indexed_library_paths = paths;
        self.metadata_cache = cache;
    }

    pub fn compute_paths_for_quality_class(
        library_root: Option<PathBuf>,
        class: QualityClass,
        mut indexed_paths: Vec<PathBuf>,
        current_folder: Option<PathBuf>,
        mut metadata_cache: FxHashMap<PathBuf, CachedImageData>,
        disabled_folders: Vec<PathBuf>,
    ) -> (
        Vec<PathBuf>,
        FxHashMap<PathBuf, CachedImageData>,
        Vec<PathBuf>,
    ) {
        let paths =
            collect_library_image_paths(library_root, current_folder.as_deref(), &disabled_folders);
        if paths != indexed_paths {
            for path in &paths {
                Self::cached_image_data_static(path, &mut metadata_cache);
            }
            indexed_paths = paths;
        }

        let filtered = indexed_paths
            .iter()
            .filter(|path| {
                metadata_cache
                    .get(*path)
                    .map(|cached| cached.quality.class == class)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        (indexed_paths, metadata_cache, filtered)
    }

    pub fn cached_image_data_static(
        path: &Path,
        cache: &mut FxHashMap<PathBuf, CachedImageData>,
    ) -> CachedImageData {
        if let Some(cached) = cache.get(path) {
            return cached.clone();
        }

        let file_size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        let dimensions = image::image_dimensions(path).ok();
        let format = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        let cached = CachedImageData {
            file_size,
            dimensions,
            quality: crate::quality::scorer::score_file_info(dimensions, file_size, format),
        };
        cache.insert(path.to_path_buf(), cached.clone());
        cached
    }

    fn is_image(path: &Path) -> bool {
        path.extension()
            .map(|ext| {
                let low = ext.to_string_lossy().to_lowercase();
                IMAGE_EXTENSIONS.contains(&low.as_str())
            })
            .unwrap_or(false)
    }

    fn build_entry(&mut self, path: PathBuf, refresh: bool) -> ImageEntry {
        let entry = ImageEntry::new(path.clone());
        let cached = self.cached_image_data(&path, refresh);
        entry.set_file_size(cached.file_size);
        if let Some((width, height)) = cached.dimensions {
            entry.set_dimensions(width, height);
        }
        if let Some(texture) = self.thumbnail_cache.get(&path) {
            entry.set_thumbnail(Some(texture.clone()));
        }
        entry
    }

    fn cached_image_data(&mut self, path: &Path, refresh: bool) -> CachedImageData {
        if refresh {
            self.metadata_cache.remove(path);
        }
        Self::cached_image_data_static(path, &mut self.metadata_cache)
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

fn collect_library_image_paths(
    library_root: Option<PathBuf>,
    current_folder: Option<&Path>,
    disabled_folders: &[PathBuf],
) -> Vec<PathBuf> {
    let mut folders = collect_library_folders(library_root);
    if let Some(current_folder) = current_folder {
        if current_folder.is_dir()
            && !path_is_disabled(current_folder, disabled_folders)
            && !folders.iter().any(|folder| folder == current_folder)
        {
            folders.push(current_folder.to_path_buf());
        }
    }

    let mut paths = Vec::new();
    for folder in folders {
        if path_is_disabled(&folder, disabled_folders) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };

        let mut folder_paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|entry| entry.path())
            .filter(|path| LibraryManager::is_image(path))
            .collect();
        folder_paths.sort_by(|a, b| path_sort_key(a, b));
        paths.extend(folder_paths);
    }

    paths.sort_by(|a, b| path_sort_key(a, b));
    paths.dedup();
    paths
}

fn path_is_disabled(path: &Path, disabled_folders: &[PathBuf]) -> bool {
    disabled_folders
        .iter()
        .any(|folder| path.starts_with(folder))
}

fn collect_library_folders(library_root: Option<PathBuf>) -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home".into());
    let home = PathBuf::from(home);
    let roots = if let Some(root) = library_root {
        vec![root]
    } else {
        vec![home.join("Pictures"), home.join("Downloads"), home]
    };

    let mut folders = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }

        if directory_contains_images(&root) && seen.insert(root.clone()) {
            folders.push(root.clone());
        }

        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }

            let path = entry.path();
            if directory_contains_images(&path) && seen.insert(path.clone()) {
                folders.push(path);
            }
        }
    }

    folders.sort_by(|a, b| path_sort_key(a, b));
    folders
}

fn directory_contains_images(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };

    entries.filter_map(Result::ok).any(|entry| {
        entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            && LibraryManager::is_image(&entry.path())
    })
}

fn path_sort_key(a: &Path, b: &Path) -> std::cmp::Ordering {
    a.to_string_lossy()
        .to_lowercase()
        .cmp(&b.to_string_lossy().to_lowercase())
}

pub fn sort_raw_entries(entries: &mut [RawImageEntry], order: SortOrder) {
    match order {
        SortOrder::Name => {}
        SortOrder::DateModified => entries.sort_by(|a, b| b.modified.cmp(&a.modified)),
        SortOrder::FileType => entries.sort_by(|a, b| {
            let ext = |e: &RawImageEntry| {
                e.path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase()
            };
            ext(a)
                .cmp(&ext(b))
                .then_with(|| a.filename.to_lowercase().cmp(&b.filename.to_lowercase()))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppSettings;
    use image::codecs::jpeg::JpegEncoder;
    use image::{ImageBuffer, ImageFormat, Rgb};

    fn temp_path(name: &str) -> PathBuf {
        let unique = format!(
            "sharpr-library-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique).join(name)
    }

    fn write_bmp(path: &Path, width: u32, height: u32) {
        let image = ImageBuffer::from_pixel(width, height, Rgb([120u8, 90u8, 40u8]));
        image.save_with_format(path, ImageFormat::Bmp).unwrap();
    }

    fn write_jpeg(path: &Path, width: u32, height: u32, quality: u8) {
        let image = ImageBuffer::from_pixel(width, height, Rgb([10u8, 10u8, 10u8]));
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = JpegEncoder::new_with_quality(file, quality);
        encoder
            .encode_image(&image::DynamicImage::ImageRgb8(image))
            .unwrap();
    }

    #[test]
    fn load_virtual_preserves_session_hashes_for_duplicates() {
        let path = PathBuf::from("/photos/a.jpg");
        let mut library = LibraryManager::new();
        library.insert_hash(path.clone(), 0xabcd);

        let pending = library.load_virtual(&[path.clone()]);

        assert_eq!(pending, vec![path.clone()]);
        assert_eq!(library.all_hashes_snapshot(), vec![(path, 0xabcd)]);
    }

    #[test]
    fn quality_paths_are_global_across_library_folders() {
        let root = temp_path("root");
        let folder_a = root.join("A");
        let folder_b = root.join("B");
        std::fs::create_dir_all(&folder_a).unwrap();
        std::fs::create_dir_all(&folder_b).unwrap();

        let excellent = folder_b.join("excellent.bmp");
        let needs_upscale = folder_a.join("small.jpg");
        write_bmp(&excellent, 3840, 2160);
        write_jpeg(&needs_upscale, 960, 640, 8);

        let mut settings = AppSettings::default();
        settings.library_root = Some(root.clone());
        let mut library = LibraryManager::new();
        library.scan_folder(&folder_a);

        let (indexed_paths, current_folder, metadata_cache) = library.bg_scan_quality_prep();
        let (_, _, excellent_paths) = LibraryManager::compute_paths_for_quality_class(
            settings.library_root.clone(),
            QualityClass::Excellent,
            indexed_paths.clone(),
            current_folder.clone(),
            metadata_cache.clone(),
            Vec::new(),
        );
        let (_, _, upscale_paths) = LibraryManager::compute_paths_for_quality_class(
            settings.library_root.clone(),
            QualityClass::NeedsUpscale,
            indexed_paths,
            current_folder,
            metadata_cache,
            Vec::new(),
        );

        assert_eq!(excellent_paths, vec![excellent]);
        assert_eq!(upscale_paths, vec![needs_upscale]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scan_folder_raw_returns_sorted_plain_entries_without_dimensions() {
        let root = temp_path("raw-scan");
        std::fs::create_dir_all(&root).unwrap();

        let img_a = root.join("a.JPG");
        let img_b = root.join("B.bmp");
        let txt = root.join("note.txt");

        write_jpeg(&img_a, 640, 480, 80);
        write_bmp(&img_b, 320, 200);
        std::fs::write(&txt, b"not an image").unwrap();

        let entries = LibraryManager::scan_folder_raw(&root);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, img_a);
        assert_eq!(entries[0].filename, "a.JPG");
        assert_eq!(entries[0].width, 0);
        assert_eq!(entries[0].height, 0);
        assert!(entries[0].file_size > 0);

        assert_eq!(entries[1].path, img_b);
        assert_eq!(entries[1].filename, "B.bmp");
        assert_eq!(entries[1].width, 0);
        assert_eq!(entries[1].height, 0);
        assert!(entries[1].file_size > 0);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preview_cache_evicts_lru_when_budget_exceeded() {
        let mut mgr = LibraryManager::new();
        // Use 1-byte-per-pixel stand-ins: 4000×4000 = 64 MiB each, budget 128 MiB.
        let a = PathBuf::from("/a.jpg");
        let b = PathBuf::from("/b.jpg");
        let c = PathBuf::from("/c.jpg");
        let bytes_64mib = vec![0u8; 4000 * 4000 * 4];
        mgr.insert_preview(a.clone(), bytes_64mib.clone(), 4000, 4000);
        mgr.insert_preview(b.clone(), bytes_64mib.clone(), 4000, 4000);
        let (used, _) = mgr.cache_stats();
        assert_eq!(used, 2 * 4000 * 4000 * 4);
        // Adding c (64 MiB) would exceed 128 MiB budget — evicts a (oldest).
        mgr.insert_preview(c.clone(), bytes_64mib.clone(), 4000, 4000);
        assert!(
            mgr.cached_preview(&a).is_none(),
            "a should have been evicted"
        );
        assert!(mgr.cached_preview(&b).is_some());
        assert!(mgr.cached_preview(&c).is_some());
        let (used, _) = mgr.cache_stats();
        assert_eq!(used, 2 * 4000 * 4000 * 4);
    }

    #[test]
    fn oversized_preview_not_cached() {
        let mut mgr = LibraryManager::new();
        // 8000×8000 RGBA = 256 MiB > budget; should be silently dropped.
        let path = PathBuf::from("/huge.tif");
        let oversized = vec![0u8; 8000 * 8000 * 4];
        mgr.insert_preview(path.clone(), oversized, 8000, 8000);
        assert!(mgr.cached_preview(&path).is_none());
        let (used, _) = mgr.cache_stats();
        assert_eq!(used, 0);
    }

    #[test]
    fn prefetch_cache_evicts_lru_when_budget_exceeded() {
        let mut mgr = LibraryManager::new();
        // 32 MiB each, budget 64 MiB.
        let a = PathBuf::from("/a.jpg");
        let b = PathBuf::from("/b.jpg");
        let c = PathBuf::from("/c.jpg");
        let bytes_32mib = vec![0u8; 2828 * 2828 * 4]; // ≈32 MiB
        let (w, h) = (2828u32, 2828u32);
        mgr.insert_prefetch(a.clone(), bytes_32mib.clone(), w, h);
        mgr.insert_prefetch(b.clone(), bytes_32mib.clone(), w, h);
        // c should evict a.
        mgr.insert_prefetch(c.clone(), bytes_32mib.clone(), w, h);
        assert!(
            mgr.take_prefetch(&a).is_none(),
            "a should have been evicted"
        );
        assert!(mgr.take_prefetch(&b).is_some());
        assert!(mgr.take_prefetch(&c).is_some());
    }

    #[test]
    fn take_prefetch_decrements_byte_count() {
        let mut mgr = LibraryManager::new();
        let path = PathBuf::from("/img.jpg");
        mgr.insert_prefetch(path.clone(), vec![0u8; 100 * 100 * 4], 100, 100);
        let (_, pre) = mgr.cache_stats();
        assert_eq!(pre, 100 * 100 * 4);
        let _ = mgr.take_prefetch(&path);
        let (_, post) = mgr.cache_stats();
        assert_eq!(post, 0);
    }
}
