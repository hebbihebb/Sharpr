use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use gdk4::Texture;
use gio::prelude::*;
use rustc_hash::FxHashMap;

use crate::config::FolderMode;
use crate::library_index::{basic_info_from_path, BasicImageInfo, IndexedImage};
use crate::model::image_entry::ImageEntry;
use crate::model::walk;
use crate::quality::QualityScore;

/// Process-wide, so a stale virtual load never matches a newer one.
fn next_virtual_generation() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortField {
    Name,
    DateModified,
    FileType,
    Aspect,
    WideScreen,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum SortOrder {
    NameAsc,
    NameDesc,
    DateModifiedAsc,
    #[default]
    DateModifiedDesc,
    FileTypeAsc,
    FileTypeDesc,
    AspectAsc,
    AspectDesc,
    WideScreenAsc,
    WideScreenDesc,
}

impl SortOrder {
    pub fn field(self) -> SortField {
        match self {
            Self::NameAsc | Self::NameDesc => SortField::Name,
            Self::DateModifiedAsc | Self::DateModifiedDesc => SortField::DateModified,
            Self::FileTypeAsc | Self::FileTypeDesc => SortField::FileType,
            Self::AspectAsc | Self::AspectDesc => SortField::Aspect,
            Self::WideScreenAsc | Self::WideScreenDesc => SortField::WideScreen,
        }
    }

    pub fn descending(self) -> bool {
        matches!(
            self,
            Self::NameDesc
                | Self::DateModifiedDesc
                | Self::FileTypeDesc
                | Self::AspectDesc
                | Self::WideScreenDesc
        )
    }

    pub fn from_parts(field: SortField, descending: bool) -> Self {
        match (field, descending) {
            (SortField::Name, false) => Self::NameAsc,
            (SortField::Name, true) => Self::NameDesc,
            (SortField::DateModified, false) => Self::DateModifiedAsc,
            (SortField::DateModified, true) => Self::DateModifiedDesc,
            (SortField::FileType, false) => Self::FileTypeAsc,
            (SortField::FileType, true) => Self::FileTypeDesc,
            (SortField::Aspect, false) => Self::AspectAsc,
            (SortField::Aspect, true) => Self::AspectDesc,
            (SortField::WideScreen, false) => Self::WideScreenAsc,
            (SortField::WideScreen, true) => Self::WideScreenDesc,
        }
    }
}

/// Known image file extensions (lower-case).
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jxl", "png", "gif", "webp", "tiff", "tif", "bmp", "ico", "avif", "heic", "heif",
];

#[derive(Clone)]
pub struct CachedImageData {
    pub file_size: u64,
    pub dimensions: Option<(u32, u32)>,
    pub quality: QualityScore,
}

#[derive(Clone)]
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
    pub virtual_generation: u64,
    /// Identifies the load that last filled the store; see `is_current_load`.
    load_token: u64,
    /// True from the moment a folder load empties the store until it has
    /// filled it; rows added meanwhile could be listed twice.
    folder_loading: bool,
    folder_history: FxHashMap<PathBuf, u32>,
    /// O(1) path → list index lookup, kept in sync with `store`.
    pub(crate) path_to_index: FxHashMap<PathBuf, u32>,
    /// True when the current virtual view lists some path more than once,
    /// which `path_to_index` cannot represent.
    virtual_has_repeated_paths: bool,
    /// Set of all image paths encountered during the session, for cross-folder duplicates.
    pub(crate) all_known_paths: HashSet<PathBuf>,
    pub(crate) removed_paths: HashSet<PathBuf>,
    active_thumbnail_cache: FxHashMap<PathBuf, Texture>,
    thumbnail_cache: FxHashMap<PathBuf, Texture>,
    /// Insertion order for LRU eviction.
    cache_order: Vec<PathBuf>,
    thumbnail_cache_max: usize,
    metadata_cache: FxHashMap<PathBuf, CachedImageData>,
    indexed_library_paths: Vec<PathBuf>,
}

impl LibraryManager {
    pub fn new() -> Self {
        Self {
            store: gio::ListStore::new::<ImageEntry>(),
            current_folder: None,
            selected_index: None,
            virtual_generation: 0,
            load_token: 0,
            folder_loading: false,
            folder_history: FxHashMap::default(),
            path_to_index: FxHashMap::default(),
            virtual_has_repeated_paths: false,
            all_known_paths: HashSet::new(),
            removed_paths: HashSet::new(),
            active_thumbnail_cache: FxHashMap::default(),
            thumbnail_cache: FxHashMap::default(),
            cache_order: Vec::new(),
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

    /// Token of the load that currently owns the store.
    pub fn load_token(&self) -> u64 {
        self.load_token
    }

    /// True while `token`, taken from `load_token()` when a load began, still
    /// owns the store and `folder` is the open folder.
    pub fn is_current_load(&self, folder: &Path, token: u64) -> bool {
        self.current_folder.as_deref() == Some(folder) && self.load_token == token
    }

    /// True while a folder load is still filling the store.
    pub fn is_loading_folder(&self) -> bool {
        self.folder_loading
    }

    /// Empties the store for `folder` and retires every load already in flight.
    pub fn reset_for_folder(&mut self, folder: &Path) {
        self.load_token = self.load_token.wrapping_add(1);
        self.clear_for_folder(folder);
    }

    fn clear_for_folder(&mut self, folder: &Path) {
        self.folder_loading = true;
        self.removed_paths.clear();
        self.store.remove_all();
        self.path_to_index.clear();
        self.active_thumbnail_cache.clear();
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

    /// Start loading a folder by clearing the store and setting up metadata.
    /// Follow with calls to `append_indexed_rows` to populate the store.
    /// Keeps the load token: the caller already holds the token of this load.
    pub fn start_load_folder(&mut self, folder: &Path) {
        self.clear_for_folder(folder);
    }

    /// Append a chunk of indexed rows to the store.
    /// This should be called on the main thread, ideally in chunks to keep the UI responsive.
    pub fn append_indexed_rows(&mut self, rows: Vec<IndexedImage>) {
        let mut new_entries = Vec::with_capacity(rows.len());
        let mut seen_paths: HashSet<PathBuf> = self.indexed_library_paths.iter().cloned().collect();
        let start_index = self.store.n_items();

        for (i, row) in rows.into_iter().enumerate() {
            let index = start_index + i as u32;
            let entry = self.entry_from_row(&row);
            self.path_to_index.insert(row.path.clone(), index);
            self.all_known_paths.insert(row.path.clone());

            if seen_paths.insert(row.path.clone()) {
                self.indexed_library_paths.push(row.path.clone());
            }
            new_entries.push(entry);
        }
        self.store.splice(start_index, 0, &new_entries);
    }

    fn entry_from_row(&self, row: &IndexedImage) -> ImageEntry {
        let entry = ImageEntry::new(row.path.clone());
        entry.set_file_size(row.file_size);
        entry.set_modified_secs(row.modified_secs);
        if let (Some(width), Some(height)) = (row.width, row.height) {
            entry.set_dimensions(width, height);
        }
        if let Some(texture) = self.cached_thumbnail(&row.path) {
            entry.set_thumbnail(Some(texture));
        }
        entry
    }

    /// Finalize folder loading by sorting the indexed paths.
    pub fn finish_load_folder(&mut self) {
        self.folder_loading = false;
        self.indexed_library_paths
            .sort_by(|a, b| path_sort_key(a, b));
    }

    pub fn load_indexed_folder(&mut self, folder: &Path, rows: Vec<IndexedImage>) {
        self.start_load_folder(folder);
        self.append_indexed_rows(rows);
        self.finish_load_folder();
    }

    pub fn update_entry_metadata(&mut self, path: &Path, width: u32, height: u32) {
        self.metadata_cache.remove(path);
        if let Some(index) = self.index_of_path(path) {
            if let Some(entry) = self.entry_at(index) {
                entry.set_dimensions(width, height);
            }
        }
    }

    /// Reads what the filmstrip shows of a file made in `folder` after it was
    /// listed: size, modification time, and header dimensions. Blocking, so
    /// call it off the GTK thread. `None` if the file is gone.
    pub fn read_new_file_row(folder: &Path, path: PathBuf) -> Option<IndexedImage> {
        if !path.is_file() {
            return None;
        }
        let info = basic_info_from_path(folder, path);
        let dimensions = crate::metadata::exif::display_dimensions(&info.path);
        Some(IndexedImage {
            path: info.path,
            filename: info.filename,
            extension: info.extension,
            file_size: info.file_size,
            modified_secs: info.modified_secs,
            width: dimensions.map(|d| d.0),
            height: dimensions.map(|d| d.1),
            metadata_status: if dimensions.is_some() {
                "ready"
            } else {
                "missing"
            }
            .to_string(),
        })
    }

    /// Builds the row for a file made in `folder` after it was listed, or
    /// `None` if it is not a supported image directly in `folder`. Seeds the
    /// metadata cache from `row` so nothing reads the file again, and forgets
    /// an earlier removal of the same path.
    pub fn entry_for_new_file(&mut self, folder: &Path, row: &IndexedImage) -> Option<ImageEntry> {
        if !Self::is_image(&row.path) || row.path.parent() != Some(folder) {
            return None;
        }
        let dimensions = row.width.zip(row.height);
        self.metadata_cache.insert(
            row.path.clone(),
            CachedImageData {
                file_size: row.file_size,
                dimensions,
                quality: crate::quality::scorer::score_file_info(dimensions),
            },
        );
        self.removed_paths.remove(&row.path);
        Some(self.entry_from_row(row))
    }

    /// Adds files made in the open folder after it was listed (a copy, a
    /// duplicate, a rename, or a move into it), each where a reload sorted by
    /// `order` would put it. Rows already shown are left alone. The selected
    /// row stays selected, and nothing else in the store is touched. Returns
    /// how many rows were added: none if `folder` is not the open folder or
    /// its load has not finished (the load lists whatever is on disk).
    pub fn insert_new_files(
        &mut self,
        folder: &Path,
        rows: &[IndexedImage],
        order: SortOrder,
    ) -> usize {
        if self.current_folder.as_deref() != Some(folder) || self.folder_loading {
            return 0;
        }
        let mut added = 0;
        for row in rows {
            if self.path_to_index.contains_key(&row.path) {
                continue;
            }
            let Some(entry) = self.entry_for_new_file(folder, row) else {
                continue;
            };
            let at = insertion_point(self.store.n_items(), order, &entry, |index| {
                self.entry_at(index)
            });
            self.store.insert(at, &entry);
            self.reindex_from(at);
            if self.selected_index.is_some_and(|selected| selected >= at) {
                self.selected_index = self.selected_index.map(|selected| selected + 1);
            }
            self.all_known_paths.insert(row.path.clone());
            if let Err(position) = self
                .indexed_library_paths
                .binary_search_by(|known| path_sort_key(known, &row.path))
            {
                self.indexed_library_paths
                    .insert(position, row.path.clone());
            }
            added += 1;
        }
        added
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

    pub fn entry_for_path(&self, path: &std::path::Path) -> Option<ImageEntry> {
        self.entry_at(self.index_of_path(path)?)
    }

    /// Every row showing `path`. Uses the path index; only a virtual view
    /// that lists a path more than once needs the full scan.
    pub fn entries_for_path(&self, path: &std::path::Path) -> Vec<(u32, ImageEntry)> {
        if self.current_folder.is_some() || !self.virtual_has_repeated_paths {
            return self
                .index_of_path(path)
                .and_then(|index| self.entry_at(index).map(|entry| (index, entry)))
                .into_iter()
                .collect();
        }

        (0..self.store.n_items())
            .filter_map(|index| {
                self.entry_at(index)
                    .filter(|entry| entry.path() == path)
                    .map(|entry| (index, entry))
            })
            .collect()
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
        self.removed_paths.insert(path.to_path_buf());
        let Some(index) = self.path_to_index.remove(path) else {
            return;
        };
        self.store.remove(index);
        self.active_thumbnail_cache.remove(path);
        self.thumbnail_cache.remove(path);
        if let Some(pos) = self.cache_order.iter().position(|p| p == path) {
            self.cache_order.remove(pos);
        }
        self.metadata_cache.remove(path);
        self.indexed_library_paths.retain(|p| p != path);
        match self.selected_index {
            Some(selected) if selected == index => self.selected_index = None,
            // The list closes up, so the selected image moves up with it.
            Some(selected) if selected > index => self.selected_index = Some(selected - 1),
            _ => {}
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

    /// Populate the current store with a virtual image set. The rows replace
    /// the old ones in one `items-changed` signal, so the sorted model and the
    /// list view update once instead of once per row.
    pub fn load_virtual(&mut self, paths: &[PathBuf]) -> (u64, Vec<PathBuf>) {
        let generation = next_virtual_generation();
        self.virtual_generation = generation;
        self.load_token = self.load_token.wrapping_add(1);
        self.folder_loading = false;
        self.path_to_index.clear();
        // A sort/filter rebuild must not lose textures evicted from the global
        // cache. Carry only thumbnails belonging to the new view forward.
        let previous_thumbnails = std::mem::take(&mut self.active_thumbnail_cache);
        self.selected_index = None;
        self.current_folder = None;
        let mut metadata_pending = Vec::new();
        let mut repeated = false;
        let mut entries = Vec::with_capacity(paths.len());
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
            if let Some(texture) = previous_thumbnails
                .get(path)
                .or_else(|| self.thumbnail_cache.get(path))
            {
                entry.set_thumbnail(Some(texture.clone()));
                self.active_thumbnail_cache
                    .insert(path.clone(), texture.clone());
            }
            entries.push(entry);
            repeated |= self
                .path_to_index
                .insert(path.clone(), index as u32)
                .is_some();
        }
        self.store.splice(0, self.store.n_items(), &entries);
        self.virtual_has_repeated_paths = repeated;
        (generation, metadata_pending)
    }

    /// Restore a captured view without rescanning or decoding on the GTK thread.
    pub fn restore_view(&mut self, entries: &[ImageEntry], folder: Option<PathBuf>) {
        self.virtual_generation = next_virtual_generation();
        self.load_token = self.load_token.wrapping_add(1);
        self.folder_loading = false;
        self.path_to_index.clear();
        self.active_thumbnail_cache.clear();
        self.current_folder = folder;
        self.selected_index = None;
        let mut repeated = false;
        for (i, entry) in entries.iter().enumerate() {
            repeated |= self.path_to_index.insert(entry.path(), i as u32).is_some();
        }
        // One signal for the whole view, as in load_virtual.
        self.store.splice(0, self.store.n_items(), entries);
        self.virtual_has_repeated_paths = repeated;
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

    pub fn cached_image_data_static(
        path: &Path,
        cache: &mut FxHashMap<PathBuf, CachedImageData>,
    ) -> CachedImageData {
        if let Some(cached) = cache.get(path) {
            return cached.clone();
        }

        let file_size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        let dimensions = crate::metadata::exif::display_dimensions(path);
        let cached = CachedImageData {
            file_size,
            dimensions,
            quality: crate::quality::scorer::score_file_info(dimensions),
        };
        cache.insert(path.to_path_buf(), cached.clone());
        cached
    }

    pub(crate) fn is_image(path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        if name.contains(".pending-") || name.contains(".ncnn-intermediate") {
            return false;
        }
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

/// Cancellation is cooperative between filesystem operations; no GTK work is done here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirectoryStamp {
    pub device: u64,
    pub inode: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl DirectoryStamp {
    pub(crate) fn read(path: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path)?;
        if !m.is_dir() {
            return Err(std::io::Error::other("not a directory"));
        }
        Ok(Self {
            device: m.dev(),
            inode: m.ino(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScanCoverage {
    pub directories: HashMap<PathBuf, DirectoryStamp>,
    pub failed: usize,
    pub excluded: usize,
}

impl ScanCoverage {
    /// A skipped location might hold a moved copy. Never search it merely to
    /// permit cleanup, and never mistake its absence from this walk for loss.
    pub(crate) fn unchanged_and_complete(&self) -> bool {
        self.failed == 0
            && self.excluded == 0
            && !self.directories.is_empty()
            && self
                .directories
                .iter()
                .all(|(path, before)| DirectoryStamp::read(path).is_ok_and(|now| now == *before))
    }
}

/// Each start (a root, or the open folder) is walked on its own filesystem and
/// without hidden folders, like the sidebar tree (`model::walk`).
pub(crate) fn collect_library_image_paths_cancellable(
    library_roots: &[PathBuf],
    folder_mode: FolderMode,
    current_folder: Option<&Path>,
    disabled: &[PathBuf],
    canceled: &dyn Fn() -> bool,
) -> Vec<PathBuf> {
    collect_library_image_paths_with_coverage(
        library_roots,
        folder_mode,
        current_folder,
        disabled,
        canceled,
    )
    .0
}

pub(crate) fn collect_library_image_paths_with_coverage(
    library_roots: &[PathBuf],
    folder_mode: FolderMode,
    current_folder: Option<&Path>,
    disabled: &[PathBuf],
    canceled: &dyn Fn() -> bool,
) -> (Vec<PathBuf>, ScanCoverage) {
    let mut coverage = ScanCoverage::default();
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home"));
    let roots = if library_roots.is_empty() {
        vec![home.join("Pictures"), home.join("Downloads"), home]
    } else {
        library_roots.to_vec()
    };
    let maximum_depth = if folder_mode == FolderMode::DrillDown {
        usize::MAX
    } else {
        1
    };
    let mut pending: Vec<_> = roots
        .into_iter()
        .map(|p| {
            let device = walk::device_of(&p);
            (p, 0, device)
        })
        .collect();
    if let Some(folder) = current_folder {
        let device = walk::device_of(folder);
        pending.insert(0, (folder.to_path_buf(), maximum_depth, device));
    }
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    while let Some((folder, depth, device)) = pending.pop() {
        if canceled() {
            coverage.failed += 1;
            return (Vec::new(), coverage);
        }
        if path_is_disabled(&folder, disabled) {
            coverage.excluded += 1;
            continue;
        }
        if !seen.insert(folder.clone()) {
            continue;
        }
        match DirectoryStamp::read(&folder) {
            Ok(stamp) => {
                // A mount can appear after its parent queued this folder.
                if Some(stamp.device) != device {
                    coverage.excluded += 1;
                    continue;
                }
                coverage.directories.insert(folder.clone(), stamp);
            }
            Err(_) => {
                coverage.failed += 1;
                continue;
            }
        }
        let Ok(entries) = std::fs::read_dir(&folder) else {
            coverage.failed += 1;
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                coverage.failed += 1;
                continue;
            };
            if canceled() {
                coverage.failed += 1;
                return (Vec::new(), coverage);
            }
            let path = entry.path();
            if path_is_disabled(&path, disabled) {
                coverage.excluded += 1;
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                coverage.failed += 1;
                continue;
            };
            if kind.is_dir() && depth < maximum_depth {
                if walk::may_descend_into(&entry, device) {
                    pending.push((path, depth + 1, device));
                } else {
                    coverage.excluded += 1;
                }
            } else if kind.is_file() && LibraryManager::is_image(&path) {
                paths.push(path);
            } else if kind.is_symlink() {
                coverage.excluded += 1;
            }
        }
    }
    paths.sort_by(|a, b| path_sort_key(a, b));
    paths.dedup();
    (paths, coverage)
}

fn path_is_disabled(path: &Path, disabled: &[PathBuf]) -> bool {
    disabled.iter().any(|folder| path.starts_with(folder))
}

fn path_sort_key(a: &Path, b: &Path) -> std::cmp::Ordering {
    a.to_string_lossy()
        .to_lowercase()
        .cmp(&b.to_string_lossy().to_lowercase())
}

/// What the sort rule looks at in one row.
pub struct SortRow<'a> {
    pub path: &'a Path,
    pub filename: &'a str,
    pub modified_secs: Option<i64>,
    pub dimensions: Option<(u32, u32)>,
}

/// The one sort rule for every listing (Work Order item 16): `order`'s own
/// key, then the name ignoring case, then the full path, so rows that tie
/// always come out the same way. Rows without a time or a size go last in
/// either direction. Name Z–A reverses only the name; ties still go by path.
pub fn compare_rows(order: SortOrder, a: &SortRow, b: &SortRow) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let by_name = || a.filename.to_lowercase().cmp(&b.filename.to_lowercase());
    let extension = |row: &SortRow| {
        row.path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_lowercase()
    };
    let key = match order {
        SortOrder::NameAsc => by_name(),
        SortOrder::NameDesc => by_name().reverse(),
        SortOrder::DateModifiedAsc | SortOrder::DateModifiedDesc => {
            match (a.modified_secs, b.modified_secs) {
                (Some(a), Some(b)) if order.descending() => b.cmp(&a),
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        }
        SortOrder::FileTypeAsc => extension(a).cmp(&extension(b)),
        SortOrder::FileTypeDesc => extension(b).cmp(&extension(a)),
        SortOrder::AspectAsc
        | SortOrder::AspectDesc
        | SortOrder::WideScreenAsc
        | SortOrder::WideScreenDesc => crate::model::browse::compare_aspects(
            a.dimensions,
            b.dimensions,
            order.field() == SortField::WideScreen,
            order.descending(),
        ),
    };
    key.then_with(by_name).then_with(|| a.path.cmp(b.path))
}

/// Orders two rows as a folder listing does for `order`, so a row added
/// later lands where a reload would put it.
pub fn compare_entries(order: SortOrder, a: &ImageEntry, b: &ImageEntry) -> std::cmp::Ordering {
    let (a_path, b_path) = (a.path(), b.path());
    let (a_name, b_name) = (a.filename(), b.filename());
    compare_rows(
        order,
        &SortRow {
            path: &a_path,
            filename: &a_name,
            modified_secs: a.modified_secs(),
            dimensions: a.dimensions(),
        },
        &SortRow {
            path: &b_path,
            filename: &b_name,
            modified_secs: b.modified_secs(),
            dimensions: b.dimensions(),
        },
    )
}

/// Where `new` goes among `len` rows already in `order`: after any that
/// compare equal.
fn insertion_point(
    len: u32,
    order: SortOrder,
    new: &ImageEntry,
    at: impl Fn(u32) -> Option<ImageEntry>,
) -> u32 {
    let (mut low, mut high) = (0, len);
    while low < high {
        let middle = low + (high - low) / 2;
        match at(middle) {
            Some(existing) if compare_entries(order, &existing, new).is_gt() => high = middle,
            _ => low = middle + 1,
        }
    }
    low
}

/// Puts `entry` into `entries`, already in `order`, at its place.
pub fn insert_in_order(entries: &mut Vec<ImageEntry>, entry: ImageEntry, order: SortOrder) {
    let at = insertion_point(entries.len() as u32, order, &entry, |index| {
        entries.get(index as usize).cloned()
    });
    entries.insert(at as usize, entry);
}

pub fn sort_raw_entries(entries: &mut [RawImageEntry], order: SortOrder) {
    let row = |entry: &RawImageEntry| -> (Option<i64>, Option<(u32, u32)>) {
        let modified = entry
            .modified
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_secs() as i64);
        (modified, Some((entry.width, entry.height)))
    };
    entries.sort_by(|a, b| {
        let ((a_time, a_size), (b_time, b_size)) = (row(a), row(b));
        compare_rows(
            order,
            &SortRow {
                path: &a.path,
                filename: &a.filename,
                modified_secs: a_time,
                dimensions: a_size,
            },
            &SortRow {
                path: &b.path,
                filename: &b.filename,
                modified_secs: b_time,
                dimensions: b_size,
            },
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn whole_library_scan_finds_unindexed_images_and_excludes_disabled_subtrees() {
        let root = temp_path("enabled-tree");
        let enabled = root.join("enabled/deep");
        let disabled = root.join("disabled");
        std::fs::create_dir_all(&enabled).unwrap();
        std::fs::create_dir_all(disabled.join("deep")).unwrap();
        write_bmp(&root.join("root.bmp"), 16, 9);
        write_bmp(&enabled.join("new.bmp"), 16, 9);
        write_bmp(&disabled.join("hidden.bmp"), 16, 9);
        write_bmp(&disabled.join("deep/hidden.bmp"), 16, 9);
        let paths = collect_library_image_paths_cancellable(
            std::slice::from_ref(&root),
            FolderMode::DrillDown,
            None,
            std::slice::from_ref(&disabled),
            &|| false,
        );
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&enabled.join("new.bmp")));
        assert!(paths.contains(&root.join("root.bmp")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn whole_library_scan_covers_every_root_and_their_disabled_folders() {
        let base = temp_path("two-roots");
        let (pictures, downloads) = (base.join("Pictures"), base.join("Downloads"));
        std::fs::create_dir_all(pictures.join("art")).unwrap();
        std::fs::create_dir_all(downloads.join("off")).unwrap();
        std::fs::create_dir_all(base.join("Elsewhere")).unwrap();
        write_bmp(&pictures.join("art/a.bmp"), 16, 9);
        write_bmp(&downloads.join("b.bmp"), 16, 9);
        write_bmp(&downloads.join("off/c.bmp"), 16, 9);
        write_bmp(&base.join("Elsewhere/d.bmp"), 16, 9);
        let paths = collect_library_image_paths_cancellable(
            &[pictures.clone(), downloads.clone()],
            FolderMode::DrillDown,
            None,
            &[downloads.join("off")],
            &|| false,
        );
        let mut expected = vec![pictures.join("art/a.bmp"), downloads.join("b.bmp")];
        expected.sort_by(|a, b| path_sort_key(a, b));
        assert_eq!(paths, expected);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn whole_library_scan_skips_hidden_folders_but_not_a_hidden_start() {
        let root = temp_path("hidden-tree");
        let nested = root.join("art/deep");
        let hidden = root.join(".cache/thumbs");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&hidden).unwrap();
        write_bmp(&nested.join("kept.bmp"), 16, 9);
        write_bmp(&hidden.join("cached.bmp"), 16, 9);
        write_bmp(&root.join(".cache/direct.bmp"), 16, 9);
        let paths = collect_library_image_paths_cancellable(
            std::slice::from_ref(&root),
            FolderMode::DrillDown,
            None,
            &[],
            &|| false,
        );
        assert_eq!(paths, vec![nested.join("kept.bmp")]);

        // Choosing a hidden folder on purpose still lists it.
        let paths = collect_library_image_paths_cancellable(
            std::slice::from_ref(&root),
            FolderMode::DrillDown,
            Some(&root.join(".cache")),
            &[],
            &|| false,
        );
        assert!(paths.contains(&root.join(".cache/direct.bmp")));
        assert!(
            !paths.contains(&hidden.join("cached.bmp")),
            "its subfolders stay hidden"
        );
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn entries_for_path_in_virtual_views_with_and_without_repeats() {
        let mut library = LibraryManager::new();
        let a = PathBuf::from("/v/a.jpg");
        let b = PathBuf::from("/v/b.jpg");
        library.load_virtual(&[a.clone(), b.clone()]);
        let found: Vec<u32> = library
            .entries_for_path(&b)
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        assert_eq!(found, vec![1]);
        assert!(library
            .entries_for_path(Path::new("/v/none.jpg"))
            .is_empty());

        library.load_virtual(&[a.clone(), b, a.clone()]);
        let found: Vec<u32> = library
            .entries_for_path(&a)
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        assert_eq!(found, vec![0, 2], "a repeated path is found in every row");
    }

    fn row(name: &str, modified: Option<i64>, size: Option<(u32, u32)>) -> IndexedImage {
        IndexedImage {
            path: PathBuf::from("/photos").join(name),
            filename: name.to_string(),
            extension: name.rsplit('.').next().unwrap_or_default().to_lowercase(),
            file_size: 1000,
            modified_secs: modified,
            width: size.map(|s| s.0),
            height: size.map(|s| s.1),
            metadata_status: "ready".to_string(),
        }
    }

    fn names(library: &LibraryManager) -> Vec<String> {
        (0..library.image_count())
            .map(|i| library.entry_at(i).unwrap().filename())
            .collect()
    }

    /// An open, fully loaded folder holding `rows`, already in `order`.
    fn loaded(rows: Vec<IndexedImage>) -> LibraryManager {
        let folder = PathBuf::from("/photos");
        let mut library = LibraryManager::new();
        library.reset_for_folder(&folder);
        library.load_indexed_folder(&folder, rows);
        library
    }

    #[test]
    fn new_files_land_where_each_sort_order_would_list_them() {
        let folder = Path::new("/photos");
        let cases: [(SortOrder, Vec<IndexedImage>, IndexedImage, [&str; 4]); 6] = [
            (
                SortOrder::NameAsc,
                vec![
                    row("a.png", None, None),
                    row("c.png", None, None),
                    row("d.png", None, None),
                ],
                row("B.png", None, None),
                ["a.png", "B.png", "c.png", "d.png"],
            ),
            (
                SortOrder::NameDesc,
                vec![
                    row("d.png", None, None),
                    row("c.png", None, None),
                    row("a.png", None, None),
                ],
                row("B.png", None, None),
                ["d.png", "c.png", "B.png", "a.png"],
            ),
            (
                SortOrder::DateModifiedDesc,
                vec![
                    row("a.png", Some(300), None),
                    row("b.png", Some(200), None),
                    row("c.png", Some(100), None),
                ],
                row("new.png", Some(250), None),
                ["a.png", "new.png", "b.png", "c.png"],
            ),
            (
                SortOrder::DateModifiedAsc,
                vec![
                    row("c.png", Some(100), None),
                    row("b.png", Some(200), None),
                    row("a.png", None, None),
                ],
                row("new.png", Some(250), None),
                ["c.png", "b.png", "new.png", "a.png"],
            ),
            (
                SortOrder::FileTypeAsc,
                vec![
                    row("a.jpg", None, None),
                    row("b.png", None, None),
                    row("c.webp", None, None),
                ],
                row("z.png", None, None),
                ["a.jpg", "b.png", "z.png", "c.webp"],
            ),
            (
                SortOrder::AspectAsc,
                vec![
                    row("tall.png", None, Some((90, 160))),
                    row("wide.png", None, Some((160, 90))),
                    row("unknown.png", None, None),
                ],
                row("square.png", None, Some((100, 100))),
                ["tall.png", "square.png", "wide.png", "unknown.png"],
            ),
        ];
        for (order, rows, new, expected) in cases {
            let mut library = loaded(rows);
            let added = library.insert_new_files(folder, std::slice::from_ref(&new), order);
            assert_eq!(added, 1, "{order:?}");
            assert_eq!(names(&library), expected, "{order:?}");
            for (index, name) in expected.iter().enumerate() {
                let path = folder.join(name);
                assert_eq!(
                    library.index_of_path(&path),
                    Some(index as u32),
                    "{order:?}"
                );
            }
        }
    }

    #[test]
    fn a_new_row_above_the_selection_keeps_the_selection_on_its_image() {
        let folder = Path::new("/photos");
        let mut library = loaded(vec![
            row("a.png", Some(300), None),
            row("b.png", Some(200), None),
            row("c.png", Some(100), None),
        ]);
        library.selected_index = Some(1);

        library.insert_new_files(
            folder,
            &[
                row("new.png", Some(400), None),
                row("late.png", Some(50), None),
            ],
            SortOrder::DateModifiedDesc,
        );

        assert_eq!(
            names(&library),
            ["new.png", "a.png", "b.png", "c.png", "late.png"]
        );
        assert_eq!(library.selected_index, Some(2));
        assert_eq!(library.selected_entry().unwrap().filename(), "b.png");
    }

    #[test]
    fn new_files_are_not_added_twice_or_while_the_folder_loads() {
        let folder = Path::new("/photos");
        let mut library = loaded(vec![row("a.png", Some(300), None)]);
        let new = row("b.png", Some(200), None);
        let order = SortOrder::DateModifiedDesc;

        assert_eq!(
            library.insert_new_files(folder, std::slice::from_ref(&new), order),
            1
        );
        assert_eq!(
            library.insert_new_files(folder, std::slice::from_ref(&new), order),
            0
        );
        assert_eq!(library.image_count(), 2);

        // Not the open folder, not an image, not directly in the folder.
        let elsewhere = IndexedImage {
            path: PathBuf::from("/other/c.png"),
            ..row("c.png", None, None)
        };
        let nested = IndexedImage {
            path: PathBuf::from("/photos/sub/d.png"),
            ..row("d.png", None, None)
        };
        let text = row("notes.txt", None, None);
        assert_eq!(
            library.insert_new_files(folder, &[elsewhere, nested, text], order),
            0
        );
        assert_eq!(
            library.insert_new_files(Path::new("/other"), &[row("e.png", None, None)], order),
            0
        );

        // A load still filling the store will list the file itself.
        library.reset_for_folder(folder);
        assert!(library.is_loading_folder());
        assert_eq!(
            library.insert_new_files(folder, &[row("f.png", None, None)], order),
            0
        );
        library.finish_load_folder();
        assert!(!library.is_loading_folder());
        assert_eq!(
            library.insert_new_files(folder, &[row("f.png", None, None)], order),
            1
        );
    }

    #[test]
    fn a_path_removed_earlier_can_be_added_back() {
        let folder = Path::new("/photos");
        let mut library = loaded(vec![
            row("a.png", Some(300), None),
            row("b.png", Some(200), None),
        ]);
        let a = folder.join("a.png");
        library.remove_path(&a);
        assert!(library.removed_paths.contains(&a));

        library.insert_new_files(
            folder,
            &[row("a.png", Some(300), None)],
            SortOrder::DateModifiedDesc,
        );

        assert_eq!(names(&library), ["a.png", "b.png"]);
        assert!(!library.removed_paths.contains(&a));
    }

    #[test]
    fn removing_a_row_above_the_selection_keeps_the_selection_on_its_image() {
        let mut library = loaded(vec![
            row("a.png", Some(300), None),
            row("b.png", Some(200), None),
            row("c.png", Some(100), None),
        ]);
        library.selected_index = Some(2);

        library.remove_path(Path::new("/photos/a.png"));

        assert_eq!(library.selected_index, Some(1));
        assert_eq!(library.selected_entry().unwrap().filename(), "c.png");
    }

    #[test]
    fn insert_in_order_places_a_kept_view_entry() {
        let mut entries: Vec<ImageEntry> = ["a.png", "c.png"]
            .iter()
            .map(|name| ImageEntry::new(PathBuf::from("/photos").join(name)))
            .collect();
        insert_in_order(
            &mut entries,
            ImageEntry::new(PathBuf::from("/photos/b.png")),
            SortOrder::NameAsc,
        );
        let names: Vec<_> = entries.iter().map(|e| e.filename()).collect();
        assert_eq!(names, ["a.png", "b.png", "c.png"]);
    }

    #[test]
    fn reopening_a_folder_retires_the_earlier_load() {
        let a = PathBuf::from("/photos/a");
        let b = PathBuf::from("/photos/b");
        let mut library = LibraryManager::new();

        library.reset_for_folder(&a);
        let first_load = library.load_token();
        library.reset_for_folder(&b);
        library.reset_for_folder(&a);

        assert!(
            !library.is_current_load(&a, first_load),
            "the first load of A must not append once A was opened again"
        );
        assert!(library.is_current_load(&a, library.load_token()));
    }

    #[test]
    fn replacing_the_view_retires_the_earlier_load() {
        let a = PathBuf::from("/photos/a");
        let mut library = LibraryManager::new();

        library.reset_for_folder(&a);
        let load = library.load_token();
        library.load_virtual(&[]);
        library.restore_view(&[], Some(a.clone()));

        assert!(
            !library.is_current_load(&a, load),
            "a view restored over the store must not receive an old load's rows"
        );
    }

    #[test]
    fn a_load_keeps_its_token_while_it_fills_the_store() {
        let a = PathBuf::from("/photos/a");
        let mut library = LibraryManager::new();

        library.reset_for_folder(&a);
        let load = library.load_token();
        library.start_load_folder(&a);
        library.append_indexed_rows(Vec::new());
        library.finish_load_folder();
        library.load_indexed_folder(&a, Vec::new());

        assert!(library.is_current_load(&a, load));
    }

    #[test]
    fn load_virtual_bumps_generation_each_time() {
        let path = PathBuf::from("/photos/a.jpg");
        let mut library = LibraryManager::new();

        let (first, _) = library.load_virtual(std::slice::from_ref(&path));
        let (second, _) = library.load_virtual(&[]);

        assert_eq!(library.virtual_generation, second);
        assert!(second > first);
    }

    #[test]
    fn virtual_reload_keeps_active_thumbnails_after_global_cache_eviction() {
        let mut library = LibraryManager::new();
        library.set_thumbnail_cache_max(100);
        let paths: Vec<_> = (0..103)
            .map(|i| PathBuf::from(format!("/photos/{i}.png")))
            .collect();
        library.load_virtual(&paths);
        let textures: Vec<Texture> = (0..paths.len())
            .map(|i| {
                gdk4::MemoryTexture::new(
                    1,
                    1,
                    gdk4::MemoryFormat::R8g8b8a8,
                    &glib::Bytes::from_owned(vec![i as u8, 0, 0, 255]),
                    4,
                )
                .upcast()
            })
            .collect();
        for (i, (path, texture)) in paths.iter().zip(&textures).enumerate() {
            library
                .entry_at(i as u32)
                .unwrap()
                .set_thumbnail(Some(texture.clone()));
            library.insert_thumbnail(path.clone(), texture.clone(), true);
        }
        assert!(!library.thumbnail_cache.contains_key(&paths[0]));
        let (before, _) =
            library.load_virtual(&[paths[1].clone(), paths[0].clone(), paths[1].clone()]);
        for (i, original) in [1, 0, 1].into_iter().enumerate() {
            assert_eq!(
                library.entry_at(i as u32).unwrap().thumbnail(),
                Some(textures[original].clone()),
                "reordered and repeated rows keep the correct cached texture"
            );
        }
        assert_eq!(library.active_thumbnail_cache.len(), 2);
        assert!(!library.active_thumbnail_cache.contains_key(&paths[102]));
        let (after, _) = library.load_virtual(&[paths[0].clone(), paths[1].clone()]);
        assert!(after > before);
        assert_eq!(
            library.entry_at(0).unwrap().thumbnail(),
            Some(textures[0].clone())
        );
        library.load_virtual(&[]);
        assert!(library.active_thumbnail_cache.is_empty());
    }

    #[test]
    fn scan_folder_raw_returns_sorted_plain_entries_without_dimensions() {
        let root = temp_path("raw-scan");
        std::fs::create_dir_all(&root).unwrap();

        let img_a = root.join("a.JPG");
        let img_b = root.join("B.bmp");
        let pending = root.join("x.pending-123.png");
        let intermediate = root.join("y.ncnn-intermediate.webp");
        let txt = root.join("note.txt");

        write_jpeg(&img_a, 640, 480, 80);
        write_bmp(&img_b, 320, 200);
        write_bmp(&pending, 64, 64);
        write_bmp(&intermediate, 64, 64);
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
    fn is_image_rejects_transient_upscale_artifacts() {
        assert!(!LibraryManager::is_image(Path::new(
            "/tmp/example.pending-123.png"
        )));
        assert!(!LibraryManager::is_image(Path::new(
            "/tmp/example.ncnn-intermediate.webp"
        )));
        assert!(LibraryManager::is_image(Path::new("/tmp/example.webp")));
    }
}
