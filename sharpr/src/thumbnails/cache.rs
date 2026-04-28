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
        return Some(
            PathBuf::from(cache_home)
                .join("sharpr")
                .join("thumbnails-r1"),
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn temp_file_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sharpr-thumb-{}-{nanos}.png", std::process::id()))
    }

    #[test]
    fn thumbnail_cache_dir_prefers_xdg_cache_home() {
        let _guard = env_lock().lock().unwrap();
        let original = std::env::var_os("XDG_CACHE_HOME");
        std::env::set_var("XDG_CACHE_HOME", "/tmp/sharpr-cache-home");

        let dir = thumbnail_cache_dir().unwrap();

        match original {
            Some(value) => std::env::set_var("XDG_CACHE_HOME", value),
            None => std::env::remove_var("XDG_CACHE_HOME"),
        }

        assert_eq!(
            dir,
            PathBuf::from("/tmp/sharpr-cache-home/sharpr/thumbnails-r1")
        );
    }

    #[test]
    fn thumbnail_cache_path_includes_png_extension_and_metadata_fingerprint() {
        let path = temp_file_path();
        let bytes = b"thumb";
        std::fs::write(&path, bytes).unwrap();

        let cache_path = thumbnail_cache_path(&path).unwrap();

        let _ = std::fs::remove_file(&path);
        let filename = cache_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap();
        assert_eq!(
            cache_path.extension().and_then(|ext| ext.to_str()),
            Some("png")
        );
        assert!(filename.contains(&format!("-{}-", bytes.len())));
    }
}
