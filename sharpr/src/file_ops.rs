use std::path::{Path, PathBuf};

/// Returns the first non-existing path of the form:
///   `dir/stem.ext` → `dir/stem (copy).ext` → `dir/stem (copy 2).ext` → …
///
/// `exists` is a predicate so this can be tested without touching the filesystem.
pub fn next_available_path<F>(dir: &Path, stem: &str, ext: &str, exists: F) -> PathBuf
where
    F: Fn(&Path) -> bool,
{
    let make = |name: String| dir.join(name);

    let base = if ext.is_empty() {
        make(stem.to_string())
    } else {
        make(format!("{stem}.{ext}"))
    };
    if !exists(&base) {
        return base;
    }

    let copy1 = if ext.is_empty() {
        make(format!("{stem} (copy)"))
    } else {
        make(format!("{stem} (copy).{ext}"))
    };
    if !exists(&copy1) {
        return copy1;
    }

    let mut n = 2u32;
    loop {
        let candidate = if ext.is_empty() {
            make(format!("{stem} (copy {n})"))
        } else {
            make(format!("{stem} (copy {n}).{ext}"))
        };
        if !exists(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Copies `src` to the same directory with a non-colliding name.
/// Returns the new path on success.
pub fn duplicate_file(src: &Path) -> Result<PathBuf, String> {
    let parent = src
        .parent()
        .ok_or_else(|| format!("no parent directory for {}", src.display()))?;
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("invalid filename: {}", src.display()))?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");

    let dest = next_available_path(parent, stem, ext, |p| p.exists());
    std::fs::copy(src, &dest).map_err(|e| format!("copy failed: {e}"))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn make_exists(paths: &[PathBuf]) -> impl Fn(&Path) -> bool + '_ {
        let set: HashSet<PathBuf> = paths.iter().cloned().collect();
        move |p| set.contains(p)
    }

    #[test]
    fn no_collision_returns_base() {
        let dir = Path::new("/tmp");
        let result = next_available_path(dir, "foo", "jpg", |_| false);
        assert_eq!(result, dir.join("foo.jpg"));
    }

    #[test]
    fn base_taken_returns_copy() {
        let dir = Path::new("/tmp");
        let result = next_available_path(dir, "foo", "jpg", make_exists(&[dir.join("foo.jpg")]));
        assert_eq!(result, dir.join("foo (copy).jpg"));
    }

    #[test]
    fn copy_taken_returns_copy_2() {
        let dir = Path::new("/tmp");
        let result = next_available_path(
            dir,
            "foo",
            "jpg",
            make_exists(&[dir.join("foo.jpg"), dir.join("foo (copy).jpg")]),
        );
        assert_eq!(result, dir.join("foo (copy 2).jpg"));
    }

    #[test]
    fn no_extension_works() {
        let dir = Path::new("/tmp");
        let result = next_available_path(dir, "foo", "", make_exists(&[dir.join("foo")]));
        assert_eq!(result, dir.join("foo (copy)"));
    }
}
