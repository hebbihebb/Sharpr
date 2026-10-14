//! Bucket membership, independent of GTK (`ROADMAP.md` Phase 4).
//!
//! Each bucket stores its own members in `bucket_members`: a file's last
//! known path and, once known, its content fingerprint. A member is the file
//! at that path or any file with the same fingerprint, so moved, renamed,
//! and identical copies all count. Members found nowhere are missing; they
//! are counted and never dropped.
//!
//! A bucket shows its own members plus every member of its sub-buckets.
//! Adding to a sub-bucket doesn't add to the parent's own list; removing
//! from a bucket removes from it and its sub-buckets. Tags are no longer
//! written. Until the one-time migration (`migrate`), membership lived in
//! tags, and those rows are kept untouched.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::library_index::{effective_tags_in, Collection, LibraryIndex, MemberRow};
use crate::tags::TagDatabase;

/// Colours for buckets without one, chosen by their root bucket's id.
const FALLBACK_COLORS: &[&str] = &[
    "#57e389", "#62a0ea", "#ff7800", "#f5c211", "#dc8add", "#5bc8af", "#e01b24", "#9141ac",
];

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

fn visible(path: &Path, root: Option<&Path>, disabled: &[PathBuf]) -> bool {
    root.is_none_or(|root| path.starts_with(root)) && !disabled.iter().any(|d| path.starts_with(d))
}

/// Resolves stored member rows to the files they mean now.
fn resolve(
    index: &LibraryIndex,
    rows: &[MemberRow],
    root: Option<&Path>,
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
        let mut exists = false;
        if stamp_of(&row.last_path).is_some() {
            exists = true;
            if visible(&row.last_path, root, disabled) {
                paths.insert(row.last_path.clone());
            }
        }
        for (path, size, mtime) in row
            .fingerprint
            .as_ref()
            .and_then(|hash| by_hash.get(hash))
            .into_iter()
            .flatten()
        {
            // A cached fingerprint counts only while the file is unchanged.
            if stamp_of(path) == Some((*size, *mtime)) {
                exists = true;
                if visible(path, root, disabled) {
                    paths.insert(path.clone());
                }
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
    root: Option<&Path>,
    disabled: &[PathBuf],
) -> Result<BucketView, String> {
    let collections = index.list_collections().map_err(|e| e.to_string())?;
    let rows = index
        .bucket_members(&subtree_ids(&collections, id))
        .map_err(|e| e.to_string())?;
    resolve(index, &rows, root, disabled)
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

/// Brings stored members up to date after a fingerprint pass, using only
/// cached fingerprints:
/// - a member whose file is at its path gets that file's current fingerprint
///   (first known, or changed because the file was edited);
/// - a member whose file is gone moves to an unchanged file with its
///   fingerprint, if there is one. Others stay, as missing.
pub fn relink(index: &LibraryIndex) -> Result<RelinkSummary, String> {
    let rows = index.all_bucket_members().map_err(|e| e.to_string())?;
    let mut summary = RelinkSummary::default();
    let lost: Vec<String> = rows
        .iter()
        .filter(|r| stamp(&r.last_path).is_none())
        .filter_map(|r| r.fingerprint.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let by_hash = index
        .files_with_fingerprints(&lost)
        .map_err(|e| e.to_string())?;
    for row in rows {
        match stamp(&row.last_path) {
            Some((size, mtime)) => {
                let current = index
                    .fresh_fingerprint(&row.last_path, size, mtime)
                    .map_err(|e| e.to_string())?;
                if let Some(current) = current.filter(|c| row.fingerprint.as_ref() != Some(c)) {
                    index
                        .set_member_fingerprint(row.collection_id, &row.last_path, &current)
                        .map_err(|e| e.to_string())?;
                    summary.fingerprinted += 1;
                }
            }
            None => {
                let mut candidates: Vec<&PathBuf> = row
                    .fingerprint
                    .as_ref()
                    .and_then(|hash| by_hash.get(hash))
                    .into_iter()
                    .flatten()
                    .filter(|(path, size, mtime)| stamp(path) == Some((*size, *mtime)))
                    .map(|(path, _, _)| path)
                    .collect();
                candidates.sort();
                if let Some(to) = candidates.first() {
                    index
                        .move_member(row.collection_id, &row.last_path, to)
                        .map_err(|e| e.to_string())?;
                    summary.relinked += 1;
                }
            }
        }
    }
    Ok(summary)
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
        verify_backup(&dir, collections.len())?;
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

fn verify_backup(dir: &Path, collections: usize) -> Result<(), String> {
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
    root: &Path,
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
            || !visible(&file, Some(root), disabled)
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
    root: &Path,
    disabled: &[PathBuf],
) -> Result<Vec<TagOffer>, String> {
    let mut offers: Vec<TagOffer> = tag_candidates(index, tags, root, disabled)?
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
    root: &Path,
    disabled: &[PathBuf],
    tag: &str,
) -> Result<(Collection, usize), String> {
    require_migrated(index)?;
    let paths = tag_candidates(index, tags, root, disabled)?
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
            view(&self.index, id, Some(&self.root), &[]).unwrap().paths
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
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
        crate::fingerprint::run_pass(&f.index, &f.root, &[], &[], &never, &|_| {}).unwrap();

        let shown = view(&f.index, bucket, Some(&f.root), &[]).unwrap();
        assert_eq!(shown.paths, vec![copy.clone(), moved.clone()]);
        assert_eq!(shown.missing, 1, "the deleted file is missing, not dropped");

        // A member in a disabled folder is hidden, not missing.
        let hidden = view(&f.index, bucket, Some(&f.root), &[f.root.join("sub")]).unwrap();
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
            crate::fingerprint::run_pass(&f.index, &f.root, &[], &[], &never, &|_| {}).unwrap();
            relink(&f.index).unwrap()
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
        let gone = view(&f.index, bucket, Some(&f.root), &[]).unwrap();
        assert_eq!((gone.paths.len(), gone.missing), (0, 1));
        assert_eq!(f.index.bucket_members(&[bucket]).unwrap().len(), 1);
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
        let art_view = view(&f.index, art, Some(&f.root), &[]).unwrap();
        assert_eq!(
            (art_view.paths, art_view.missing),
            (vec![photo.clone(), render.clone()], 1)
        );
        let diffusion_view = view(&f.index, diffusion, Some(&f.root), &[]).unwrap();
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

        let offers = tag_offers(&f.index, &f.tags, &f.root, &[]).unwrap();
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

        let (bucket, members) =
            bucket_from_tag(&f.index, &f.tags, "test", &f.root, &[], "nature").unwrap();
        assert_eq!((bucket.name.as_str(), members), ("nature", 3));
        let view = view(&f.index, bucket.id, Some(&f.root), &[]).unwrap();
        assert_eq!(view.paths, vec![a.clone(), b.clone()]);
        assert_eq!(view.missing, 1, "the missing file is kept as a member");
        let mut rows_after = f.tags.stored_rows().unwrap();
        rows_after.sort();
        assert_eq!(rows_after, rows_before, "tag rows are untouched");
        assert!(
            tag_offers(&f.index, &f.tags, &f.root, &[])
                .unwrap()
                .iter()
                .all(|o| o.tag != "nature"),
            "a converted tag is no longer offered"
        );
        assert!(
            bucket_from_tag(&f.index, &f.tags, "test", &f.root, &[], "Nature").is_err(),
            "no second bucket with the same name"
        );

        // The conversion takes exactly what the offer counted: the JPEG's
        // hand-made png tag, not the PNG's automatic one.
        let (png, members) =
            bucket_from_tag(&f.index, &f.tags, "test", &f.root, &[], "png").unwrap();
        assert_eq!(members, 1);
        assert_eq!(
            super::view(&f.index, png.id, Some(&f.root), &[])
                .unwrap()
                .paths,
            vec![b.clone()]
        );

        // Disabled folders are neither counted nor written.
        let hidden = f.image("off/d.jpg");
        f.tags.add_tag(&hidden, "comics");
        f.tags.add_tag(&a, "comics");
        let disabled = vec![f.root.join("off")];
        let offers = tag_offers(&f.index, &f.tags, &f.root, &disabled).unwrap();
        assert_eq!(offers.iter().find(|o| o.tag == "comics").unwrap().images, 1);
        let (comics, members) =
            bucket_from_tag(&f.index, &f.tags, "test", &f.root, &disabled, "comics").unwrap();
        assert_eq!(members, 1);
        assert!(f
            .index
            .bucket_members(&[comics.id])
            .unwrap()
            .iter()
            .all(|m| !m.last_path.starts_with(f.root.join("off"))));
    }
}
