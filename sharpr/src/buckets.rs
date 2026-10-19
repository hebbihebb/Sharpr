//! Bucket membership, independent of GTK (`ROADMAP.md` Phase 4).
//!
//! Each bucket stores its own members in `bucket_members`: a file's last
//! known path and, once known, its content fingerprint. A member is the file
//! at that path or any file with the same fingerprint, so moved, renamed,
//! and identical copies all count. Members found nowhere are missing. After
//! a complete fingerprint pass, `drop_missing` removes the ones that cannot
//! come back (decision D42); until then they are counted.
//!
//! A bucket shows its own members plus every member of its sub-buckets.
//! Adding to a sub-bucket doesn't add to the parent's own list; removing
//! from a bucket removes from it and its sub-buckets. Tags are no longer
//! written. Until the one-time migration (`migrate`), membership lived in
//! tags, and those rows are kept untouched.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::library_index::{effective_tags_in, Collection, LibraryIndex, MemberRow};
use crate::tags::TagDatabase;

/// Colours for buckets without one, chosen by their root bucket's id.
use crate::palette::BUCKETS as FALLBACK_COLORS;

/// What opening a bucket shows: its visible members (own and sub-buckets'),
/// and how many members exist nowhere.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BucketView {
    pub paths: Vec<PathBuf>,
    pub missing: usize,
}

fn collection(index: &LibraryIndex, id: i64) -> Result<Collection, String> {
    index
        .collection(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Bucket no longer exists".to_string())
}

fn require_migrated(index: &LibraryIndex) -> Result<(), String> {
    if index.is_membership_migrated().map_err(|e| e.to_string())? {
        Ok(())
    } else {
        Err("buckets have not been upgraded yet; restart Sharpr and try again".into())
    }
}

/// `id` and all of its descendants.
pub fn subtree_ids(collections: &[Collection], id: i64) -> Vec<i64> {
    let mut ids = vec![id];
    let mut cursor = 0;
    while cursor < ids.len() {
        let current = ids[cursor];
        for child in collections.iter().filter(|c| c.parent_id == Some(current)) {
            if !ids.contains(&child.id) {
                ids.push(child.id);
            }
        }
        cursor += 1;
    }
    ids
}

fn ancestors(collections: &[Collection], id: i64) -> Vec<i64> {
    let mut found = Vec::new();
    let mut next = collections
        .iter()
        .find(|c| c.id == id)
        .and_then(|c| c.parent_id);
    while let Some(parent) = next.filter(|p| !found.contains(p) && *p != id) {
        found.push(parent);
        next = collections
            .iter()
            .find(|c| c.id == parent)
            .and_then(|c| c.parent_id);
    }
    found
}

/// The colour a bucket's images show: its root bucket's colour.
pub fn bucket_color(collections: &[Collection], id: i64) -> String {
    let root = ancestors(collections, id).last().copied().unwrap_or(id);
    collections
        .iter()
        .find(|c| c.id == root)
        .and_then(|c| c.color.clone())
        .unwrap_or_else(|| {
            FALLBACK_COLORS[root.rem_euclid(FALLBACK_COLORS.len() as i64) as usize].into()
        })
}

fn stamp(path: &Path) -> Option<(u64, i64)> {
    std::fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| crate::fingerprint::file_stamp(&m))
}

/// The fingerprint of the file at `path`: cached if still fresh, otherwise
/// computed now and cached. `None` when the file doesn't exist.
pub fn current_fingerprint(index: &LibraryIndex, path: &Path) -> Option<String> {
    let (size, mtime) = stamp(path)?;
    if let Ok(Some(hash)) = index.fresh_fingerprint(path, size, mtime) {
        return Some(hash);
    }
    let hash = crate::fingerprint::hash_file(path).ok()?;
    let _ = index.store_fingerprints(&[(path.to_path_buf(), size, mtime, hash.clone())]);
    Some(hash)
}

/// Whether `path` lies under one of `roots` (any path when there are none)
/// and outside the `disabled` folders.
pub(crate) fn visible(path: &Path, roots: &[PathBuf], disabled: &[PathBuf]) -> bool {
    (roots.is_empty() || roots.iter().any(|root| path.starts_with(root)))
        && !disabled.iter().any(|d| path.starts_with(d))
}

/// Resolves stored member rows to the files they mean now.
fn resolve(
    index: &LibraryIndex,
    rows: &[MemberRow],
    roots: &[PathBuf],
    disabled: &[PathBuf],
) -> Result<BucketView, String> {
    let hashes: Vec<String> = rows
        .iter()
        .filter_map(|r| r.fingerprint.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let by_hash = index
        .files_with_fingerprints(&hashes)
        .map_err(|e| e.to_string())?;
    let mut stamps: HashMap<PathBuf, Option<(u64, i64)>> = HashMap::new();
    let mut stamp_of = |path: &Path| {
        *stamps
            .entry(path.to_path_buf())
            .or_insert_with(|| stamp(path))
    };
    let mut paths = BTreeSet::new();
    let (mut found, mut lost) = (HashSet::new(), HashSet::new());
    for row in rows {
        let key = row
            .fingerprint
            .clone()
            .unwrap_or_else(|| row.last_path.to_string_lossy().into_owned());
        // Out-of-scope members are hidden, even if their files are unavailable.
        // Do not probe disabled folders or a previous, wider root.
        let eligible = visible(&row.last_path, roots, disabled);
        let mut exists = !eligible;
        if eligible && stamp_of(&row.last_path).is_some() {
            exists = true;
            paths.insert(row.last_path.clone());
        }
        for (path, size, mtime) in row
            .fingerprint
            .as_ref()
            .and_then(|hash| by_hash.get(hash))
            .into_iter()
            .flatten()
        {
            if !visible(path, roots, disabled) {
                continue;
            }
            // A cached fingerprint counts only while the file is unchanged.
            if stamp_of(path) == Some((*size, *mtime)) {
                exists = true;
                paths.insert(path.clone());
            }
        }
        if exists {
            found.insert(key);
        } else {
            lost.insert(key);
        }
    }
    Ok(BucketView {
        paths: paths.into_iter().collect(),
        missing: lost.difference(&found).count(),
    })
}

/// What opening bucket `id` shows under `root`, outside `disabled` folders.
pub fn view(
    index: &LibraryIndex,
    id: i64,
    roots: &[PathBuf],
    disabled: &[PathBuf],
) -> Result<BucketView, String> {
    let collections = index.list_collections().map_err(|e| e.to_string())?;
    let rows = index
        .bucket_members(&subtree_ids(&collections, id))
        .map_err(|e| e.to_string())?;
    resolve(index, &rows, roots, disabled)
}

/// The buckets among `collections` whose view contains the file at `path`:
/// those it is a member of, and their ancestors.
pub fn buckets_for_path<'a>(
    index: &LibraryIndex,
    collections: &'a [Collection],
    path: &Path,
) -> Result<Vec<&'a Collection>, String> {
    let fingerprint = current_fingerprint(index, path);
    let ids: Vec<i64> = collections.iter().map(|c| c.id).collect();
    let mut direct = HashSet::new();
    for row in index.bucket_members(&ids).map_err(|e| e.to_string())? {
        if row.last_path == path || (fingerprint.is_some() && row.fingerprint == fingerprint) {
            direct.insert(row.collection_id);
        }
    }
    let mut shown: HashSet<i64> = direct.clone();
    for id in &direct {
        shown.extend(ancestors(collections, *id));
    }
    Ok(collections
        .iter()
        .filter(|c| shown.contains(&c.id))
        .collect())
}

/// Badge colour per file for the filmstrip: each member file takes the
/// colour of its bucket's root. With several buckets, the one whose primary
/// tag sorts first wins, as when buckets were tags. Cached
/// fingerprints are trusted without checking files, which is fine for a
/// colour hint.
pub fn path_colors(
    index: &LibraryIndex,
    collections: &[Collection],
) -> Result<HashMap<PathBuf, String>, String> {
    let ids: Vec<i64> = collections.iter().map(|c| c.id).collect();
    let mut rows = index.bucket_members(&ids).map_err(|e| e.to_string())?;
    let tag_of: HashMap<i64, &str> = collections
        .iter()
        .map(|c| (c.id, c.primary_tag.as_str()))
        .collect();
    rows.sort_by_key(|r| (tag_of.get(&r.collection_id).copied(), r.collection_id));
    let hashes: Vec<String> = rows
        .iter()
        .filter_map(|r| r.fingerprint.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let by_hash = index
        .files_with_fingerprints(&hashes)
        .map_err(|e| e.to_string())?;
    let mut colors = HashMap::new();
    for row in rows {
        let color = bucket_color(collections, row.collection_id);
        colors
            .entry(row.last_path.clone())
            .or_insert_with(|| color.clone());
        for (path, _, _) in row
            .fingerprint
            .as_ref()
            .and_then(|hash| by_hash.get(hash))
            .into_iter()
            .flatten()
        {
            colors.entry(path.clone()).or_insert_with(|| color.clone());
        }
    }
    Ok(colors)
}

/// Adds `paths` to bucket `id` (not to its parents, which show them anyway).
/// Returns how many were new.
pub fn add_to_bucket(index: &LibraryIndex, id: i64, paths: &[PathBuf]) -> Result<usize, String> {
    require_migrated(index)?;
    collection(index, id)?;
    let members: Vec<(PathBuf, Option<String>)> = paths
        .iter()
        .map(|path| (path.clone(), current_fingerprint(index, path)))
        .collect();
    index
        .add_bucket_members(id, &members)
        .map_err(|e| e.to_string())
}

/// The parent of the buckets that collect edits (D35).
pub const EDITS_BUCKET: &str = "Edits";

/// An action that makes a copy; the original and the copy join its bucket
/// under "Edits" (D35), so they can be compared wherever the copy went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Cropped,
    Exported,
    Duplicated,
}

impl EditKind {
    pub fn bucket_name(self) -> &'static str {
        match self {
            Self::Cropped => "Cropped",
            Self::Exported => "Exported",
            Self::Duplicated => "Duplicated",
        }
    }
}

/// The bucket named `name` in this library, created under `parent` when
/// missing. A bucket of that name elsewhere in the library is reused, so
/// moving or making one by hand never splits the collection.
fn ensure_bucket(
    index: &LibraryIndex,
    library_id: &str,
    parent: Option<i64>,
    name: &str,
) -> Result<i64, String> {
    let find = || -> Result<Option<i64>, String> {
        let tag = crate::library_index::normalize_collection_tag(name);
        Ok(index
            .list_collections_for_library(Some(library_id))
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|bucket| bucket.primary_tag == tag)
            .map(|bucket| bucket.id))
    };
    if let Some(id) = find()? {
        return Ok(id);
    }
    match index.create_collection(library_id, parent, name, &[], None, None) {
        Ok(bucket) => Ok(bucket.id),
        // Another writer may have just made it.
        Err(error) => find()?.ok_or_else(|| format!("Could not create {name}: {error}")),
    }
}

/// Adds originals and their copies (`paths`) to `kind`'s bucket under
/// "Edits", creating both buckets when needed. Returns the bucket's id and
/// how many members were new; an image already there is not added twice.
pub fn add_edit_copies(
    index: &LibraryIndex,
    library_id: &str,
    kind: EditKind,
    paths: &[PathBuf],
) -> Result<(i64, usize), String> {
    require_migrated(index)?;
    let edits = ensure_bucket(index, library_id, None, EDITS_BUCKET)?;
    let id = ensure_bucket(index, library_id, Some(edits), kind.bucket_name())?;
    Ok((id, add_to_bucket(index, id, paths)?))
}

/// Removes `paths`, and identical copies of them, from bucket `id` and its
/// sub-buckets. Parents keep their own members.
pub fn remove_from_bucket(
    index: &LibraryIndex,
    id: i64,
    paths: &[PathBuf],
) -> Result<usize, String> {
    require_migrated(index)?;
    let collections = index.list_collections().map_err(|e| e.to_string())?;
    let fingerprints: Vec<String> = paths
        .iter()
        .filter_map(|path| current_fingerprint(index, path))
        .collect();
    index
        .remove_bucket_members(&subtree_ids(&collections, id), paths, &fingerprints)
        .map_err(|e| e.to_string())
}

/// Renames bucket `id`. Membership is unaffected.
pub fn rename_bucket(index: &LibraryIndex, id: i64, new_name: &str) -> Result<(), String> {
    require_migrated(index)?;
    let before = collection(index, id)?;
    index
        .update_collection(
            id,
            new_name,
            &before.extra_tags,
            before.color.as_deref(),
            before.icon_name.as_deref(),
        )
        .map_err(|e| e.to_string())
}

/// Moves bucket `id` (with its sub-buckets) under `parent`. Its members go
/// with it: old ancestors stop showing them unless they are their own
/// members too. Returns the bucket's name.
pub fn reparent_bucket(
    index: &LibraryIndex,
    id: i64,
    parent: Option<i64>,
) -> Result<String, String> {
    require_migrated(index)?;
    let name = collection(index, id)?.name;
    index
        .reparent_collection(id, parent)
        .map_err(|e| e.to_string())?;
    Ok(name)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RelinkSummary {
    /// Members whose fingerprint was recorded or updated.
    pub fingerprinted: usize,
    /// Members whose file moved, now pointing at its new path.
    pub relinked: usize,
}

/// A pass that would move at least this many members backs up the index
/// first, since that many stored paths change at once.
const BULK_RELINK: usize = 10;

/// Brings stored members up to date after a fingerprint pass, using only
/// cached fingerprints, and matching on content, never on names:
/// - a member whose file is gone moves to the one unchanged file with its
///   fingerprint. With several identical copies it stays where it is, since
///   the bucket shows every copy anyway; with none it stays, as missing;
/// - a member whose file is at its path gets that file's current fingerprint
///   (first known, or changed because the file was edited), unless the old
///   content still exists elsewhere: then another file took the name, and
///   the member follows its content to a single copy, or stays as it is.
///
/// Moving `BULK_RELINK` or more members at once first backs up the index to
/// a new folder under `backups`; if that fails nothing is changed.
#[cfg(test)]
pub fn relink(
    index: &LibraryIndex,
    roots: &[PathBuf],
    disabled: &[PathBuf],
    backups: &Path,
) -> Result<RelinkSummary, String> {
    relink_guarded(index, roots, disabled, backups, None)
}

pub fn relink_guarded(
    index: &LibraryIndex,
    roots: &[PathBuf],
    disabled: &[PathBuf],
    backups: &Path,
    guard: Option<&crate::fingerprint::PassGuard>,
) -> Result<RelinkSummary, String> {
    // Excluded members remain stored, untouched, without even a metadata probe.
    let rows: Vec<(MemberRow, Option<String>, bool)> = index
        .all_bucket_members()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|row| visible(&row.last_path, roots, disabled))
        .map(|row| {
            let (current, present) = match stamp(&row.last_path) {
                Some((size, mtime)) => (
                    index
                        .fresh_fingerprint(&row.last_path, size, mtime)
                        .map_err(|e| e.to_string())?,
                    true,
                ),
                None => (None, false),
            };
            Ok((row, current, present))
        })
        .collect::<Result<_, String>>()?;
    // The content to look for elsewhere: members whose file is gone, or whose
    // path now holds other content.
    let sought: Vec<String> = rows
        .iter()
        .filter(|(row, current, present)| {
            !present || (current.is_some() && *current != row.fingerprint)
        })
        .filter_map(|(row, _, _)| row.fingerprint.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let by_hash = index
        .files_with_fingerprints(&sought)
        .map_err(|e| e.to_string())?;
    let copies = |row: &MemberRow| -> Vec<&PathBuf> {
        row.fingerprint
            .as_ref()
            .and_then(|hash| by_hash.get(hash))
            .into_iter()
            .flatten()
            .filter(|(path, size, mtime)| {
                *path != row.last_path
                    && visible(path, roots, disabled)
                    && stamp(path) == Some((*size, *mtime))
            })
            .map(|(path, _, _)| path)
            .collect()
    };
    let mut moves: Vec<(i64, PathBuf, PathBuf)> = Vec::new();
    let mut fingerprints: Vec<(i64, PathBuf, String)> = Vec::new();
    for (row, current, present) in &rows {
        if *present && (current.is_none() || *current == row.fingerprint) {
            continue;
        }
        let elsewhere = copies(row);
        if let [to] = elsewhere.as_slice() {
            moves.push((row.collection_id, row.last_path.clone(), (*to).clone()));
        } else if *present && elsewhere.is_empty() {
            if let Some(current) = current {
                fingerprints.push((row.collection_id, row.last_path.clone(), current.clone()));
            }
        }
    }
    if moves.len() >= BULK_RELINK {
        back_up_members(index, backups, "before-relink")?;
    }
    if moves.is_empty() && fingerprints.is_empty() {
        return Ok(RelinkSummary::default());
    }
    let _gate = guard.map(|g| g.scope_gate.lock().unwrap_or_else(|e| e.into_inner()));
    if !index
        .apply_bucket_relinks(&fingerprints, &moves, &|| {
            guard.is_none_or(|g| g.is_current())
        })
        .map_err(|e| e.to_string())?
    {
        return Ok(RelinkSummary::default());
    }
    Ok(RelinkSummary {
        fingerprinted: fingerprints.len(),
        relinked: moves.len(),
    })
}

/// Only positive observations in this process may establish a missing
/// member's filesystem. Unknown legacy/missing locations remain recoverable.
#[derive(Default)]
pub struct CleanupState {
    observed: HashMap<(i64, PathBuf), ObservedLocation>,
    #[cfg(test)]
    after_backup: Option<Box<dyn FnMut() + Send>>,
}

struct ObservedLocation {
    fingerprint: String,
    root: PathBuf,
    // Bind mounts can share st_dev with the root. Every containing directory
    // must remain the same object, including an otherwise-empty mountpoint.
    ancestors: Vec<(PathBuf, u64, u64)>,
}

/// Whether `root` looks present: a folder with something in it. The mount
/// point of an unplugged drive is empty or gone.
fn root_available(root: &Path) -> bool {
    std::fs::read_dir(root).is_ok_and(|mut entries| entries.next().is_some())
}

/// Removes members that exist nowhere in the library (decision D42). Call it
/// only after `relink` that follows a complete fingerprint pass, so a moved
/// file has already been found. A member stays while it may come back: one
/// outside `roots` or in a `disabled` folder, one whose root is unavailable
/// (an unplugged drive), one with a copy still in the library, and one whose
/// file is in the trash (`trashed`, original paths), since Undo or a restore
/// brings it back. With no `roots` nothing is removed. Backs up the index to a new folder under
/// `backups` first; if that fails nothing is changed. Returns how many
/// members left each bucket.
pub fn drop_missing(
    index: &LibraryIndex,
    job: &crate::fingerprint::Job,
    pass: &crate::fingerprint::PassSummary,
    trashed: &crate::file_ops::TrashSnapshot,
    backups: &Path,
    state: &mut CleanupState,
    guard: &crate::fingerprint::PassGuard,
) -> Result<BTreeMap<i64, usize>, String> {
    let (roots, disabled) = (&job.roots, &job.disabled);
    if !guard.is_current()
        || pass.failed > 0
        || !pass.coverage.unchanged_and_complete()
        || !trashed.is_current()
    {
        return Ok(BTreeMap::new());
    }
    let members = index.all_bucket_members().map_err(|e| e.to_string())?;
    let keys: HashSet<_> = members
        .iter()
        .map(|row| (row.collection_id, row.last_path.as_path()))
        .collect();
    state
        .observed
        .retain(|(id, path), _| keys.contains(&(*id, path.as_path())));
    // Observe only files actually read/cached in this complete pass. An
    // unmounted directory cannot manufacture provenance for its missing files.
    for row in &members {
        if !guard.is_current() {
            return Ok(BTreeMap::new());
        }
        if let Some(hash) = pass.observed.get(&row.last_path) {
            if row.fingerprint.as_ref() == Some(hash) {
                if let Some(root) = roots.iter().find(|root| row.last_path.starts_with(root)) {
                    let ancestors: Option<Vec<_>> = row
                        .last_path
                        .parent()
                        .into_iter()
                        .flat_map(Path::ancestors)
                        .take_while(|parent| parent.starts_with(root))
                        .map(|parent| {
                            pass.coverage
                                .directories
                                .get(parent)
                                .map(|stamp| (parent.to_path_buf(), stamp.device, stamp.inode))
                        })
                        .collect();
                    if let Some(ancestors) = ancestors.filter(|ancestors| {
                        ancestors.last().is_some_and(|(path, _, _)| path == root)
                    }) {
                        state.observed.insert(
                            (row.collection_id, row.last_path.clone()),
                            ObservedLocation {
                                fingerprint: hash.clone(),
                                root: root.clone(),
                                ancestors,
                            },
                        );
                    }
                }
            }
        }
    }
    let available: Vec<&PathBuf> = roots.iter().filter(|r| root_available(r)).collect();
    if available.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows: Vec<MemberRow> = members
        .into_iter()
        .filter(|row| {
            guard.is_current()
                && visible(&row.last_path, roots, disabled)
                && available.iter().any(|root| row.last_path.starts_with(root))
                && stamp(&row.last_path).is_none()
                && !trashed.protects(&row.last_path)
                && state
                    .observed
                    .get(&(row.collection_id, row.last_path.clone()))
                    .is_some_and(|observed| {
                        row.fingerprint.as_ref() == Some(&observed.fingerprint)
                            && roots.contains(&observed.root)
                            && observed.ancestors.iter().all(|(path, device, inode)| {
                                pass.coverage.directories.get(path).is_some_and(|stamp| {
                                    stamp.device == *device && stamp.inode == *inode
                                })
                            })
                    })
        })
        .collect();
    let hashes: Vec<String> = rows
        .iter()
        .filter_map(|r| r.fingerprint.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let by_hash = index
        .files_with_fingerprints(&hashes)
        .map_err(|e| e.to_string())?;
    let gone: Vec<(i64, PathBuf)> = rows
        .into_iter()
        .filter(|row| {
            !row.fingerprint
                .as_ref()
                .and_then(|hash| by_hash.get(hash))
                .into_iter()
                .flatten()
                .any(|(path, size, mtime)| {
                    visible(path, roots, disabled) && stamp(path) == Some((*size, *mtime))
                })
        })
        .map(|row| (row.collection_id, row.last_path))
        .collect();
    if gone.is_empty() {
        return Ok(BTreeMap::new());
    }
    back_up_members(index, backups, "before-drop-missing")?;
    #[cfg(test)]
    if let Some(hook) = state.after_backup.as_mut() {
        hook();
    }
    // An external file manager cannot share our commit lock. Recheck its
    // effects just before arbitration; monitor events also retire the epoch.
    if !pass.coverage.unchanged_and_complete() || !trashed.is_current() {
        return Ok(BTreeMap::new());
    }
    let _gate = guard.scope_gate.lock().unwrap_or_else(|e| e.into_inner());
    let removed = index
        .remove_members_checked(&gone, &|| guard.is_current())
        .map_err(|e| e.to_string())?;
    if removed == 0 {
        return Ok(BTreeMap::new());
    }
    let mut dropped = BTreeMap::new();
    for (id, _) in &gone {
        *dropped.entry(*id).or_insert(0) += 1;
    }
    Ok(dropped)
}

/// The one toast for a `drop_missing` result.
pub fn dropped_message(collections: &[Collection], dropped: &BTreeMap<i64, usize>) -> String {
    let total: usize = dropped.values().sum();
    let names: Vec<&str> = dropped
        .keys()
        .filter_map(|id| collections.iter().find(|c| c.id == *id))
        .map(|c| c.name.as_str())
        .collect();
    let from = match names.as_slice() {
        [one] if dropped.len() == 1 => (*one).to_string(),
        [a, b] if dropped.len() == 2 => format!("{a} and {b}"),
        _ => format!("{} buckets", dropped.len()),
    };
    format!(
        "Removed {total} image{} that can’t be found from {from}",
        if total == 1 { "" } else { "s" }
    )
}

/// Copies the index to a new `<prefix>-<time>` folder under `backups` and
/// checks that the copy holds every member row.
fn back_up_members(index: &LibraryIndex, backups: &Path, prefix: &str) -> Result<PathBuf, String> {
    let members = index.all_bucket_members().map_err(|e| e.to_string())?.len();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let dir = backups.join(format!("{prefix}-{stamp}"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let copy = dir.join("library-index.sqlite");
    index.backup_to(&copy)?;
    let stored: i64 =
        rusqlite::Connection::open_with_flags(&copy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .and_then(|backup| {
                backup.query_row("SELECT COUNT(*) FROM bucket_members", [], |row| row.get(0))
            })
            .map_err(|e| format!("backup unreadable: {e}"))?;
    if stored as usize != members {
        return Err(format!(
            "backup has {stored} bucket members instead of {members}"
        ));
    }
    crate::bench_event!(
        "buckets.relink.backup",
        serde_json::json!({"members": members, "dir": dir.display().to_string()}),
    );
    Ok(dir)
}

/// Paths of every stored member, for fingerprinting them first.
pub fn member_paths(index: &LibraryIndex) -> Vec<PathBuf> {
    index
        .all_bucket_members()
        .map(|rows| rows.into_iter().map(|r| r.last_path).collect())
        .unwrap_or_default()
}

/// Moves bucket membership out of tags, once. Backs up both databases to a
/// new folder under `backups` first, and builds each bucket's own members
/// from every stored effective-tag member, including missing files, so
/// every bucket keeps exactly its members. Tag rows are left untouched.
/// Returns the backup folder when a migration ran.
pub fn migrate(
    index: &LibraryIndex,
    tags: &TagDatabase,
    backups: &Path,
) -> Result<Option<PathBuf>, String> {
    if index.is_membership_migrated().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    let collections = index.list_collections().map_err(|e| e.to_string())?;
    let legacy = index
        .unconverted_bucket_members()
        .map_err(|e| e.to_string())?;
    let backup = if collections.is_empty() {
        None
    } else {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let dir = backups.join(format!("before-bucket-membership-{stamp}"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        index.backup_to(&dir.join("library-index.sqlite"))?;
        tags.backup_to(&dir.join("tags.sqlite3"))?;
        verify_backup(&dir, collections.len(), &legacy)?;
        Some(dir)
    };
    let mut members = Vec::new();
    let mut fingerprints = HashMap::new();
    for bucket in &collections {
        let effective = effective_tags_in(&collections, bucket.id);
        let paths = if effective.is_empty() {
            Vec::new()
        } else {
            tags.stored_paths_for_all_tags_checked(&effective)?
        };
        // Carry unconverted legacy rows directly into membership. The retired
        // tag-writing conversion stays retired; both source stores stay intact.
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .chain(legacy.get(&bucket.id).into_iter().flatten().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for path in &paths {
            // Only cached fingerprints: hashing waits for the background pass.
            if let Some((size, mtime)) = stamp(path) {
                if let Ok(Some(hash)) = index.fresh_fingerprint(path, size, mtime) {
                    fingerprints.insert(path.clone(), hash);
                }
            }
        }
        members.push((bucket.id, paths));
    }
    index.migrate_bucket_membership(&members, &fingerprints)?;
    crate::bench_event!(
        "buckets.migrated",
        serde_json::json!({
            "buckets": members.len(),
            "members": members.iter().map(|(_, p)| p.len()).sum::<usize>(),
            "backup": backup.as_ref().map(|d| d.display().to_string()),
        }),
    );
    Ok(backup)
}

fn verify_backup(
    dir: &Path,
    collections: usize,
    legacy: &HashMap<i64, Vec<PathBuf>>,
) -> Result<(), String> {
    let open = |name: &str| {
        rusqlite::Connection::open_with_flags(
            dir.join(name),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| format!("backup {name} unreadable: {e}"))
    };
    let index = open("library-index.sqlite")?;
    let stored: i64 = index
        .query_row("SELECT COUNT(*) FROM collections", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if stored as usize != collections {
        return Err(format!(
            "backup has {stored} buckets instead of {collections}"
        ));
    }
    for (id, paths) in legacy {
        let stored: i64 = index
            .query_row(
                "SELECT COUNT(*) FROM collection_items WHERE collection_id = ?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|e| format!("backup legacy members unreadable: {e}"))?;
        if stored as usize != paths.len() {
            return Err(format!(
                "backup has {stored} legacy members for bucket {id} instead of {}",
                paths.len()
            ));
        }
    }
    open("tags.sqlite3")?
        .query_row("SELECT COUNT(*) FROM file_tags", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|e| format!("backup tags unreadable: {e}"))?;
    Ok(())
}

/// A manual tag that no bucket uses, offered as a new bucket now that tags
/// are retired (`ROADMAP.md` Phase 5, decisions D1 and D2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagOffer {
    pub tag: String,
    /// Images under `root` carrying the tag, including missing files.
    pub images: usize,
}

/// The images each tag would bring into a bucket: every stored row except
/// tags that buckets use (their names and extra tags), the old automatic
/// format and resolution rows (D17), and files outside `root` or in a
/// disabled folder. Missing files are kept. The offer and the conversion
/// both use this, so they always agree.
fn tag_candidates(
    index: &LibraryIndex,
    tags: &TagDatabase,
    roots: &[PathBuf],
    disabled: &[PathBuf],
) -> Result<std::collections::BTreeMap<String, Vec<PathBuf>>, String> {
    let protected: HashSet<String> = index
        .list_collections()
        .map_err(|e| e.to_string())?
        .into_iter()
        .flat_map(|c| std::iter::once(c.primary_tag).chain(c.extra_tags))
        .collect();
    let automatic: HashSet<(String, String)> =
        tags.automatic_tag_rows(&protected)?.into_iter().collect();
    let mut candidates: std::collections::BTreeMap<String, Vec<PathBuf>> = Default::default();
    for (path, tag) in tags.stored_rows()? {
        let file = PathBuf::from(&path);
        if protected.contains(&tag)
            || !visible(&file, roots, disabled)
            || automatic.contains(&(path, tag.clone()))
        {
            continue;
        }
        candidates.entry(tag).or_default().push(file);
    }
    for paths in candidates.values_mut() {
        paths.sort();
    }
    Ok(candidates)
}

/// Tags that could become buckets, most images first.
pub fn tag_offers(
    index: &LibraryIndex,
    tags: &TagDatabase,
    roots: &[PathBuf],
    disabled: &[PathBuf],
) -> Result<Vec<TagOffer>, String> {
    let mut offers: Vec<TagOffer> = tag_candidates(index, tags, roots, disabled)?
        .into_iter()
        .map(|(tag, paths)| TagOffer {
            tag,
            images: paths.len(),
        })
        .collect();
    offers.sort_by(|a, b| b.images.cmp(&a.images).then_with(|| a.tag.cmp(&b.tag)));
    Ok(offers)
}

/// Makes a bucket named `tag` from exactly the images its offer counted,
/// with the bucket and all its members written in one transaction: on any
/// failure nothing is created. The tag rows are not touched. Returns the
/// bucket and its member count.
pub fn bucket_from_tag(
    index: &LibraryIndex,
    tags: &TagDatabase,
    library_id: &str,
    roots: &[PathBuf],
    disabled: &[PathBuf],
    tag: &str,
) -> Result<(Collection, usize), String> {
    require_migrated(index)?;
    let paths = tag_candidates(index, tags, roots, disabled)?
        .remove(tag)
        .ok_or_else(|| format!("No images to add for “{tag}”"))?;
    let members: Vec<(PathBuf, Option<String>)> = paths
        .iter()
        .map(|path| (path.clone(), current_fingerprint(index, path)))
        .collect();
    let bucket = index.create_bucket_with_members(library_id, tag, &members)?;
    Ok((bucket, members.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        index: LibraryIndex,
        tags: TagDatabase,
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "sharpr-buckets-{}-{}-{name}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let index = LibraryIndex::open_in_memory().unwrap();
            let tags = TagDatabase::open_in_memory().unwrap();
            Self { index, tags, root }
        }

        /// A fixture already past the migration, as every new library is.
        fn migrated(name: &str) -> Self {
            let f = Self::new(name);
            migrate(&f.index, &f.tags, &f.root.join("backups")).unwrap();
            f
        }

        fn image(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, format!("image {name}")).unwrap();
            path
        }

        fn bucket(&self, name: &str, parent: Option<i64>) -> i64 {
            self.index
                .create_collection("test", parent, name, &[], None, None)
                .unwrap()
                .id
        }

        fn shows(&self, id: i64) -> Vec<PathBuf> {
            view(&self.index, id, std::slice::from_ref(&self.root), &[])
                .unwrap()
                .paths
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn visibility_spans_every_root_and_leaves_out_disabled_folders() {
        let roots = [PathBuf::from("/p/Pictures"), PathBuf::from("/p/Downloads")];
        let disabled = [PathBuf::from("/p/Downloads/off")];
        assert!(visible(Path::new("/p/Pictures/a.png"), &roots, &disabled));
        assert!(visible(Path::new("/p/Downloads/b.png"), &roots, &disabled));
        assert!(!visible(
            Path::new("/p/Downloads/off/c.png"),
            &roots,
            &disabled
        ));
        assert!(!visible(Path::new("/p/Elsewhere/d.png"), &roots, &disabled));
        // A forgotten root's members are hidden, not gone.
        assert!(!visible(
            Path::new("/p/Downloads/b.png"),
            &roots[..1],
            &disabled
        ));
        assert!(visible(Path::new("/p/Elsewhere/d.png"), &[], &[]));
    }

    #[test]
    fn edit_copies_join_their_action_bucket_under_edits_once() {
        let f = Fixture::migrated("edits");
        let original = f.image("photo.png");
        let crop = f.image("elsewhere/photo-cropped.webp");
        let pair = [original.clone(), crop.clone()];
        let (cropped, added) = add_edit_copies(&f.index, "test", EditKind::Cropped, &pair).unwrap();
        assert_eq!(added, 2);
        let buckets = f.index.list_collections().unwrap();
        let edits = buckets.iter().find(|b| b.name == EDITS_BUCKET).unwrap();
        let bucket = buckets.iter().find(|b| b.id == cropped).unwrap();
        assert_eq!(
            (bucket.name.as_str(), bucket.parent_id),
            ("Cropped", Some(edits.id))
        );
        let mut shown = f.shows(cropped);
        shown.sort();
        let mut expected = vec![original.clone(), crop.clone()];
        expected.sort();
        assert_eq!(shown, expected);

        // A second crop of the same original adds only the new copy, and
        // another action gets its own bucket beside it.
        let again = f.image("photo-cropped-2.webp");
        let (same, added) = add_edit_copies(
            &f.index,
            "test",
            EditKind::Cropped,
            &[original.clone(), again],
        )
        .unwrap();
        assert_eq!((same, added), (cropped, 1));
        let copy = f.image("photo copy.png");
        let (duplicated, _) =
            add_edit_copies(&f.index, "test", EditKind::Duplicated, &[original, copy]).unwrap();
        let buckets = f.index.list_collections().unwrap();
        assert_eq!(buckets.iter().filter(|b| b.name == EDITS_BUCKET).count(), 1);
        assert_eq!(
            buckets
                .iter()
                .find(|b| b.id == duplicated)
                .unwrap()
                .parent_id,
            Some(edits.id)
        );
    }

    #[test]
    fn edit_copies_reuse_a_same_named_bucket_the_user_made() {
        let f = Fixture::migrated("edits-reuse");
        let mine = f.bucket("exported", None);
        let original = f.image("a.png");
        let copy = f.image("a.webp");
        let (id, _) =
            add_edit_copies(&f.index, "test", EditKind::Exported, &[original, copy]).unwrap();
        assert_eq!(id, mine);
        assert_eq!(f.shows(mine).len(), 2);
    }

    #[test]
    fn parents_show_sub_bucket_members_and_keep_their_own_on_child_removal() {
        let f = Fixture::migrated("inherit");
        let art = f.bucket("Art", None);
        let diffusion = f.bucket("Diffusion", Some(art));
        let render = f.image("render.png");
        let sketch = f.image("sketch.png");
        add_to_bucket(&f.index, diffusion, &[render.clone(), sketch.clone()]).unwrap();
        add_to_bucket(&f.index, art, std::slice::from_ref(&render)).unwrap();
        assert_eq!(f.shows(art), vec![render.clone(), sketch.clone()]);

        // Removing from Diffusion keeps Art's own member, drops the other.
        remove_from_bucket(&f.index, diffusion, &[render.clone(), sketch.clone()]).unwrap();
        assert!(f.shows(diffusion).is_empty());
        assert_eq!(f.shows(art), vec![render.clone()]);

        // Removing from a parent also removes from its sub-buckets.
        add_to_bucket(&f.index, diffusion, std::slice::from_ref(&render)).unwrap();
        remove_from_bucket(&f.index, art, std::slice::from_ref(&render)).unwrap();
        assert!(f.shows(art).is_empty());
        assert!(f.shows(diffusion).is_empty());
    }

    #[test]
    fn relink_leaves_disabled_and_outside_root_members_and_candidates_untouched() {
        for excluded in ["disabled", "outside"] {
            let f = Fixture::migrated(excluded);
            let root = f.root.join("active");
            std::fs::create_dir_all(&root).unwrap();
            let source = f.image("active/a.png");
            let hidden = f.root.join("hidden/b.png");
            std::fs::create_dir_all(hidden.parent().unwrap()).unwrap();
            std::fs::copy(&source, &hidden).unwrap();
            let bucket = f.bucket("Keep", None);
            add_to_bucket(&f.index, bucket, std::slice::from_ref(&source)).unwrap();
            crate::fingerprint::run_pass(
                &f.index,
                std::slice::from_ref(&f.root),
                &[],
                &[],
                &|| false,
                &|_| {},
            )
            .unwrap();
            let disabled = if excluded == "disabled" {
                vec![hidden.parent().unwrap().to_path_buf()]
            } else {
                vec![]
            };
            let active = if excluded == "disabled" {
                &f.root
            } else {
                &root
            };
            std::fs::remove_file(&source).unwrap();
            let before = f.index.bucket_members(&[bucket]).unwrap();
            let summary = relink(
                &f.index,
                std::slice::from_ref(active),
                &disabled,
                &f.root.join("backups"),
            )
            .unwrap();
            assert_eq!(
                summary.relinked, 0,
                "must not probe or choose an excluded candidate"
            );
            assert_eq!(f.index.bucket_members(&[bucket]).unwrap(), before);
            assert_eq!(
                view(&f.index, bucket, std::slice::from_ref(active), &disabled)
                    .unwrap()
                    .missing,
                1
            );
            // Conversely, an excluded stored member must not be re-linked
            // into the enabled tree just because an identical copy is there.
            std::fs::write(&source, b"enabled copy").unwrap();
            std::fs::write(&hidden, b"enabled copy").unwrap();
            crate::fingerprint::run_pass(
                &f.index,
                std::slice::from_ref(&f.root),
                &[],
                &[],
                &|| false,
                &|_| {},
            )
            .unwrap();
            let hidden_bucket = f.bucket("Hidden", None);
            add_to_bucket(&f.index, hidden_bucket, std::slice::from_ref(&hidden)).unwrap();
            std::fs::remove_file(&hidden).unwrap();
            let hidden_before = f.index.bucket_members(&[hidden_bucket]).unwrap();
            relink(
                &f.index,
                std::slice::from_ref(active),
                &disabled,
                &f.root.join("backups"),
            )
            .unwrap();
            assert_eq!(
                f.index.bucket_members(&[hidden_bucket]).unwrap(),
                hidden_before
            );
        }
    }

    #[test]
    fn members_follow_moves_and_copies_and_missing_ones_are_counted() {
        let f = Fixture::migrated("identity");
        let bucket = f.bucket("Wallpapers", None);
        let a = f.image("a.jpg");
        let gone = f.image("gone.jpg");
        add_to_bucket(&f.index, bucket, &[a.clone(), gone.clone()]).unwrap();

        // Moved and renamed outside Sharpr; a fingerprint pass sees the new file.
        let moved = f.root.join("sub/renamed.jpg");
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::rename(&a, &moved).unwrap();
        let copy = f.root.join("copy.jpg");
        std::fs::copy(&moved, &copy).unwrap();
        std::fs::remove_file(&gone).unwrap();
        let never = || false;
        crate::fingerprint::run_pass(
            &f.index,
            std::slice::from_ref(&f.root),
            &[],
            &[],
            &never,
            &|_| {},
        )
        .unwrap();

        let shown = view(&f.index, bucket, std::slice::from_ref(&f.root), &[]).unwrap();
        assert_eq!(shown.paths, vec![copy.clone(), moved.clone()]);
        assert_eq!(shown.missing, 1, "the deleted file is missing, not dropped");

        // A member in a disabled folder is hidden, not missing.
        let hidden = view(
            &f.index,
            bucket,
            std::slice::from_ref(&f.root),
            &[f.root.join("sub")],
        )
        .unwrap();
        assert_eq!((hidden.paths, hidden.missing), (vec![copy], 1));
    }

    #[test]
    fn rename_reparent_and_delete_handle_members() {
        let f = Fixture::migrated("edits");
        let trips = f.bucket("Trips", None);
        let iceland = f.bucket("Iceland", Some(trips));
        let other = f.bucket("Other", None);
        let photo = f.image("fjord.jpg");
        add_to_bucket(&f.index, iceland, std::slice::from_ref(&photo)).unwrap();

        rename_bucket(&f.index, iceland, "Iceland 2026").unwrap();
        assert_eq!(f.shows(iceland), vec![photo.clone()]);

        assert_eq!(
            reparent_bucket(&f.index, iceland, Some(other)).unwrap(),
            "Iceland 2026"
        );
        assert!(
            f.shows(trips).is_empty(),
            "the old parent no longer shows it"
        );
        assert_eq!(f.shows(other), vec![photo.clone()]);

        f.index.delete_collection(other).unwrap();
        assert!(f
            .index
            .bucket_members(&[other, iceland])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn info_chip_buckets_match_what_opening_each_bucket_shows() {
        let f = Fixture::migrated("chip");
        let art = f.bucket("Art", None);
        let diffusion = f.bucket("Diffusion", Some(art));
        let blue = f.bucket("Blue", None);
        let image = f.image("render.png");
        let check = |expected: &[i64]| {
            let collections = f.index.list_collections().unwrap();
            let mut chip: Vec<i64> = buckets_for_path(&f.index, &collections, &image)
                .unwrap()
                .iter()
                .map(|c| c.id)
                .collect();
            let mut opened: Vec<i64> = collections
                .iter()
                .map(|c| c.id)
                .filter(|id| f.shows(*id).contains(&image))
                .collect();
            chip.sort();
            opened.sort();
            assert_eq!(chip, opened, "chip and bucket view disagree");
            assert_eq!(chip, expected);
        };
        add_to_bucket(&f.index, diffusion, std::slice::from_ref(&image)).unwrap();
        check(&[art, diffusion]);
        remove_from_bucket(&f.index, diffusion, std::slice::from_ref(&image)).unwrap();
        check(&[]);
        add_to_bucket(&f.index, blue, std::slice::from_ref(&image)).unwrap();
        check(&[blue]);
    }

    #[test]
    fn relink_fills_fingerprints_and_follows_moved_and_edited_files() {
        let f = Fixture::new("relink");
        let bucket = f.bucket("Keep", None);
        let photo = f.image("photo.jpg");
        let tag = f.index.collection(bucket).unwrap().unwrap().primary_tag;
        f.tags
            .change_tags_checked(std::slice::from_ref(&photo), &[tag], false)
            .unwrap();
        // Migrated before any fingerprint existed.
        migrate(&f.index, &f.tags, &f.root.join("backups")).unwrap();
        assert_eq!(
            f.index.bucket_members(&[bucket]).unwrap()[0].fingerprint,
            None
        );
        let never = || false;
        let pass = || {
            crate::fingerprint::run_pass(
                &f.index,
                std::slice::from_ref(&f.root),
                &[],
                &[],
                &never,
                &|_| {},
            )
            .unwrap();
            relink(&f.index, &[], &[], &f.root.join("backups")).unwrap()
        };
        assert_eq!(
            pass(),
            RelinkSummary {
                fingerprinted: 1,
                relinked: 0
            }
        );
        let hash = crate::fingerprint::hash_file(&photo).unwrap();
        assert_eq!(
            f.index.bucket_members(&[bucket]).unwrap()[0].fingerprint,
            Some(hash)
        );

        // Moved and renamed outside Sharpr: the member follows the file.
        let moved = f.root.join("2026/beach.jpg");
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::rename(&photo, &moved).unwrap();
        assert_eq!(
            pass(),
            RelinkSummary {
                fingerprinted: 0,
                relinked: 1
            }
        );
        assert_eq!(
            f.index.bucket_members(&[bucket]).unwrap()[0].last_path,
            moved
        );

        // Edited in place: the member keeps its path and takes the new fingerprint.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&moved, b"edited").unwrap();
        assert_eq!(
            pass(),
            RelinkSummary {
                fingerprinted: 1,
                relinked: 0
            }
        );
        assert_eq!(f.shows(bucket), vec![moved.clone()]);

        // Deleted: it stays, as missing.
        std::fs::remove_file(&moved).unwrap();
        assert_eq!(pass(), RelinkSummary::default());
        let gone = view(&f.index, bucket, std::slice::from_ref(&f.root), &[]).unwrap();
        assert_eq!((gone.paths.len(), gone.missing), (0, 1));
        assert_eq!(f.index.bucket_members(&[bucket]).unwrap().len(), 1);
    }

    /// A fingerprint pass over the fixture's root, then a relink.
    fn pass(f: &Fixture) -> RelinkSummary {
        crate::fingerprint::run_pass(
            &f.index,
            std::slice::from_ref(&f.root),
            &[],
            &[],
            &|| false,
            &|_| {},
        )
        .unwrap();
        relink(
            &f.index,
            std::slice::from_ref(&f.root),
            &[],
            &f.root.join("backups"),
        )
        .unwrap()
    }

    fn move_file(from: &Path, to: &Path) {
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::rename(from, to).unwrap();
    }

    #[test]
    fn a_member_moved_outside_sharpr_is_found_by_content_never_by_name() {
        let f = Fixture::migrated("moved-outside");
        let bucket = f.bucket("Keep", None);
        let photo = f.image("a/photo.jpg");
        let hash = crate::fingerprint::hash_file(&photo).unwrap();
        add_to_bucket(&f.index, bucket, std::slice::from_ref(&photo)).unwrap();
        // Another file with the member's name, other content, elsewhere.
        let decoy = f.root.join("b/photo.jpg");
        std::fs::create_dir_all(decoy.parent().unwrap()).unwrap();
        std::fs::write(&decoy, b"another picture").unwrap();

        // Moved and renamed in a file manager: the bucket finds it.
        let moved = f.root.join("c/beach.jpg");
        move_file(&photo, &moved);
        assert_eq!(
            pass(&f),
            RelinkSummary {
                fingerprinted: 0,
                relinked: 1
            }
        );
        let rows = f.index.bucket_members(&[bucket]).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].last_path, moved);
        assert_eq!(rows[0].fingerprint.as_ref(), Some(&hash));
        assert_eq!(f.shows(bucket), vec![moved.clone()]);

        // Moved again, and a different picture takes its old name: the
        // member follows its content, not the name.
        let again = f.root.join("d/beach.jpg");
        move_file(&moved, &again);
        std::fs::write(&moved, b"a newer download").unwrap();
        assert_eq!(
            pass(&f),
            RelinkSummary {
                fingerprinted: 0,
                relinked: 1
            }
        );
        let rows = f.index.bucket_members(&[bucket]).unwrap();
        assert_eq!(rows[0].last_path, again);
        assert_eq!(rows[0].fingerprint.as_ref(), Some(&hash));
        assert_eq!(f.shows(bucket), vec![again.clone()]);
        assert!(!f.shows(bucket).contains(&decoy));

        // Gone from the library: only same-named files are left, and
        // neither is taken for it. It stays, as missing.
        std::fs::remove_file(&again).unwrap();
        assert_eq!(pass(&f), RelinkSummary::default());
        let gone = view(&f.index, bucket, std::slice::from_ref(&f.root), &[]).unwrap();
        assert_eq!((gone.paths, gone.missing), (vec![], 1));
        assert_eq!(
            f.index.bucket_members(&[bucket]).unwrap()[0].last_path,
            again
        );
    }

    #[test]
    fn identical_copies_leave_the_stored_path_and_all_show() {
        let f = Fixture::migrated("copies");
        let bucket = f.bucket("Keep", None);
        let photo = f.image("photo.jpg");
        add_to_bucket(&f.index, bucket, std::slice::from_ref(&photo)).unwrap();
        let (one, two) = (f.root.join("x/one.jpg"), f.root.join("y/two.jpg"));
        for copy in [&one, &two] {
            std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
            std::fs::copy(&photo, copy).unwrap();
        }
        std::fs::remove_file(&photo).unwrap();
        // No unique match: the stored path is not rewritten to either copy.
        assert_eq!(pass(&f), RelinkSummary::default());
        assert_eq!(
            f.index.bucket_members(&[bucket]).unwrap()[0].last_path,
            photo
        );
        let shown = view(&f.index, bucket, std::slice::from_ref(&f.root), &[]).unwrap();
        assert_eq!(
            (shown.paths, shown.missing),
            (vec![one.clone(), two.clone()], 0)
        );

        // One copy goes: the member follows the other.
        std::fs::remove_file(&one).unwrap();
        assert_eq!(pass(&f).relinked, 1);
        assert_eq!(f.index.bucket_members(&[bucket]).unwrap()[0].last_path, two);
    }

    #[test]
    fn moving_many_members_at_once_backs_up_the_index_first() {
        let f = Fixture::migrated("bulk");
        let bucket = f.bucket("Keep", None);
        let photos: Vec<PathBuf> = (0..13).map(|i| f.image(&format!("old/{i}.jpg"))).collect();
        add_to_bucket(&f.index, bucket, &photos).unwrap();
        let backups = || -> Vec<PathBuf> {
            std::fs::read_dir(f.root.join("backups"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("before-relink-")
                })
                .collect()
        };
        let move_all = |few: &[PathBuf]| {
            for path in few {
                move_file(path, &f.root.join("new").join(path.file_name().unwrap()));
            }
        };
        move_all(&photos[..3]);
        assert_eq!(pass(&f).relinked, 3);
        assert!(backups().is_empty());

        move_all(&photos[3..]);
        assert_eq!(pass(&f).relinked, 10);
        let made = backups();
        assert_eq!(made.len(), 1);
        let copy = rusqlite::Connection::open(made[0].join("library-index.sqlite")).unwrap();
        // Taken before the moves: 13 members, ten still at their old paths.
        let (all, old): (i64, i64) = copy
            .query_row(
                "SELECT COUNT(*), SUM(last_path LIKE '%/old/%') FROM bucket_members",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((all, old), (13, 10));
        assert_eq!(f.shows(bucket).len(), 13);
    }

    #[test]
    fn migration_keeps_every_member_and_leaves_tags_untouched() {
        let f = Fixture::new("migrate");
        let art = f.bucket("Art", None);
        let diffusion = f.bucket("Diffusion", Some(art));
        let render = f.image("render.png");
        let photo = f.image("photo.jpg");
        let missing = f.root.join("missing.png");
        // Tag-backed membership as older builds stored it.
        let art_tag = f.index.collection(art).unwrap().unwrap().primary_tag;
        let diffusion_tag = f.index.collection(diffusion).unwrap().unwrap().primary_tag;
        f.tags
            .change_tags_checked(
                &[render.clone(), missing.clone()],
                &[art_tag.clone(), diffusion_tag.clone()],
                false,
            )
            .unwrap();
        f.tags
            .change_tags_checked(
                std::slice::from_ref(&photo),
                std::slice::from_ref(&art_tag),
                false,
            )
            .unwrap();
        f.tags.add_tag(&photo, "landscape");
        let tag_rows = |db: &TagDatabase| {
            let mut rows: Vec<(PathBuf, Vec<String>)> = [&render, &photo, &missing]
                .iter()
                .map(|p| ((*p).clone(), db.tags_for_path(p)))
                .collect();
            rows.sort();
            rows
        };
        let before = tag_rows(&f.tags);
        assert!(add_to_bucket(&f.index, art, std::slice::from_ref(&photo)).is_err());

        let backups = f.root.join("backups");
        let backup = migrate(&f.index, &f.tags, &backups).unwrap().unwrap();
        assert!(backup.join("library-index.sqlite").is_file());
        assert!(backup.join("tags.sqlite3").is_file());
        assert_eq!(tag_rows(&f.tags), before, "tag rows are untouched");

        // Same members as the tag rules gave, the missing file included.
        let art_view = view(&f.index, art, std::slice::from_ref(&f.root), &[]).unwrap();
        assert_eq!(
            (art_view.paths, art_view.missing),
            (vec![photo.clone(), render.clone()], 1)
        );
        let diffusion_view = view(&f.index, diffusion, std::slice::from_ref(&f.root), &[]).unwrap();
        assert_eq!(
            (diffusion_view.paths, diffusion_view.missing),
            (vec![render.clone()], 1)
        );

        // It runs once.
        assert_eq!(migrate(&f.index, &f.tags, &backups).unwrap(), None);
        assert_eq!(f.index.bucket_members(&[art]).unwrap().len(), 3);
    }

    #[test]
    fn unused_manual_tags_become_buckets_and_tag_rows_stay() {
        let f = Fixture::migrated("tag-offers");
        let a = f.image("a.png");
        let b = f.image("b.jpg");
        let missing = f.root.join("gone.jpg");
        let outside = PathBuf::from("/elsewhere/c.jpg");
        let art = f.bucket("Art", None);
        add_to_bucket(&f.index, art, std::slice::from_ref(&a)).unwrap();
        for (path, tag) in [
            (&a, "nature"),
            (&b, "nature"),
            (&missing, "nature"),
            (&outside, "nature"),
            (&a, "art"), // a bucket's own tag
            (&a, "png"), // automatic: the file's own format
            (&b, "png"), // hand-made: a JPEG tagged png
            (&b, "4k"),  // automatic resolution tag
        ] {
            f.tags.add_tag(path, tag);
        }
        let rows_before = {
            let mut rows = f.tags.stored_rows().unwrap();
            rows.sort();
            rows
        };

        let offers = tag_offers(&f.index, &f.tags, std::slice::from_ref(&f.root), &[]).unwrap();
        assert_eq!(
            offers,
            vec![
                TagOffer {
                    tag: "nature".into(),
                    images: 3
                },
                TagOffer {
                    tag: "png".into(),
                    images: 1
                },
            ],
            "bucket tags, automatic rows, and images outside the library are left out"
        );

        let (bucket, members) = bucket_from_tag(
            &f.index,
            &f.tags,
            "test",
            std::slice::from_ref(&f.root),
            &[],
            "nature",
        )
        .unwrap();
        assert_eq!((bucket.name.as_str(), members), ("nature", 3));
        let view = view(&f.index, bucket.id, std::slice::from_ref(&f.root), &[]).unwrap();
        assert_eq!(view.paths, vec![a.clone(), b.clone()]);
        assert_eq!(view.missing, 1, "the missing file is kept as a member");
        let mut rows_after = f.tags.stored_rows().unwrap();
        rows_after.sort();
        assert_eq!(rows_after, rows_before, "tag rows are untouched");
        assert!(
            tag_offers(&f.index, &f.tags, std::slice::from_ref(&f.root), &[])
                .unwrap()
                .iter()
                .all(|o| o.tag != "nature"),
            "a converted tag is no longer offered"
        );
        assert!(
            bucket_from_tag(
                &f.index,
                &f.tags,
                "test",
                std::slice::from_ref(&f.root),
                &[],
                "Nature"
            )
            .is_err(),
            "no second bucket with the same name"
        );

        // The conversion takes exactly what the offer counted: the JPEG's
        // hand-made png tag, not the PNG's automatic one.
        let (png, members) = bucket_from_tag(
            &f.index,
            &f.tags,
            "test",
            std::slice::from_ref(&f.root),
            &[],
            "png",
        )
        .unwrap();
        assert_eq!(members, 1);
        assert_eq!(
            super::view(&f.index, png.id, std::slice::from_ref(&f.root), &[])
                .unwrap()
                .paths,
            vec![b.clone()]
        );

        // Disabled folders are neither counted nor written.
        let hidden = f.image("off/d.jpg");
        f.tags.add_tag(&hidden, "comics");
        f.tags.add_tag(&a, "comics");
        let disabled = vec![f.root.join("off")];
        let offers =
            tag_offers(&f.index, &f.tags, std::slice::from_ref(&f.root), &disabled).unwrap();
        assert_eq!(offers.iter().find(|o| o.tag == "comics").unwrap().images, 1);
        let (comics, members) = bucket_from_tag(
            &f.index,
            &f.tags,
            "test",
            std::slice::from_ref(&f.root),
            &disabled,
            "comics",
        )
        .unwrap();
        assert_eq!(members, 1);
        assert!(f
            .index
            .bucket_members(&[comics.id])
            .unwrap()
            .iter()
            .all(|m| !m.last_path.starts_with(f.root.join("off"))));
    }

    fn cleanup_pass(
        f: &Fixture,
        job: &crate::fingerprint::Job,
        state: &mut CleanupState,
        trash: &crate::file_ops::TrashSnapshot,
        guard: &crate::fingerprint::PassGuard,
    ) -> BTreeMap<i64, usize> {
        let pass = crate::fingerprint::run_pass(
            &f.index,
            &job.roots,
            &job.disabled,
            &[],
            &|| false,
            &|_| {},
        )
        .unwrap();
        relink(&f.index, &job.roots, &job.disabled, &f.root.join("backups")).unwrap();
        super::drop_missing(
            &f.index,
            job,
            &pass,
            trash,
            &f.root.join("backups"),
            state,
            guard,
        )
        .unwrap()
    }

    fn cleanup_job(f: &Fixture) -> crate::fingerprint::Job {
        let root = f.root.join("library");
        std::fs::create_dir_all(&root).unwrap();
        crate::fingerprint::Job {
            roots: vec![root],
            disabled: Vec::new(),
        }
    }

    #[test]
    fn cleanup_retains_a_member_without_observed_filesystem() {
        let f = Fixture::migrated("unknown-filesystem");
        let job = cleanup_job(&f);
        let bucket = f.bucket("Keep", None);
        let missing = f.image("library/mount/photo.jpg");
        add_to_bucket(&f.index, bucket, std::slice::from_ref(&missing)).unwrap();
        std::fs::remove_file(&missing).unwrap();
        f.image("library/present.jpg");
        let dropped = cleanup_pass(
            &f,
            &job,
            &mut CleanupState::default(),
            &Default::default(),
            &crate::fingerprint::PassGuard::for_test(),
        );
        assert!(dropped.is_empty());
        assert_eq!(f.index.bucket_members(&[bucket]).unwrap().len(), 1);
    }

    #[test]
    fn a_member_gone_from_the_library_leaves_its_buckets_after_a_pass() {
        let f = Fixture::migrated("drop-missing");
        let job = cleanup_job(&f);
        let mut state = CleanupState::default();
        let guard = crate::fingerprint::PassGuard::for_test();
        let keep = f.bucket("Keep", None);
        let other = f.bucket("Other", None);
        let gone = f.image("library/a/gone.jpg");
        let stays = f.image("library/a/stays.jpg");
        add_to_bucket(&f.index, keep, &[gone.clone(), stays.clone()]).unwrap();
        add_to_bucket(&f.index, other, std::slice::from_ref(&gone)).unwrap();
        assert!(cleanup_pass(&f, &job, &mut state, &Default::default(), &guard).is_empty());
        std::fs::remove_file(&gone).unwrap();
        let dropped = cleanup_pass(&f, &job, &mut state, &Default::default(), &guard);
        assert_eq!(dropped, BTreeMap::from([(keep, 1), (other, 1)]));
        assert_eq!(
            f.index
                .bucket_members(&[keep, other])
                .unwrap()
                .iter()
                .map(|r| r.last_path.clone())
                .collect::<Vec<_>>(),
            vec![stays]
        );
        let backups = f.root.join("backups");
        let before = std::fs::read_dir(&backups).unwrap().count();
        assert!(before > 0);
        assert!(cleanup_pass(&f, &job, &mut state, &Default::default(), &guard).is_empty());
        assert_eq!(std::fs::read_dir(backups).unwrap().count(), before);
    }

    #[test]
    fn missing_members_stay_while_they_may_come_back() {
        let f = Fixture::migrated("keep-missing");
        let mut job = cleanup_job(&f);
        let mut state = CleanupState::default();
        let guard = crate::fingerprint::PassGuard::for_test();
        let bucket = f.bucket("Keep", None);
        let missing = f.image("library/set/photo.jpg");
        f.image("library/present.jpg");
        add_to_bucket(&f.index, bucket, std::slice::from_ref(&missing)).unwrap();
        cleanup_pass(&f, &job, &mut state, &Default::default(), &guard);
        std::fs::remove_file(&missing).unwrap();
        job.disabled.push(f.root.join("library/set"));
        assert!(cleanup_pass(&f, &job, &mut state, &Default::default(), &guard).is_empty());
        job.disabled.clear();
        let trash = crate::file_ops::TrashSnapshot::from_paths([f.root.join("library/set")]);
        assert!(
            cleanup_pass(&f, &job, &mut state, &trash, &guard).is_empty(),
            "trashed parent protects descendants"
        );
        assert_eq!(f.index.bucket_members(&[bucket]).unwrap().len(), 1);
    }

    #[test]
    fn a_hidden_move_or_changed_directory_cannot_prove_absence() {
        let f = Fixture::migrated("hidden-move");
        let job = cleanup_job(&f);
        let mut state = CleanupState::default();
        let guard = crate::fingerprint::PassGuard::for_test();
        let bucket = f.bucket("Keep", None);
        let photo = f.image("library/photo.jpg");
        add_to_bucket(&f.index, bucket, std::slice::from_ref(&photo)).unwrap();
        cleanup_pass(&f, &job, &mut state, &Default::default(), &guard);
        let pass = crate::fingerprint::run_pass(&f.index, &job.roots, &[], &[], &|| false, &|_| {})
            .unwrap();
        move_file(&photo, &f.root.join("library/.hidden/photo.jpg"));
        assert!(
            super::drop_missing(
                &f.index,
                &job,
                &pass,
                &Default::default(),
                &f.root.join("backups"),
                &mut state,
                &guard
            )
            .unwrap()
            .is_empty(),
            "directory changed after collection"
        );
        assert!(
            cleanup_pass(&f, &job, &mut state, &Default::default(), &guard).is_empty(),
            "hidden destination was not searched"
        );
        assert_eq!(f.index.bucket_members(&[bucket]).unwrap().len(), 1);
    }

    #[test]
    fn observed_members_survive_unavailable_or_excluded_roots() {
        for scenario in ["empty", "missing", "removed", "disabled", "none"] {
            let f = Fixture::migrated(scenario);
            let mut job = cleanup_job(&f);
            let mut state = CleanupState::default();
            let guard = crate::fingerprint::PassGuard::for_test();
            let bucket = f.bucket("Keep", None);
            let photo = f.image("library/sub/photo.jpg");
            let present = f.image("library/present.jpg");
            add_to_bucket(&f.index, bucket, std::slice::from_ref(&photo)).unwrap();
            cleanup_pass(&f, &job, &mut state, &Default::default(), &guard);
            assert_eq!(
                state.observed.len(),
                1,
                "positive provenance before {scenario}"
            );
            std::fs::remove_file(&photo).unwrap();
            match scenario {
                "empty" => {
                    std::fs::remove_file(present).unwrap();
                    std::fs::remove_dir(photo.parent().unwrap()).unwrap();
                }
                "missing" => {
                    std::fs::remove_dir_all(&job.roots[0]).unwrap();
                }
                "removed" => {
                    f.image("another/present.jpg");
                    job.roots = vec![f.root.join("another")];
                }
                "disabled" => {
                    job.disabled = vec![photo.parent().unwrap().to_path_buf()];
                }
                "none" => {}
                _ => unreachable!(),
            }
            if scenario == "none" {
                // Reuse the isolated scan, then remove its scope. Never let an
                // empty-roots scan fall back to the real user's Home folder.
                let pass = crate::fingerprint::run_pass(
                    &f.index,
                    &job.roots,
                    &[],
                    &[],
                    &|| false,
                    &|_| {},
                )
                .unwrap();
                job.roots.clear();
                assert!(super::drop_missing(
                    &f.index,
                    &job,
                    &pass,
                    &Default::default(),
                    &f.root.join("backups"),
                    &mut state,
                    &guard
                )
                .unwrap()
                .is_empty());
            } else {
                assert!(
                    cleanup_pass(&f, &job, &mut state, &Default::default(), &guard).is_empty(),
                    "{scenario}"
                );
            }
            assert_eq!(
                f.index.bucket_members(&[bucket]).unwrap().len(),
                1,
                "{scenario}"
            );
        }
    }

    #[test]
    fn an_observed_missing_member_with_multiple_cached_copies_stays() {
        let f = Fixture::migrated("copies-survive");
        let job = cleanup_job(&f);
        let mut state = CleanupState::default();
        let guard = crate::fingerprint::PassGuard::for_test();
        let bucket = f.bucket("Keep", None);
        let photo = f.image("library/photo.jpg");
        add_to_bucket(&f.index, bucket, std::slice::from_ref(&photo)).unwrap();
        cleanup_pass(&f, &job, &mut state, &Default::default(), &guard);
        for name in ["copy-1.jpg", "copy-2.jpg"] {
            std::fs::copy(&photo, job.roots[0].join(name)).unwrap();
        }
        std::fs::remove_file(&photo).unwrap();
        assert!(cleanup_pass(&f, &job, &mut state, &Default::default(), &guard).is_empty());
        let rows = f.index.bucket_members(&[bucket]).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].last_path, photo,
            "multiple copies keep the stored location"
        );
        assert_eq!(
            view(&f.index, bucket, &job.roots, &[]).unwrap().paths.len(),
            2
        );
    }

    #[test]
    fn invalidation_during_backup_or_staged_delete_keeps_members() {
        for change_scope in [false, true] {
            let f = Fixture::migrated("backup-invalidation");
            let job = cleanup_job(&f);
            let mut state = CleanupState::default();
            let guard = crate::fingerprint::PassGuard::for_test();
            let bucket = f.bucket("Keep", None);
            let photo = f.image("library/photo.jpg");
            f.image("library/present.jpg");
            add_to_bucket(&f.index, bucket, std::slice::from_ref(&photo)).unwrap();
            cleanup_pass(&f, &job, &mut state, &Default::default(), &guard);
            std::fs::remove_file(&photo).unwrap();
            let stale = guard.clone();
            let appeared = f.root.join("library/appeared.jpg");
            state.after_backup = Some(Box::new(move || {
                if change_scope {
                    stale.invalidate_for_test();
                } else {
                    std::fs::write(&appeared, b"a possible moved copy").unwrap();
                }
            }));
            assert!(cleanup_pass(&f, &job, &mut state, &Default::default(), &guard).is_empty());
            assert_eq!(f.index.bucket_members(&[bucket]).unwrap().len(), 1);
        }

        let f = Fixture::migrated("transaction-invalidation");
        let bucket = f.bucket("Keep", None);
        let photo = f.image("photo.jpg");
        add_to_bucket(&f.index, bucket, std::slice::from_ref(&photo)).unwrap();
        let checked = std::cell::Cell::new(false);
        let removed = f
            .index
            .remove_members_checked(&[(bucket, photo)], &|| !checked.replace(true))
            .unwrap();
        assert_eq!(removed, 0, "invalidating staged work rolls back its DELETE");
        assert_eq!(f.index.bucket_members(&[bucket]).unwrap().len(), 1);
    }

    #[test]
    fn a_replaced_same_filesystem_member_parent_is_not_permanent_loss() {
        let f = Fixture::migrated("same-device-mount");
        let job = cleanup_job(&f);
        let mut state = CleanupState::default();
        let guard = crate::fingerprint::PassGuard::for_test();
        let bucket = f.bucket("Keep", None);
        let photo = f.image("library/mount/photo.jpg");
        f.image("library/present.jpg");
        add_to_bucket(&f.index, bucket, std::slice::from_ref(&photo)).unwrap();
        cleanup_pass(&f, &job, &mut state, &Default::default(), &guard);
        // A same-filesystem bind mount disappears without changing the root's
        // device or inode. Keep its recoverable bytes outside the scanned root.
        std::fs::rename(f.root.join("library/mount"), f.root.join("offline")).unwrap();
        std::fs::create_dir(f.root.join("library/mount")).unwrap();
        assert!(cleanup_pass(&f, &job, &mut state, &Default::default(), &guard).is_empty());
        assert_eq!(f.index.bucket_members(&[bucket]).unwrap().len(), 1);
        assert!(f.root.join("offline/photo.jpg").exists());
    }

    #[test]
    fn changed_root_identity_or_stale_scope_cannot_drop_members() {
        let f = Fixture::migrated("stale-scope");
        let job = cleanup_job(&f);
        let mut state = CleanupState::default();
        let guard = crate::fingerprint::PassGuard::for_test();
        let bucket = f.bucket("Keep", None);
        let photo = f.image("library/photo.jpg");
        add_to_bucket(&f.index, bucket, std::slice::from_ref(&photo)).unwrap();
        cleanup_pass(&f, &job, &mut state, &Default::default(), &guard);
        std::fs::rename(&job.roots[0], f.root.join("unmounted")).unwrap();
        std::fs::create_dir(&job.roots[0]).unwrap();
        f.image("library/present.jpg");
        assert!(
            cleanup_pass(&f, &job, &mut state, &Default::default(), &guard).is_empty(),
            "replacement mountpoint is a different root identity"
        );
        std::fs::remove_dir_all(&job.roots[0]).unwrap();
        std::fs::rename(f.root.join("unmounted"), &job.roots[0]).unwrap();
        std::fs::remove_file(&photo).unwrap();
        f.image("library/present.jpg");
        guard.invalidate_for_test();
        assert!(cleanup_pass(&f, &job, &mut state, &Default::default(), &guard).is_empty());
        assert_eq!(f.index.bucket_members(&[bucket]).unwrap().len(), 1);
    }

    #[test]
    fn the_drop_message_names_up_to_two_buckets() {
        let f = Fixture::migrated("drop-message");
        let (a, b, c) = (
            f.bucket("Art", None),
            f.bucket("Beach", None),
            f.bucket("City", None),
        );
        let collections = f.index.list_collections().unwrap();
        assert_eq!(
            dropped_message(&collections, &BTreeMap::from([(a, 1)])),
            "Removed 1 image that can’t be found from Art"
        );
        assert_eq!(
            dropped_message(&collections, &BTreeMap::from([(a, 2), (b, 1)])),
            "Removed 3 images that can’t be found from Art and Beach"
        );
        assert_eq!(
            dropped_message(&collections, &BTreeMap::from([(a, 1), (b, 1), (c, 1)])),
            "Removed 3 images that can’t be found from 3 buckets"
        );
    }
}
