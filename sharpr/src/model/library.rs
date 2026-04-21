use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use gdk4::Texture;
use gio::prelude::*;

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
    folder_history: HashMap<PathBuf, u32>,
    /// O(1) path → list index lookup, kept in sync with `store`.
    pub(crate) path_to_index: HashMap<PathBuf, u32>,
    /// Set of all image paths encountered during the session, for cross-folder duplicates.
    pub(crate) all_known_paths: HashSet<PathBuf>,
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
    metadata_cache: HashMap<PathBuf, CachedImageData>,
    indexed_library_paths: Vec<PathBuf>,
}

impl LibraryManager {
    const MAX_PREVIEW_CACHE: usize = 3;

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
            metadata_cache: HashMap::new(),
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
        self.prefetch_cache.clear();
        self.prefetch_order.clear();
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
                let (width, height) = image::image_dimensions(&path).unwrap_or((0, 0));

                RawImageEntry {
                    path,
                    filename,
                    file_size,
                    modified,
                    width,
                    height,
                }
            })
            .collect()
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
        self.thumbnail_cache.remove(path);
        if let Some(pos) = self.cache_order.iter().position(|p| p == path) {
            self.cache_order.remove(pos);
        }
        self.preview_cache.remove(path);
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
        self.thumbnail_cache.remove(path);
        if let Some(pos) = self.cache_order.iter().position(|p| p == path) {
            self.cache_order.remove(pos);
        }

        self.preview_cache.remove(path);
        if let Some(pos) = self.preview_order.iter().position(|p| p == path) {
            self.preview_order.remove(pos);
        }

        self.prefetch_cache.remove(path);
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

    /// Store completed prefetch bytes. Evicts oldest if cache exceeds 3 entries.
    pub fn insert_prefetch(&mut self, path: PathBuf, bytes: Vec<u8>, width: u32, height: u32) {
        self.prefetch_in_flight.remove(&path);
        if self.prefetch_cache.contains_key(&path) {
            return;
        }
        if self.prefetch_order.len() >= 3 {
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
            let entry = self.build_entry(path.clone(), false);
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

    pub fn bg_scan_quality_prep(
        &self,
    ) -> (
        Vec<PathBuf>,
        Option<PathBuf>,
        HashMap<PathBuf, CachedImageData>,
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
        cache: HashMap<PathBuf, CachedImageData>,
    ) {
        self.indexed_library_paths = paths;
        self.metadata_cache = cache;
    }

    pub fn compute_paths_for_quality_class(
        library_root: Option<PathBuf>,
        class: QualityClass,
        mut indexed_paths: Vec<PathBuf>,
        current_folder: Option<PathBuf>,
        mut metadata_cache: HashMap<PathBuf, CachedImageData>,
    ) -> (
        Vec<PathBuf>,
        HashMap<PathBuf, CachedImageData>,
        Vec<PathBuf>,
    ) {
        let paths = collect_library_image_paths(library_root, current_folder.as_deref());
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
        cache: &mut HashMap<PathBuf, CachedImageData>,
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
) -> Vec<PathBuf> {
    let mut folders = collect_library_folders(library_root);
    if let Some(current_folder) = current_folder {
        if current_folder.is_dir() && !folders.iter().any(|folder| folder == current_folder) {
            folders.push(current_folder.to_path_buf());
        }
    }

    let mut paths = Vec::new();
    for folder in folders {
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

pub fn sort_raw_entries(entries: &mut Vec<RawImageEntry>, order: SortOrder) {
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
        );
        let (_, _, upscale_paths) = LibraryManager::compute_paths_for_quality_class(
            settings.library_root.clone(),
            QualityClass::NeedsUpscale,
            indexed_paths,
            current_folder,
            metadata_cache,
        );

        assert_eq!(excellent_paths, vec![excellent]);
        assert_eq!(upscale_paths, vec![needs_upscale]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scan_folder_raw_returns_sorted_plain_entries_with_metadata() {
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
        assert_eq!(entries[0].width, 640);
        assert_eq!(entries[0].height, 480);
        assert!(entries[0].file_size > 0);

        assert_eq!(entries[1].path, img_b);
        assert_eq!(entries[1].filename, "B.bmp");
        assert_eq!(entries[1].width, 320);
        assert_eq!(entries[1].height, 200);
        assert!(entries[1].file_size > 0);

        std::fs::remove_dir_all(root).unwrap();
    }
}
