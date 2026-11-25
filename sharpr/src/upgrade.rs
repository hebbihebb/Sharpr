//! Replace with upgrade (D49): pair upgraded images with their originals,
//! then, only after the user confirms, swap them. The new file takes the
//! original's place (the original's name with its own extension), the
//! original's bucket membership moves to it, and the original goes to the
//! trash, with an Undo that puts everything back. GTK-free; the swap runs on
//! the file-jobs thread.
//!
//! Matching uses the stored visual-search vectors and Show Similar's Exact
//! rule (`copies::version_of`: at least 0.95 alike with matching 32×32
//! versions), which accepts resized, converted and lightly cropped copies.
//! Only strong matches are proposed; weak ones are offered, labelled weak,
//! when the user asks to pick an original by hand. Nothing is paired
//! silently: several strong originals make the user choose.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::copies::{version_of, Readers, MAYBE_SAME};
use crate::library_index::LibraryIndex;
use crate::visual_search::similarity;

/// How many weak candidates "Pick original…" shows.
pub const WEAK_SHOWN: usize = 5;
/// At most this many of the closest images are checked pixel by pixel.
const STRONG_CHECKED: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    pub fingerprint: String,
    pub score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoMatch {
    /// The new image has no fingerprint or vector yet.
    NotIndexed,
    /// Nothing in the library is close enough to be its original.
    NoStrongMatch,
    /// The only close match is an identical copy of the new file itself.
    OnlyIdenticalCopies,
}

impl NoMatch {
    pub fn words(self) -> &'static str {
        match self {
            Self::NotIndexed => "not indexed for visual search yet",
            Self::NoStrongMatch => "no original is close enough",
            Self::OnlyIdenticalCopies => "only identical copies of this file were found",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Match {
    /// One strong original.
    Paired(Candidate),
    /// Several strong originals, closest first: the user picks one.
    Choose(Vec<Candidate>),
    /// No strong original; `weak` holds the closest few, never paired
    /// unless the user picks one.
    Unmatched { why: NoMatch, weak: Vec<Candidate> },
}

/// Finds each new image's original among `library` (path, fingerprint:
/// current files in enabled folders), using `vectors` (fingerprint → unit
/// vector). `new` holds the new images with their fingerprints, `None`
/// when they have none yet. The new images, and identical copies of them,
/// are never candidates.
pub fn find_originals(
    new: &[(PathBuf, Option<String>)],
    library: &[(PathBuf, String)],
    vectors: &HashMap<String, Vec<f32>>,
    read: &Readers,
) -> Vec<(PathBuf, Match)> {
    let new_paths: std::collections::HashSet<&Path> =
        new.iter().map(|(p, _)| p.as_path()).collect();
    let new_prints: std::collections::HashSet<&str> =
        new.iter().filter_map(|(_, f)| f.as_deref()).collect();
    new.iter()
        .map(|(path, fingerprint)| {
            let found = match fingerprint
                .as_deref()
                .and_then(|f| Some((f, vectors.get(f)?)))
            {
                None => Match::Unmatched {
                    why: NoMatch::NotIndexed,
                    weak: Vec::new(),
                },
                Some((fingerprint, vector)) => {
                    let mut identical = false;
                    let mut ranked: Vec<Candidate> = library
                        .iter()
                        .filter(|(p, f)| {
                            if new_paths.contains(p.as_path()) {
                                return false;
                            }
                            if f == fingerprint || new_prints.contains(f.as_str()) {
                                identical |= f == fingerprint;
                                return false;
                            }
                            true
                        })
                        .filter_map(|(p, f)| {
                            Some(Candidate {
                                path: p.clone(),
                                fingerprint: f.clone(),
                                score: similarity(vector, vectors.get(f)?),
                            })
                        })
                        .collect();
                    ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.path.cmp(&b.path)));
                    let (strong, weak): (Vec<Candidate>, Vec<Candidate>) = ranked
                        .into_iter()
                        .take(STRONG_CHECKED.max(WEAK_SHOWN))
                        .partition(|c| {
                            c.score >= MAYBE_SAME
                                && version_of(
                                    (path, fingerprint),
                                    (&c.path, &c.fingerprint),
                                    c.score,
                                    read,
                                )
                        });
                    match strong.len() {
                        1 => Match::Paired(strong.into_iter().next().unwrap()),
                        0 => Match::Unmatched {
                            why: if identical {
                                NoMatch::OnlyIdenticalCopies
                            } else {
                                NoMatch::NoStrongMatch
                            },
                            weak: weak.into_iter().take(WEAK_SHOWN).collect(),
                        },
                        _ => Match::Choose(strong),
                    }
                }
            };
            (path.clone(), found)
        })
        .collect()
}

/// The images `paths` with their fresh fingerprints, and every current,
/// fingerprinted image in `library_paths`, from the index's cache.
pub fn fingerprints_for(
    index: &LibraryIndex,
    paths: &[PathBuf],
    library_paths: &[PathBuf],
) -> Result<(Vec<(PathBuf, Option<String>)>, Vec<(PathBuf, String)>), String> {
    let cached = index.cached_fingerprints().map_err(|e| e.to_string())?;
    let fresh = |path: &Path| -> Option<String> {
        let metadata = std::fs::metadata(path).ok()?;
        let (size, mtime, hash) = cached.get(path.to_string_lossy().as_ref())?;
        ((*size, *mtime) == crate::fingerprint::file_stamp(&metadata)).then(|| hash.clone())
    };
    let new = paths.iter().map(|p| (p.clone(), fresh(p))).collect();
    let library = library_paths
        .iter()
        .filter_map(|p| Some((p.clone(), fresh(p)?)))
        .collect();
    Ok((new, library))
}

// ---------------------------------------------------------------------------
// The swap
// ---------------------------------------------------------------------------

/// One confirmed replacement.
#[derive(Debug, Clone, PartialEq)]
pub struct Pair {
    /// The upgraded image, where it is now.
    pub new: PathBuf,
    /// The original it replaces.
    pub original: PathBuf,
    /// Other matched copies the user ticked to trash as well.
    pub also_trash: Vec<PathBuf>,
    /// (size, mtime) of `new` and `original` when the user reviewed them.
    pub new_stamp: (u64, i64),
    pub original_stamp: (u64, i64),
}

/// What a finished replacement did, so Undo can put it back.
#[derive(Debug, Clone, PartialEq)]
pub struct Replaced {
    pub new_from: PathBuf,
    pub placed: PathBuf,
    pub original: PathBuf,
    pub also_trashed: Vec<PathBuf>,
    /// Bucket rows removed and added, for Undo.
    pub removed_rows: Vec<crate::library_index::MemberRecord>,
    pub added_rows: Vec<crate::library_index::MemberRecord>,
}

/// File-system steps, so tests can make each one fail.
pub struct FileSystem<'a> {
    pub rename: &'a dyn Fn(&Path, &Path) -> std::io::Result<()>,
    pub trash: &'a dyn Fn(&Path) -> Result<(), String>,
    pub restore: &'a dyn Fn(&Path) -> Result<(), String>,
}

impl FileSystem<'static> {
    pub fn real() -> Self {
        FileSystem {
            rename: &|from, to| crate::file_ops::rename_no_replace(from, to),
            trash: &|path| {
                use gtk4::gio::prelude::FileExt;
                gtk4::gio::File::for_path(path)
                    .trash(None::<&gtk4::gio::Cancellable>)
                    .map_err(|e| e.to_string())
            },
            restore: &|path| crate::file_ops::restore_from_trash(path),
        }
    }
}

fn stamp(path: &Path) -> Option<(u64, i64)> {
    std::fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| crate::fingerprint::file_stamp(&m))
}

/// Checks a pair before anything is changed: both files unchanged since
/// review, on the same drive, and the original's folder writable.
pub fn preflight(pair: &Pair) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    if stamp(&pair.new) != Some(pair.new_stamp) {
        return Err("the new image changed or moved since you reviewed it".into());
    }
    if stamp(&pair.original) != Some(pair.original_stamp) {
        return Err("the original changed or moved since you reviewed it".into());
    }
    let folder = pair.original.parent().ok_or("the original has no folder")?;
    let device = |p: &Path| std::fs::metadata(p).map(|m| m.dev()).ok();
    if device(&pair.new) != device(folder) {
        return Err("on another drive; move it next to the original first".into());
    }
    let c_folder = std::ffi::CString::new(folder.as_os_str().as_encoded_bytes())
        .map_err(|_| "the original's folder has an unusable name")?;
    // SAFETY: a valid NUL-terminated path; access only reads it.
    if unsafe { libc::access(c_folder.as_ptr(), libc::W_OK) } != 0 {
        return Err("the original's folder can't be written to".into());
    }
    Ok(())
}

/// Where the new file goes: the original's folder, the original's name with
/// the new file's own extension. The original's own path is allowed (it is
/// being replaced); another file, or a name buckets or tags still keep,
/// gets a safe suffix.
pub fn target_path(pair: &Pair, taken: &dyn Fn(&Path) -> bool) -> PathBuf {
    let folder = pair.original.parent().unwrap_or(Path::new("/"));
    let stem = pair
        .original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let extension = pair
        .new
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();
    let preferred = if extension.is_empty() {
        folder.join(stem)
    } else {
        folder.join(format!("{stem}.{extension}"))
    };
    if preferred == pair.original || preferred == pair.new {
        return preferred;
    }
    crate::file_ops::next_available_path(folder, stem, &extension, |p| {
        p != pair.new && (p.exists() || taken(p))
    })
}

/// A hidden, unused name in `folder` for the moment between trashing the
/// original and moving the new file to its name.
fn temporary_path(folder: &Path, extension: Option<&std::ffi::OsStr>) -> PathBuf {
    for n in 0.. {
        let mut name = format!(".sharpr-replacing-{}-{n}", std::process::id());
        if let Some(extension) = extension.and_then(|e| e.to_str()) {
            name.push('.');
            name.push_str(extension);
        }
        let path = folder.join(name);
        if !path.exists() {
            return path;
        }
    }
    unreachable!()
}

/// Replaces one pair: preflight, then move the new file to its final name
/// and trash the original last (or, when the final name is the original's
/// own path, move the new file to a temporary name, trash the original and
/// rename into place), then move bucket membership to the new file. Every
/// failure puts back what had been done.
pub fn replace(
    index: &LibraryIndex,
    tags: &crate::tags::TagDatabase,
    pair: &Pair,
    fs: &FileSystem,
) -> Result<Replaced, String> {
    preflight(pair)?;
    let taken = |p: &Path| {
        index.retains_bucket_path(p).unwrap_or(true)
            || tags
                .tags_for_path_checked(p)
                .map(|t| !t.is_empty())
                .unwrap_or(true)
    };
    let placed = target_path(pair, &taken);
    let rename = |from: &Path, to: &Path| {
        (fs.rename)(from, to).map_err(|e| format!("couldn't move {}: {e}", from.display()))
    };
    let new_fingerprint = index.cached_fingerprints().ok().and_then(|c| {
        c.get(pair.new.to_string_lossy().as_ref())
            .map(|r| r.2.clone())
    });

    if placed == pair.original {
        let temporary = temporary_path(
            pair.original.parent().unwrap_or(Path::new("/")),
            pair.new.extension(),
        );
        rename(&pair.new, &temporary)?;
        if let Err(error) = (fs.trash)(&pair.original) {
            return Err(match rename(&temporary, &pair.new) {
                Ok(()) => format!("couldn't move the original to the trash: {error}"),
                Err(back) => format!("couldn't move the original to the trash: {error}; {back}"),
            });
        }
        if let Err(error) = rename(&temporary, &placed) {
            let restored =
                (fs.restore)(&pair.original).and_then(|()| rename(&temporary, &pair.new));
            return Err(match restored {
                Ok(()) => error,
                Err(back) => format!("{error}; putting things back failed: {back}"),
            });
        }
    } else {
        rename(&pair.new, &placed)?;
        if let Err(error) = (fs.trash)(&pair.original) {
            return Err(match rename(&placed, &pair.new) {
                Ok(()) => format!("couldn't move the original to the trash: {error}"),
                Err(back) => format!(
                    "the new image is at {}, but the original couldn't go to the trash: {error}; {back}",
                    placed.display()
                ),
            });
        }
    }

    // The files are in place; now the index follows.
    let records = index.replace_identity(
        &pair.original,
        &pair.also_trash,
        &pair.new,
        &placed,
        new_fingerprint.as_deref(),
    );
    let (removed_rows, added_rows) = match records {
        Ok(rows) => rows,
        Err(error) => {
            let back = rename(&placed, &pair.new).and_then(|()| (fs.restore)(&pair.original));
            return Err(match back {
                Ok(()) => format!("couldn't move bucket membership: {error}"),
                Err(back) => format!(
                    "couldn't move bucket membership: {error}; putting things back failed: {back}"
                ),
            });
        }
    };
    let mut also_trashed = Vec::new();
    for extra in &pair.also_trash {
        if stamp(extra).is_some() && (fs.trash)(extra).is_ok() {
            also_trashed.push(extra.clone());
        }
    }
    Ok(Replaced {
        new_from: pair.new.clone(),
        placed,
        original: pair.original.clone(),
        also_trashed,
        removed_rows,
        added_rows,
    })
}

/// Undoes one replacement: moves the new file back, restores the original
/// and the other trashed copies from the trash, and puts the bucket rows
/// back. Stops at the first step that can't be done, changing nothing more.
pub fn undo(index: &LibraryIndex, done: &Replaced, fs: &FileSystem) -> Result<(), String> {
    if done.new_from.exists() {
        return Err(format!(
            "another file now has the name {}",
            done.new_from.display()
        ));
    }
    (fs.rename)(&done.placed, &done.new_from)
        .map_err(|e| format!("couldn't move {} back: {e}", done.placed.display()))?;
    (fs.restore)(&done.original)?;
    let mut failed = Vec::new();
    for extra in &done.also_trashed {
        if let Err(error) = (fs.restore)(extra) {
            failed.push(format!("{}: {error}", extra.display()));
        }
    }
    index
        .undo_replace_identity(
            &done.placed,
            &done.new_from,
            &done.removed_rows,
            &done.added_rows,
        )
        .map_err(|e| format!("files restored, but bucket membership couldn't be put back: {e}"))?;
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("couldn't restore {}", failed.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::TagDatabase;
    use std::cell::Cell;

    fn unit(seed: &[f32]) -> Vec<f32> {
        let mut v = vec![0.0f32; 8];
        v[..seed.len()].copy_from_slice(seed);
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect()
    }

    fn readers<'a>(tiny: &'a (dyn Fn(&Path) -> Option<Vec<u8>> + Sync)) -> Readers<'a> {
        Readers {
            size_of: &|_| None,
            tiny,
            canceled: &|| false,
            progress: &|_, _| {},
        }
    }

    /// A library where `new.png` is an upscale of `wall/orig.jpg` (same
    /// picture), `wall/other.jpg` is a different picture with a close
    /// vector, and `copy.png` is an identical copy of the new file.
    fn library() -> (HashMap<String, Vec<f32>>, Vec<(PathBuf, String)>) {
        let vectors = HashMap::from([
            ("new".to_string(), unit(&[1.0, 0.0])),
            ("orig".to_string(), unit(&[1.0, 0.05])),
            ("other".to_string(), unit(&[1.0, 0.2])),
            ("far".to_string(), unit(&[0.0, 1.0])),
        ]);
        let library = vec![
            (PathBuf::from("/l/new.png"), "new".to_string()),
            (PathBuf::from("/l/wall/orig.jpg"), "orig".to_string()),
            (PathBuf::from("/l/wall/other.jpg"), "other".to_string()),
            (PathBuf::from("/l/wall/far.jpg"), "far".to_string()),
            (PathBuf::from("/l/copy.png"), "new".to_string()),
        ];
        (vectors, library)
    }

    /// 32×32 versions: the new image and its original look the same;
    /// "other" differs; a crop of the original differs a little.
    fn tiny(path: &Path) -> Option<Vec<u8>> {
        let name = path.file_stem()?.to_str()?;
        let value = match name {
            "new" | "orig" | "copy" => 100,
            "crop" => 110,
            "other" => 180,
            _ => 30,
        };
        Some(vec![value; 32 * 32 * 3])
    }

    #[test]
    fn an_upgrade_pairs_with_its_original_only() {
        let (vectors, library) = library();
        let new = [(PathBuf::from("/l/new.png"), Some("new".to_string()))];
        let found = find_originals(&new, &library, &vectors, &readers(&tiny));
        let Match::Paired(original) = &found[0].1 else {
            panic!("{found:?}");
        };
        assert_eq!(original.path, PathBuf::from("/l/wall/orig.jpg"));
    }

    #[test]
    fn several_strong_originals_make_the_user_choose() {
        let (mut vectors, mut library) = library();
        vectors.insert("crop".into(), unit(&[1.0, 0.08]));
        library.push((PathBuf::from("/l/needs/crop.webp"), "crop".into()));
        let new = [(PathBuf::from("/l/new.png"), Some("new".to_string()))];
        let found = find_originals(&new, &library, &vectors, &readers(&tiny));
        let Match::Choose(candidates) = &found[0].1 else {
            panic!("{found:?}");
        };
        let paths: Vec<_> = candidates.iter().map(|c| c.path.clone()).collect();
        assert_eq!(
            paths,
            [
                PathBuf::from("/l/wall/orig.jpg"),
                PathBuf::from("/l/needs/crop.webp")
            ]
        );
    }

    #[test]
    fn a_weak_match_is_never_paired_but_offered() {
        let (vectors, mut library) = library();
        library.retain(|(p, _)| p != Path::new("/l/wall/orig.jpg"));
        let new = [(PathBuf::from("/l/new.png"), Some("new".to_string()))];
        let found = find_originals(&new, &library, &vectors, &readers(&tiny));
        let Match::Unmatched { why, weak } = &found[0].1 else {
            panic!("{found:?}");
        };
        assert_eq!(*why, NoMatch::OnlyIdenticalCopies);
        assert_eq!(weak[0].path, PathBuf::from("/l/wall/other.jpg"));
        assert!(weak.len() <= WEAK_SHOWN);
        assert!(weak.iter().all(|c| c.path != Path::new("/l/copy.png")));
    }

    #[test]
    fn an_unindexed_image_says_so() {
        let (vectors, library) = library();
        let new = [
            (PathBuf::from("/l/fresh.png"), None),
            (
                PathBuf::from("/l/novector.png"),
                Some("unknown".to_string()),
            ),
        ];
        let found = find_originals(&new, &library, &vectors, &readers(&tiny));
        for (_, m) in found {
            assert_eq!(
                m,
                Match::Unmatched {
                    why: NoMatch::NotIndexed,
                    weak: Vec::new()
                }
            );
        }
    }

    #[test]
    fn other_new_images_are_never_originals() {
        let (vectors, library) = library();
        let new = [
            (PathBuf::from("/l/new.png"), Some("new".to_string())),
            (PathBuf::from("/l/wall/orig.jpg"), Some("orig".to_string())),
        ];
        let found = find_originals(&new, &library, &vectors, &readers(&tiny));
        assert!(found
            .iter()
            .all(|(_, m)| !matches!(m, Match::Paired(_) | Match::Choose(_))));
    }

    // -- the swap ------------------------------------------------------------

    struct Fixture {
        root: PathBuf,
        bin: PathBuf,
        index: LibraryIndex,
        tags: TagDatabase,
    }

    fn fixture(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "sharpr-upgrade-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let index = LibraryIndex::open_in_memory().unwrap();
        let tags = TagDatabase::open_in_memory().unwrap();
        crate::buckets::migrate(&index, &tags, &root.join("backups")).unwrap();
        Fixture {
            root,
            bin,
            index,
            tags,
        }
    }

    impl Fixture {
        fn file(&self, rel: &str, bytes: &[u8], fingerprint: &str) -> PathBuf {
            let path = self.root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            let (size, mtime) = stamp(&path).unwrap();
            self.index
                .store_fingerprints(&[(path.clone(), size, mtime, fingerprint.to_string())])
                .unwrap();
            path
        }

        fn pair(&self, new: &Path, original: &Path) -> Pair {
            Pair {
                new: new.to_path_buf(),
                original: original.to_path_buf(),
                also_trash: Vec::new(),
                new_stamp: stamp(new).unwrap(),
                original_stamp: stamp(original).unwrap(),
            }
        }

        fn bin_trash(&self) -> impl Fn(&Path) -> Result<(), String> + '_ {
            move |path: &Path| {
                std::fs::rename(path, self.bin.join(path.file_name().unwrap()))
                    .map_err(|e| e.to_string())
            }
        }

        fn bin_restore(&self) -> impl Fn(&Path) -> Result<(), String> + '_ {
            move |path: &Path| {
                if path.exists() {
                    return Err("another file now has its name".into());
                }
                std::fs::rename(self.bin.join(path.file_name().unwrap()), path)
                    .map_err(|e| e.to_string())
            }
        }

        /// Every member row, sorted, as (bucket, path, fingerprint).
        fn members(&self) -> Vec<(i64, PathBuf, Option<String>)> {
            let mut rows: Vec<_> = self
                .index
                .all_bucket_members()
                .unwrap()
                .into_iter()
                .map(|r| (r.collection_id, r.last_path, r.fingerprint))
                .collect();
            rows.sort();
            rows
        }

        fn bucket(&self, name: &str, members: &[&Path]) -> i64 {
            let id = self
                .index
                .create_collection("test", None, name, &[], None, None)
                .unwrap()
                .id;
            let rows: Vec<(PathBuf, Option<String>)> = members
                .iter()
                .map(|p| {
                    let fp = self
                        .index
                        .cached_fingerprints()
                        .unwrap()
                        .get(p.to_string_lossy().as_ref())
                        .map(|r| r.2.clone());
                    (p.to_path_buf(), fp)
                })
                .collect();
            self.index.add_bucket_members(id, &rows).unwrap();
            id
        }
    }

    fn real_rename(from: &Path, to: &Path) -> std::io::Result<()> {
        crate::file_ops::rename_no_replace(from, to)
    }

    #[test]
    fn replace_puts_the_upgrade_in_place_with_buckets_and_undo_restores_all() {
        let f = fixture("swap");
        let original = f.file("Wallpapers/sunset.jpg", b"original bytes", "orig");
        let new = f.file("upscale/sunset_seedvr2_00001.png", b"upscaled", "new");
        let kept = f.bucket("Kept", &[&original]);
        let other = f.bucket("Other", &[&original]);
        let before = f.members();
        let (trash, restore) = (f.bin_trash(), f.bin_restore());
        let fs = FileSystem {
            rename: &real_rename,
            trash: &trash,
            restore: &restore,
        };
        let done = replace(&f.index, &f.tags, &f.pair(&new, &original), &fs).unwrap();
        let placed = f.root.join("Wallpapers/sunset.png");
        assert_eq!(done.placed, placed);
        assert_eq!(std::fs::read(&placed).unwrap(), b"upscaled");
        assert!(!original.exists() && !new.exists());
        // The original's bytes are untouched in the trash.
        assert_eq!(
            std::fs::read(f.bin.join("sunset.jpg")).unwrap(),
            b"original bytes"
        );
        assert_eq!(
            f.members(),
            vec![
                (kept, placed.clone(), Some("new".into())),
                (other, placed.clone(), Some("new".into()))
            ]
        );
        let cached = f.index.cached_fingerprints().unwrap();
        assert_eq!(cached[placed.to_string_lossy().as_ref()].2, "new");

        undo(&f.index, &done, &fs).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), b"original bytes");
        assert_eq!(std::fs::read(&new).unwrap(), b"upscaled");
        assert!(!placed.exists());
        assert_eq!(f.members(), before);
    }

    #[test]
    fn a_same_extension_upgrade_takes_the_identical_path() {
        let f = fixture("same-path");
        let original = f.file("Wallpapers/sunset.png", b"original bytes", "orig");
        let new = f.file("upscale/sunset.png", b"upscaled", "new");
        let kept = f.bucket("Kept", &[&original]);
        let (trash, restore) = (f.bin_trash(), f.bin_restore());
        let fs = FileSystem {
            rename: &real_rename,
            trash: &trash,
            restore: &restore,
        };
        let done = replace(&f.index, &f.tags, &f.pair(&new, &original), &fs).unwrap();
        assert_eq!(done.placed, original);
        assert_eq!(std::fs::read(&original).unwrap(), b"upscaled");
        assert_eq!(
            f.members(),
            vec![(kept, original.clone(), Some("new".into()))]
        );
        // No temporary file is left behind.
        let left: Vec<_> = std::fs::read_dir(original.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("sunset.png")]);

        undo(&f.index, &done, &fs).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), b"original bytes");
        assert_eq!(std::fs::read(&new).unwrap(), b"upscaled");
        assert_eq!(
            f.members(),
            vec![(kept, original.clone(), Some("orig".into()))]
        );
    }

    #[test]
    fn a_name_clash_gets_a_safe_suffix() {
        let f = fixture("clash");
        let original = f.file("Wallpapers/sunset.jpg", b"original", "orig");
        let squatter = f.file("Wallpapers/sunset.png", b"someone else", "sq");
        let new = f.file("upscale/x.png", b"upscaled", "new");
        let (trash, restore) = (f.bin_trash(), f.bin_restore());
        let fs = FileSystem {
            rename: &real_rename,
            trash: &trash,
            restore: &restore,
        };
        let done = replace(&f.index, &f.tags, &f.pair(&new, &original), &fs).unwrap();
        assert_eq!(done.placed, f.root.join("Wallpapers/sunset (copy).png"));
        assert_eq!(std::fs::read(&squatter).unwrap(), b"someone else");
    }

    #[test]
    fn a_changed_file_stops_the_pair_before_anything_happens() {
        let f = fixture("stale");
        let original = f.file("Wallpapers/a.jpg", b"original", "orig");
        let new = f.file("upscale/a.png", b"upscaled", "new");
        let pair = f.pair(&new, &original);
        std::fs::write(&original, b"edited since review").unwrap();
        let (trash, restore) = (f.bin_trash(), f.bin_restore());
        let fs = FileSystem {
            rename: &real_rename,
            trash: &trash,
            restore: &restore,
        };
        let error = replace(&f.index, &f.tags, &pair, &fs).unwrap_err();
        assert!(error.contains("changed"), "{error}");
        assert!(original.exists() && new.exists());
    }

    #[test]
    fn a_read_only_folder_stops_the_pair_before_anything_happens() {
        use std::os::unix::fs::PermissionsExt;
        let f = fixture("read-only");
        let original = f.file("Wallpapers/a.jpg", b"original", "orig");
        let new = f.file("upscale/a.png", b"upscaled", "new");
        let folder = original.parent().unwrap();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root may write anyway; the check is meaningless there.
        let writable = unsafe { libc::geteuid() } == 0;
        let result = preflight(&f.pair(&new, &original));
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        if !writable {
            assert!(result.unwrap_err().contains("can't be written"));
        }
    }

    #[test]
    fn another_drive_stops_the_pair() {
        use std::os::unix::fs::MetadataExt;
        let f = fixture("drive");
        let original = f.file("Wallpapers/a.jpg", b"original", "orig");
        let shm = PathBuf::from("/dev/shm");
        let differs = std::fs::metadata(&shm)
            .ok()
            .zip(std::fs::metadata(&f.root).ok())
            .is_some_and(|(a, b)| a.dev() != b.dev());
        if !differs {
            return; // No second filesystem to test with here.
        }
        let new = shm.join(format!("sharpr-upgrade-{}.png", std::process::id()));
        std::fs::write(&new, b"upscaled").unwrap();
        let result = preflight(&Pair {
            new: new.clone(),
            original: original.clone(),
            also_trash: Vec::new(),
            new_stamp: stamp(&new).unwrap(),
            original_stamp: stamp(&original).unwrap(),
        });
        std::fs::remove_file(&new).unwrap();
        assert!(result.unwrap_err().contains("another drive"));
    }

    /// Runs a replacement whose `fail_rename`th rename (1-based) or whose
    /// trash fails, and checks that every file and row is as it started.
    fn assert_failure_restores(name: &str, same_path: bool, fail_rename: usize, fail_trash: bool) {
        let f = fixture(name);
        let original_name = if same_path {
            "Wallpapers/a.png"
        } else {
            "Wallpapers/a.jpg"
        };
        let original = f.file(original_name, b"original", "orig");
        let new = f.file("upscale/a.png", b"upscaled", "new");
        f.bucket("Kept", &[&original]);
        let before = f.members();
        let renames = Cell::new(0);
        let rename = |from: &Path, to: &Path| {
            renames.set(renames.get() + 1);
            if renames.get() == fail_rename {
                return Err(std::io::Error::other("injected"));
            }
            real_rename(from, to)
        };
        let bin_trash = f.bin_trash();
        let trash = |path: &Path| {
            if fail_trash {
                return Err("injected".to_string());
            }
            bin_trash(path)
        };
        let restore = f.bin_restore();
        let fs = FileSystem {
            rename: &rename,
            trash: &trash,
            restore: &restore,
        };
        let result = replace(&f.index, &f.tags, &f.pair(&new, &original), &fs);
        assert!(result.is_err(), "{name}: {result:?}");
        assert_eq!(std::fs::read(&original).unwrap(), b"original", "{name}");
        assert_eq!(std::fs::read(&new).unwrap(), b"upscaled", "{name}");
        assert_eq!(f.members(), before, "{name}");
        let wall: Vec<_> = std::fs::read_dir(original.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(wall.len(), 1, "{name}: {wall:?}");
    }

    #[test]
    fn every_failure_point_puts_things_back() {
        // Moving the new file to its final name fails.
        assert_failure_restores("move-fails", false, 1, false);
        // The trash fails after the move.
        assert_failure_restores("trash-fails", false, 0, true);
        // Same path: the move to the temporary name fails.
        assert_failure_restores("temp-move-fails", true, 1, false);
        // Same path: the trash fails.
        assert_failure_restores("same-trash-fails", true, 0, true);
        // Same path: the final rename into place fails.
        assert_failure_restores("final-rename-fails", true, 2, false);
    }

    #[test]
    fn other_ticked_copies_go_to_the_trash_and_come_back() {
        let f = fixture("also");
        let original = f.file("Wallpapers/a.jpg", b"original", "orig");
        let crop = f.file("Needs Upscale/a-crop.webp", b"crop", "crop");
        let untouched = f.file("Needs Upscale/b.webp", b"b", "b");
        let new = f.file("upscale/a.png", b"upscaled", "new");
        let kept = f.bucket("Kept", &[&original, &crop]);
        let before = f.members();
        let (trash, restore) = (f.bin_trash(), f.bin_restore());
        let fs = FileSystem {
            rename: &real_rename,
            trash: &trash,
            restore: &restore,
        };
        let mut pair = f.pair(&new, &original);
        pair.also_trash = vec![crop.clone()];
        let done = replace(&f.index, &f.tags, &pair, &fs).unwrap();
        assert!(!crop.exists() && untouched.exists());
        assert_eq!(done.also_trashed, vec![crop.clone()]);
        assert_eq!(
            f.members(),
            vec![(kept, done.placed.clone(), Some("new".into()))]
        );
        undo(&f.index, &done, &fs).unwrap();
        assert!(crop.exists() && original.exists() && new.exists());
        assert_eq!(f.members(), before);
    }

    #[test]
    fn undo_refuses_when_the_new_file_name_is_taken() {
        let f = fixture("undo-taken");
        let original = f.file("Wallpapers/a.jpg", b"original", "orig");
        let new = f.file("upscale/a.png", b"upscaled", "new");
        let (trash, restore) = (f.bin_trash(), f.bin_restore());
        let fs = FileSystem {
            rename: &real_rename,
            trash: &trash,
            restore: &restore,
        };
        let done = replace(&f.index, &f.tags, &f.pair(&new, &original), &fs).unwrap();
        std::fs::write(&new, b"a newcomer").unwrap();
        assert!(undo(&f.index, &done, &fs)
            .unwrap_err()
            .contains("another file"));
        assert_eq!(std::fs::read(&done.placed).unwrap(), b"upscaled");
        assert!(!original.exists());
    }
}
