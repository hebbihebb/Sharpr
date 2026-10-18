use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Returns the first non-existing path of the form:
///   `dir/stem.ext` → `dir/stem (copy).ext` → `dir/stem (copy 2).ext` → …
///
/// `exists` is a predicate so this can be tested without touching the filesystem.
pub fn next_available_path<F>(dir: &Path, stem: &str, ext: &str, exists: F) -> PathBuf
where
    F: Fn(&Path) -> bool,
{
    (0..)
        .map(|suffix| copy_candidate(dir, stem, ext, suffix))
        .find(|path| !exists(path))
        .expect("an unused copy name exists")
}

fn copy_candidate(dir: &Path, stem: &str, ext: &str, suffix: u32) -> PathBuf {
    let name = match suffix {
        0 => stem.to_string(),
        1 => format!("{stem} (copy)"),
        n => format!("{stem} (copy {n})"),
    };
    dir.join(if ext.is_empty() {
        name
    } else {
        format!("{name}.{ext}")
    })
}

/// Validates a single filename before a file-and-curation relocation.
pub fn rename_destination(src: &Path, new_name: &str) -> Result<PathBuf, String> {
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
    Ok(parent.join(new_name))
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

/// Puts back a file that `gio::File::trash` moved to the trash (D25). It
/// looks in the home trash and in the trash of the drive the file was on,
/// following the freedesktop.org trash spec. The newest entry recorded for
/// `original` moves back, never over a file now at that name.
pub fn restore_from_trash(original: &Path) -> Result<(), String> {
    restore_from_trashes(original, &trashes_for(original))
}

/// The home trash, plus the trash directories of the drive holding `path`.
fn trashes_for(path: &Path) -> Vec<(PathBuf, Option<PathBuf>)> {
    let mut trashes = vec![(glib::user_data_dir().join("Trash"), None)];
    if let Some(top) = mount_top(&lexical(path)) {
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        for dir in [
            top.join(".Trash").join(uid.to_string()),
            top.join(format!(".Trash-{uid}")),
        ] {
            trashes.push((dir, Some(top.clone())));
        }
    }
    trashes
}

/// The original paths of everything in the trashes that can hold files
/// from under `roots`: such a file can still be restored.
pub fn trashed_originals(roots: &[PathBuf]) -> HashSet<PathBuf> {
    let mut trashes: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    for root in roots {
        for trash in trashes_for(&root.join("_")) {
            if !trashes.contains(&trash) {
                trashes.push(trash);
            }
        }
    }
    trash_entries(&trashes)
        .into_iter()
        .map(|entry| entry.original)
        .collect()
}

/// One `.trashinfo` record: where the file came from and where it is now.
struct TrashEntry {
    original: PathBuf,
    date: String,
    file: PathBuf,
    info: PathBuf,
}

/// Every readable record in `trashes`, each paired with the top directory
/// its relative paths start from (`None` for the home trash).
fn trash_entries(trashes: &[(PathBuf, Option<PathBuf>)]) -> Vec<TrashEntry> {
    let mut found = Vec::new();
    for (trash, top) in trashes {
        let Ok(entries) = std::fs::read_dir(trash.join("info")) else {
            continue;
        };
        for entry in entries.flatten() {
            let info = entry.path();
            let Some(name) = info
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".trashinfo"))
            else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&info) else {
                continue;
            };
            let (mut path, mut date) = (None, String::new());
            for line in text.lines() {
                if let Some(value) = line.strip_prefix("Path=") {
                    path = Some(percent_decode(value));
                } else if let Some(value) = line.strip_prefix("DeletionDate=") {
                    date = value.to_string();
                }
            }
            let Some(path) = path else {
                continue;
            };
            let original = match top {
                Some(top) if path.is_relative() => top.join(path),
                _ => path,
            };
            found.push(TrashEntry {
                original,
                date,
                file: trash.join("files").join(name),
                info,
            });
        }
    }
    found
}

/// `path` without `.` and `..` components, resolved as text the way GIO
/// records a trashed file's original path (symbolic links are not followed).
fn lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The top directory of the drive holding `path`'s folder.
fn mount_top(path: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let mut top = path.parent()?.to_path_buf();
    let device = std::fs::metadata(&top).ok()?.dev();
    while let Some(parent) = top.parent() {
        if std::fs::metadata(parent).ok()?.dev() != device {
            break;
        }
        top = parent.to_path_buf();
    }
    Some(top)
}

/// `trashes` pairs each trash directory with the top directory its relative
/// paths start from (`None` for the home trash, whose paths are absolute).
fn restore_from_trashes(
    original: &Path,
    trashes: &[(PathBuf, Option<PathBuf>)],
) -> Result<(), String> {
    let original = lexical(original);
    let original = original.as_path();
    let mut newest: Option<(String, PathBuf, PathBuf)> = None;
    for entry in trash_entries(trashes) {
        // ISO 8601 dates sort as text.
        if entry.original == original && newest.as_ref().is_none_or(|(d, _, _)| entry.date > *d) {
            newest = Some((entry.date, entry.file, entry.info));
        }
    }
    let Some((_, trashed, info)) = newest else {
        return Err("it is no longer in the trash".into());
    };
    rename_no_replace(&trashed, original).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => "another file now has its name".to_string(),
        std::io::ErrorKind::NotFound => {
            "it is no longer in the trash, or its folder is gone".to_string()
        }
        _ => e.to_string(),
    })?;
    // The file is back; a leftover record would only list a missing item.
    let _ = std::fs::remove_file(info);
    Ok(())
}

/// Decodes the `%XX` escapes of a `.trashinfo` path into raw bytes.
fn percent_decode(value: &str) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

/// Copies `src` to the same directory with a non-colliding name.
/// Returns the new path on success.
pub fn duplicate_file(src: &Path) -> Result<PathBuf, String> {
    let parent = src
        .parent()
        .ok_or_else(|| format!("no parent directory for {}", src.display()))?;
    copy_to_folder(src, parent)
}

/// Copies bytes through an exclusively owned partial file, then publishes
/// under a unique name without replacing files or following destination links.
pub fn copy_to_folder(src: &Path, destination: &Path) -> Result<PathBuf, String> {
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("invalid filename: {}", src.display()))?
        .to_string();
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();
    let mut source = std::fs::File::open(src).map_err(|e| format!("copy failed: {e}"))?;
    let destination = destination.to_path_buf();
    let mut pending = crate::export::Pending::reserve(move |suffix| {
        copy_candidate(&destination, &stem, &ext, suffix)
    })
    .map_err(|e| e.to_string())?;
    pending.copy_from(&mut source).map_err(|e| e.to_string())?;
    pending.publish().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Renames `src` to `new_name` in the same directory without ever replacing
    /// an existing file. Returns the new path.
    fn rename_within_dir(src: &Path, new_name: &str) -> Result<PathBuf, String> {
        let dest = rename_destination(src, new_name)?;
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
    fn duplicate_skips_dangling_symlinks_and_preserves_original_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("copy-symlink");
        let src = dir.join("a.png");
        let unexpected = dir.join("must-not-create.png");
        let occupied = dir.join("a (copy).png");
        std::fs::write(&src, b"original pixels").unwrap();
        std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::symlink(&unexpected, &occupied).unwrap();

        let copy = duplicate_file(&src).unwrap();

        assert_eq!(copy, dir.join("a (copy 2).png"));
        assert!(
            !unexpected.exists(),
            "an occupied symlink must never be followed"
        );
        assert!(occupied
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&src).unwrap(), b"original pixels");
        assert_eq!(std::fs::read(&copy).unwrap(), b"original pixels");
        assert_eq!(
            std::fs::metadata(copy).unwrap().permissions().mode() & 0o777,
            0o640
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_folder_copies_keep_every_output_and_existing_file() {
        let dir = temp_dir("concurrent-copy");
        let destination = dir.join("out");
        std::fs::create_dir(&destination).unwrap();
        let src = dir.join("a.png");
        let bytes = vec![73; 1024 * 1024];
        std::fs::write(&src, &bytes).unwrap();
        std::fs::write(destination.join("a.png"), b"existing file").unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let (src, destination, barrier) =
                    (src.clone(), destination.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    copy_to_folder(&src, &destination).unwrap()
                })
            })
            .collect();
        let copies: HashSet<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(copies.len(), 8);
        for copy in copies {
            assert_eq!(std::fs::read(copy).unwrap(), bytes);
        }
        assert_eq!(std::fs::read(&src).unwrap(), bytes);
        assert_eq!(
            std::fs::read(destination.join("a.png")).unwrap(),
            b"existing file"
        );
        assert_eq!(std::fs::read_dir(destination).unwrap().count(), 9);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_folder_copy_removes_only_its_partial() {
        let dir = temp_dir("copy-failed");
        let source = dir.join("source");
        let destination = dir.join("out");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        // Opening a directory succeeds on Linux, but reading its bytes fails.
        assert!(copy_to_folder(&source, &destination).is_err());
        assert!(source.is_dir());
        assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
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

    /// Lays out a trash entry as GIO does: the file under `files/`, its
    /// record under `info/`.
    fn trash_entry(trash: &Path, name: &str, recorded: &str, date: &str, bytes: &[u8]) {
        std::fs::create_dir_all(trash.join("files")).unwrap();
        std::fs::create_dir_all(trash.join("info")).unwrap();
        std::fs::write(trash.join("files").join(name), bytes).unwrap();
        std::fs::write(
            trash.join("info").join(format!("{name}.trashinfo")),
            format!("[Trash Info]\nPath={recorded}\nDeletionDate={date}\n"),
        )
        .unwrap();
    }

    #[test]
    fn restore_puts_the_newest_trashed_copy_back() {
        let dir = temp_dir("restore");
        let trash = dir.join("Trash");
        let folder = dir.join("my pictures");
        std::fs::create_dir_all(&folder).unwrap();
        let recorded = folder
            .join("sea view.png")
            .to_str()
            .unwrap()
            .replace(' ', "%20");
        // GIO records the path without `..`; the caller's may still have it.
        let original = folder.join("../my pictures/./sea view.png");
        trash_entry(
            &trash,
            "sea view.png",
            &recorded,
            "2026-10-14T09:00:00",
            b"older",
        );
        trash_entry(
            &trash,
            "sea view.2.png",
            &recorded,
            "2026-10-15T01:00:00",
            b"newest",
        );

        restore_from_trashes(&original, &[(trash.clone(), None)]).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), b"newest");
        assert!(!trash.join("files/sea view.2.png").exists());
        assert!(!trash.join("info/sea view.2.png.trashinfo").exists());
        assert!(
            trash.join("files/sea view.png").exists(),
            "the older one stays"
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restore_never_replaces_a_file_at_the_original_name() {
        let dir = temp_dir("restore-taken");
        let trash = dir.join("Trash");
        let original = dir.join("a.jpg");
        trash_entry(
            &trash,
            "a.jpg",
            original.to_str().unwrap(),
            "2026-10-15T01:00:00",
            b"trashed",
        );
        std::fs::write(&original, b"new file").unwrap();

        let error = restore_from_trashes(&original, &[(trash.clone(), None)]).unwrap_err();
        assert_eq!(error, "another file now has its name");
        assert_eq!(std::fs::read(&original).unwrap(), b"new file");
        assert_eq!(
            std::fs::read(trash.join("files/a.jpg")).unwrap(),
            b"trashed"
        );
        assert!(
            trash.join("info/a.jpg.trashinfo").exists(),
            "still restorable"
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restore_reads_drive_trashes_relative_to_their_top() {
        let dir = temp_dir("restore-drive");
        let trash = dir.join(".Trash-1000");
        std::fs::create_dir_all(dir.join("photos")).unwrap();
        let original = dir.join("photos/b.webp");
        trash_entry(
            &trash,
            "b.webp",
            "photos/b.webp",
            "2026-10-15T01:00:00",
            b"b",
        );

        let missing = dir.join("photos/c.webp");
        assert_eq!(
            restore_from_trashes(&missing, &[(trash.clone(), Some(dir.clone()))]).unwrap_err(),
            "it is no longer in the trash"
        );
        restore_from_trashes(&original, &[(trash, Some(dir.clone()))]).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), b"b");

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn trashinfo_paths_decode_escapes_to_raw_bytes() {
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(
            percent_decode("/a%20b/%C3%A9.png"),
            PathBuf::from("/a b/é.png")
        );
        assert_eq!(percent_decode("/x%FF").as_os_str().as_bytes(), b"/x\xff");
        assert_eq!(percent_decode("/100%"), PathBuf::from("/100%"));
    }
}
