//! Which subfolders a walk of a library root may enter, shared by the sidebar
//! tree and the library-wide path scan so they cannot disagree.
//!
//! A walk stays on the filesystem it started on, so a root such as Home does
//! not crawl network shares mounted under it (a mount is added on purpose by
//! choosing it, or as its own root), and it skips hidden folders such as
//! `.cache` or `.steam`. The starting folder itself is never filtered.

use std::ffi::OsStr;
use std::fs::DirEntry;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// The filesystem a folder lives on, or `None` if it cannot be read.
pub(crate) fn device_of(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|meta| meta.dev())
}

pub(crate) fn is_hidden(name: &OsStr) -> bool {
    name.as_encoded_bytes().first() == Some(&b'.')
}

/// Pure form of the rule: a subfolder is entered when it is not hidden and
/// is on the same filesystem as where the walk started.
pub(crate) fn may_enter(
    name: &OsStr,
    folder_device: Option<u64>,
    start_device: Option<u64>,
) -> bool {
    !is_hidden(name) && folder_device.is_some() && folder_device == start_device
}

/// Whether a walk that started on `start_device` may descend into this
/// directory entry. Does not follow symbolic links.
pub(crate) fn may_descend_into(entry: &DirEntry, start_device: Option<u64>) -> bool {
    let device = entry.metadata().ok().map(|meta| meta.dev());
    may_enter(&entry.file_name(), device, start_device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_names_start_with_a_dot() {
        assert!(is_hidden(OsStr::new(".cache")));
        assert!(is_hidden(OsStr::new(".steam")));
        assert!(!is_hidden(OsStr::new("Pictures")));
        assert!(!is_hidden(OsStr::new("a.b")));
    }

    #[test]
    fn a_walk_does_not_cross_onto_another_filesystem() {
        let here = Some(64768);
        let other = Some(52);
        assert!(may_enter(OsStr::new("Pictures"), here, here));
        assert!(
            !may_enter(OsStr::new("Gigabyte"), other, here),
            "a share mounted under the root is on another device"
        );
        assert!(!may_enter(OsStr::new("gone"), None, here), "unreadable");
        assert!(!may_enter(OsStr::new(".cache"), here, here), "hidden");
    }

    #[test]
    fn folders_in_one_temp_tree_share_a_device() {
        let root = std::env::temp_dir().join(format!("sharpr-walk-test-{}", std::process::id()));
        let child = root.join("child");
        std::fs::create_dir_all(&child).unwrap();
        assert!(device_of(&root).is_some());
        assert_eq!(device_of(&root), device_of(&child));
        let entry = std::fs::read_dir(&root).unwrap().next().unwrap().unwrap();
        assert!(may_descend_into(&entry, device_of(&root)));
        assert!(!may_descend_into(&entry, None));
        std::fs::remove_dir_all(root).unwrap();
    }
}
