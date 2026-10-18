//! What the Library Overview shows: how big the library is and what its
//! indexes hold (`ROADMAP.md` Phase 5). GTK-free; gathered on a worker.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::FolderMode;
use crate::library_index::LibraryIndex;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryStats {
    pub images: usize,
    pub bytes: u64,
    /// Image counts by format, most first (JPG and JPEG together).
    pub formats: Vec<(String, usize)>,
    /// Images known to be smaller than 3840×2160 in either orientation.
    pub below_4k: usize,
    /// Images whose size Sharpr hasn't read yet.
    pub size_unknown: usize,
    /// Each disabled folder with the images it holds.
    pub disabled: Vec<(PathBuf, usize)>,
    /// Images with a fingerprint matching the file as it is now.
    pub fingerprinted: usize,
    /// Groups of byte-identical files, and how many files beyond one each.
    pub copy_groups: usize,
    pub extra_copies: usize,
    pub buckets: usize,
    /// Distinct images in any bucket, as shown under the root.
    pub bucket_images: usize,
    /// Bucket members that can't be found anywhere.
    pub bucket_missing: usize,
    /// Size of the visual-search vector file, if it exists.
    pub vector_file_bytes: Option<u64>,
}

fn format_name(path: &Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "JPEG".into(),
        Some("jxl") => "JPEG XL".into(),
        Some("tif" | "tiff") => "TIFF".into(),
        Some(other) => other.to_ascii_uppercase(),
        None => "Other".into(),
    }
}

/// `4k` means at least 3840×2160 in either orientation (D16).
fn is_below_4k((width, height): (u32, u32)) -> bool {
    width.max(height) < 3840 || width.min(height) < 2160
}

/// Gathers the overview for the library with these `roots`. Reads the file system
/// and the indexes only; changes nothing.
pub fn gather(
    index: &LibraryIndex,
    roots: &[PathBuf],
    disabled: &[PathBuf],
    library_id: Option<&str>,
    vector_file: &Path,
) -> Result<LibraryStats, String> {
    let never = || false;
    let walk = |dirs: &[PathBuf], excluded: &[PathBuf]| {
        crate::model::library::collect_library_image_paths_cancellable(
            dirs,
            FolderMode::DrillDown,
            None,
            excluded,
            &never,
        )
    };
    let paths = walk(roots, disabled);
    let mut stats = LibraryStats {
        images: paths.len(),
        ..Default::default()
    };

    let dimensions = index.stored_dimensions().map_err(|e| e.to_string())?;
    let fingerprints = index.cached_fingerprints().map_err(|e| e.to_string())?;
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_hash: HashMap<&str, usize> = HashMap::new();
    for path in &paths {
        let key = path.to_string_lossy();
        *formats.entry(format_name(path)).or_default() += 1;
        match dimensions.get(key.as_ref()) {
            Some(size) if is_below_4k(*size) => stats.below_4k += 1,
            Some(_) => {}
            None => stats.size_unknown += 1,
        }
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        stats.bytes += metadata.len();
        let stamp = crate::fingerprint::file_stamp(&metadata);
        if let Some((size, mtime, hash)) = fingerprints.get(key.as_ref()) {
            if (*size, *mtime) == stamp {
                stats.fingerprinted += 1;
                *by_hash.entry(hash.as_str()).or_default() += 1;
            }
        }
    }
    let mut formats: Vec<(String, usize)> = formats.into_iter().collect();
    formats.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    stats.formats = formats;
    for count in by_hash.values().filter(|count| **count > 1) {
        stats.copy_groups += 1;
        stats.extra_copies += count - 1;
    }

    // Only the outermost disabled folders; nested ones are inside them.
    for folder in disabled {
        if disabled
            .iter()
            .any(|other| other != folder && folder.starts_with(other))
        {
            continue;
        }
        if roots.iter().any(|root| folder.starts_with(root)) {
            stats.disabled.push((
                folder.clone(),
                walk(std::slice::from_ref(folder), &[]).len(),
            ));
        }
    }

    let collections = index
        .list_collections_for_library(library_id)
        .map_err(|e| e.to_string())?;
    stats.buckets = collections.len();
    let mut in_buckets: HashSet<PathBuf> = HashSet::new();
    for bucket in collections.iter().filter(|c| c.parent_id.is_none()) {
        // A root bucket's view includes its sub-buckets' members.
        let view = crate::buckets::view(index, bucket.id, roots, disabled)?;
        stats.bucket_missing += view.missing;
        in_buckets.extend(view.paths);
    }
    stats.bucket_images = in_buckets.len();
    stats.vector_file_bytes = std::fs::metadata(vector_file).ok().map(|m| m.len());
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overview_counts_formats_copies_disabled_folders_and_buckets() {
        let root = std::env::temp_dir().join(format!(
            "sharpr-overview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("off")).unwrap();
        std::fs::write(root.join("a.jpg"), b"same").unwrap();
        std::fs::write(root.join("copy.jpeg"), b"same").unwrap();
        std::fs::write(root.join("b.png"), b"bb").unwrap();
        std::fs::write(root.join("off/c.jpg"), b"c").unwrap();
        let index = LibraryIndex::open_in_memory().unwrap();
        let tags = crate::tags::TagDatabase::open_in_memory().unwrap();
        crate::buckets::migrate(&index, &tags, &root.join("backups")).unwrap();
        let disabled = vec![root.join("off")];
        crate::fingerprint::run_pass(
            &index,
            std::slice::from_ref(&root),
            &disabled,
            &[],
            &|| false,
            &|_| {},
        )
        .unwrap();
        let bucket = index
            .create_collection("lib", None, "Keep", &[], None, None)
            .unwrap();
        crate::buckets::add_to_bucket(
            &index,
            bucket.id,
            &[root.join("b.png"), root.join("gone.jpg")],
        )
        .unwrap();

        let stats = gather(
            &index,
            std::slice::from_ref(&root),
            &disabled,
            Some("lib"),
            &root.join("visual-search.sqlite"),
        )
        .unwrap();
        assert_eq!(stats.images, 3);
        assert_eq!(stats.bytes, 10);
        assert_eq!(
            stats.formats,
            vec![("JPEG".to_string(), 2), ("PNG".to_string(), 1)]
        );
        assert_eq!((stats.below_4k, stats.size_unknown), (0, 3));
        assert_eq!(stats.disabled, vec![(root.join("off"), 1)]);
        assert_eq!(stats.fingerprinted, 3);
        assert_eq!((stats.copy_groups, stats.extra_copies), (1, 1));
        assert_eq!(
            (stats.buckets, stats.bucket_images, stats.bucket_missing),
            (1, 1, 1)
        );
        assert_eq!(stats.vector_file_bytes, None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn four_k_holds_in_either_orientation() {
        assert!(!is_below_4k((3840, 2160)));
        assert!(!is_below_4k((2160, 3840)));
        assert!(is_below_4k((3840, 2000)));
        assert!(is_below_4k((1920, 1080)));
    }
}
