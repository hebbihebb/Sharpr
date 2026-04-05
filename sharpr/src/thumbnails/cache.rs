use std::path::{Path, PathBuf};

/// Returns the expected disk-cache path for a source image, or `None` if
/// the source file's metadata can't be read or the cache directory can't
/// be determined.
///
/// The filename encodes the file's size and mtime so stale entries are
/// automatically bypassed when the source changes.
pub fn thumbnail_cache_path(source_path: &Path) -> Option<PathBuf> {
    let metadata = std::fs::metadata(source_path).ok()?;
    let modified = metadata.modified().ok()?;
    let modified = modified.duration_since(std::time::UNIX_EPOCH).ok()?;

    let cache_dir = thumbnail_cache_dir()?;
    let path_hash = stable_path_hash(source_path);
    let filename = format!(
        "{path_hash:016x}-{}-{}-{}.png",
        metadata.len(),
        modified.as_secs(),
        modified.subsec_nanos()
    );

    Some(cache_dir.join(filename))
}

pub fn thumbnail_cache_dir() -> Option<PathBuf> {
    if let Some(cache_home) = std::env::var_os("XDG_CACHE_HOME") {
        return Some(PathBuf::from(cache_home).join("sharpr").join("thumbnails-r1"));
    }
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".cache")
            .join("sharpr")
            .join("thumbnails-r1"),
    )
}

fn stable_path_hash(path: &Path) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in path.as_os_str().to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
