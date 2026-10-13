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

/// Renames `src` to `new_name` in the same directory without ever replacing
/// an existing file. Returns the new path.
pub fn rename_within_dir(src: &Path, new_name: &str) -> Result<PathBuf, String> {
    if new_name.is_empty()
        || new_name == "."
        || new_name == ".."
        || new_name.contains('/')
        || new_name.contains('\0')
    {
        return Err(format!("“{new_name}” is not a valid file name"));
    }
    let parent = src
        .parent()
        .ok_or_else(|| format!("no parent directory for {}", src.display()))?;
    let dest = parent.join(new_name);
    if dest == src {
        return Ok(dest);
    }
    match rename_no_replace(src, &dest) {
        Ok(()) => Ok(dest),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(format!("A file named “{new_name}” already exists"))
        }
        Err(e) => Err(format!("rename failed: {e}")),
    }
}

/// Renames `src` to `dest` unless `dest` exists, in which case it fails with
/// `ErrorKind::AlreadyExists` and leaves both files untouched.
///
/// Uses `renameat2(RENAME_NOREPLACE)`, which the kernel performs atomically.
/// Filesystems without that flag fall back to a hard link, which also fails
/// atomically when the target exists, followed by removing the old name. When
/// neither is available (some FUSE mounts), the rename is refused rather than
/// risk replacing a file another program creates at the same moment. Moves
/// across filesystems fail as with `std::fs::rename`.
pub fn rename_no_replace(src: &Path, dest: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = |p: &Path| {
        std::ffi::CString::new(p.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
    };
    let (c_src, c_dest) = (c_path(src)?, c_path(dest)?);
    // SAFETY: both pointers are valid NUL-terminated strings for the call.
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            c_src.as_ptr(),
            libc::AT_FDCWD,
            c_dest.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if !matches!(error.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS)) {
        return Err(error);
    }
    match std::fs::hard_link(src, dest) {
        Ok(()) => {
            if let Err(e) = std::fs::remove_file(src) {
                // Undo the extra name so the file is left exactly as before.
                let _ = std::fs::remove_file(dest);
                return Err(e);
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this drive cannot rename without risking an overwrite",
        )),
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

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-file-ops-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rename_within_dir_never_replaces_an_existing_file() {
        let dir = temp_dir("no-replace");
        let src = dir.join("a.jpg");
        let other = dir.join("b.jpg");
        std::fs::write(&src, b"original a").unwrap();
        std::fs::write(&other, b"original b").unwrap();

        let err = rename_within_dir(&src, "b.jpg").unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(std::fs::read(&src).unwrap(), b"original a");
        assert_eq!(std::fs::read(&other).unwrap(), b"original b");

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rename_no_replace_moves_between_folders_but_never_onto_a_name() {
        let dir = temp_dir("no-replace-move");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let src = dir.join("a.jpg");
        std::fs::write(&src, b"original a").unwrap();
        // Even a dangling symlink occupies the name.
        let link = sub.join("taken.jpg");
        std::os::unix::fs::symlink(dir.join("missing.jpg"), &link).unwrap();

        let err = rename_no_replace(&src, &link).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&src).unwrap(), b"original a");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());

        let dest = sub.join("a.jpg");
        rename_no_replace(&src, &dest).unwrap();
        assert!(!src.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"original a");

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rename_within_dir_rejects_names_that_leave_the_folder() {
        let dir = temp_dir("invalid");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let src = sub.join("a.jpg");
        std::fs::write(&src, b"a").unwrap();

        for bad in ["", ".", "..", "../a.jpg", "x/a.jpg", "nul\0.jpg"] {
            assert!(rename_within_dir(&src, bad).is_err(), "accepted {bad:?}");
        }
        assert!(src.is_file());
        assert!(!dir.join("a.jpg").exists());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rename_within_dir_moves_content_to_the_new_name() {
        let dir = temp_dir("ok");
        let src = dir.join("a.jpg");
        std::fs::write(&src, b"pixels").unwrap();

        let dest = rename_within_dir(&src, "renamed.jpg").unwrap();
        assert_eq!(dest, dir.join("renamed.jpg"));
        assert!(!src.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"pixels");
        assert_eq!(rename_within_dir(&dest, "renamed.jpg").unwrap(), dest);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn no_extension_works() {
        let dir = Path::new("/tmp");
        let result = next_available_path(dir, "foo", "", make_exists(&[dir.join("foo")]));
        assert_eq!(result, dir.join("foo (copy)"));
    }
}
