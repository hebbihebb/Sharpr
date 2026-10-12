//! Bucket membership edits, independent of GTK.
//!
//! Buckets are tag-backed until `ROADMAP.md` Phase 4: an image belongs to a
//! bucket when it carries every effective tag, meaning the bucket's own tags
//! plus all of its ancestors' tags. Rename and reparent update every stored
//! member under the library root, including images in disabled folders and
//! files that are temporarily missing, so membership survives re-enabling a
//! folder or a file reappearing.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::library_index::{normalize_collection_tag, Collection, LibraryIndex};
use crate::tags::db::TagChange;
use crate::tags::TagDatabase;

fn in_root(paths: Vec<PathBuf>, root: Option<&Path>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|path| root.is_none_or(|root| path.starts_with(root)))
        .collect()
}

fn local_tags(collection: &Collection) -> Vec<String> {
    let mut tags = vec![collection.primary_tag.clone()];
    for tag in &collection.extra_tags {
        if !tags.contains(tag) {
            tags.push(tag.clone());
        }
    }
    tags
}

fn collection(index: &LibraryIndex, id: i64) -> Result<Collection, String> {
    index
        .collection(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Collection no longer exists".to_string())
}

/// Every stored member of bucket `id` under `root`, whether or not the file
/// currently exists or sits in a disabled folder.
fn stored_members(
    index: &LibraryIndex,
    tags: &TagDatabase,
    id: i64,
    root: Option<&Path>,
) -> Result<Vec<PathBuf>, String> {
    let effective = index
        .collection_effective_tags(id)
        .map_err(|e| e.to_string())?;
    Ok(in_root(
        tags.stored_paths_for_all_tags_checked(&effective)?,
        root,
    ))
}

/// Adds `paths` to bucket `id` by giving them every effective tag.
pub fn add_to_bucket(
    index: &LibraryIndex,
    tags: &TagDatabase,
    id: i64,
    paths: &[PathBuf],
) -> Result<usize, String> {
    let effective = index
        .collection_effective_tags(id)
        .map_err(|e| e.to_string())?;
    tags.change_tags_checked(paths, &effective, false)
}

/// Removes `paths` from bucket `id` and its sub-buckets by removing only the
/// bucket's primary tag. Ancestor and extra tags stay, so the images keep
/// their place in parent and sibling buckets.
pub fn remove_from_bucket(
    index: &LibraryIndex,
    tags: &TagDatabase,
    id: i64,
    paths: &[PathBuf],
) -> Result<usize, String> {
    let bucket = collection(index, id)?;
    tags.change_tags_checked(paths, &[bucket.primary_tag], true)
}

/// Renames bucket `id`, moving its primary tag on every stored member. The
/// index change is rolled back if the tag update fails.
pub fn rename_bucket(
    index: &LibraryIndex,
    tags: &TagDatabase,
    id: i64,
    new_name: &str,
    root: Option<&Path>,
) -> Result<(), String> {
    let before = collection(index, id)?;
    let members = stored_members(index, tags, id, root)?;
    let new_primary = normalize_collection_tag(new_name);
    index
        .update_collection(
            id,
            new_name,
            &before.extra_tags,
            before.color.as_deref(),
            before.icon_name.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    if let Err(error) =
        tags.replace_tag_in_paths_checked(&members, &before.primary_tag, &new_primary)
    {
        if let Err(rollback) = index.update_collection(
            id,
            &before.name,
            &before.extra_tags,
            before.color.as_deref(),
            before.icon_name.as_deref(),
        ) {
            crate::bench_error!(
                "collection.rename.rollback.fail",
                serde_json::json!({"error":rollback.to_string()})
            );
            return Err(format!("{error}; collection rollback failed: {rollback}"));
        }
        return Err(error);
    }
    Ok(())
}

/// Moves bucket `id` (with its sub-buckets) under `parent`, updating the
/// inherited tags of every stored member. Images leave the old ancestors, but
/// keep any tag that a sibling or unrelated bucket still needs for them.
/// Returns the bucket's name.
pub fn reparent_bucket(
    index: &LibraryIndex,
    tags: &TagDatabase,
    id: i64,
    parent: Option<i64>,
    root: Option<&Path>,
) -> Result<String, String> {
    let collections = index.list_collections().map_err(|e| e.to_string())?;
    let source = collections
        .iter()
        .find(|c| c.id == id)
        .ok_or("Collection no longer exists")?;
    let old_parent = source.parent_id;
    let name = source.name.clone();

    let mut subtree = vec![id];
    let mut cursor = 0;
    while cursor < subtree.len() {
        let current = subtree[cursor];
        for child in collections.iter().filter(|c| c.parent_id == Some(current)) {
            if !subtree.contains(&child.id) {
                subtree.push(child.id);
            }
        }
        cursor += 1;
    }

    // The old ancestors lose these images by design. With tag-backed buckets
    // a direct member of an ancestor is indistinguishable from a member of
    // the moved bucket; Phase 4's explicit membership removes that ambiguity.
    let mut old_ancestors = Vec::new();
    let mut next = old_parent;
    while let Some(ancestor) = next.filter(|a| !old_ancestors.contains(a)) {
        old_ancestors.push(ancestor);
        next = collections
            .iter()
            .find(|c| c.id == ancestor)
            .and_then(|c| c.parent_id);
    }

    // Tags each image still needs for other buckets, such as siblings.
    let mut needed: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    for other in collections
        .iter()
        .filter(|c| !subtree.contains(&c.id) && !old_ancestors.contains(&c.id))
    {
        let effective = index
            .collection_effective_tags(other.id)
            .map_err(|e| e.to_string())?;
        for path in stored_members(index, tags, other.id, root)? {
            needed
                .entry(path)
                .or_default()
                .extend(effective.iter().cloned());
        }
    }

    let mut before = Vec::new();
    for member_id in &subtree {
        let bucket = collections
            .iter()
            .find(|c| c.id == *member_id)
            .ok_or("Collection no longer exists")?;
        let effective = index
            .collection_effective_tags(*member_id)
            .map_err(|e| e.to_string())?;
        let members = stored_members(index, tags, *member_id, root)?;
        before.push((*member_id, effective, local_tags(bucket), members));
    }

    index
        .reparent_collection(id, parent)
        .map_err(|e| e.to_string())?;
    let update = (|| -> Result<(), String> {
        let mut batches = Vec::new();
        for (member_id, old, local, members) in before {
            let new = index
                .collection_effective_tags(member_id)
                .map_err(|e| e.to_string())?;
            let add: Vec<String> = new
                .iter()
                .filter(|t| !local.contains(t) && !old.contains(t))
                .cloned()
                .collect();
            let dropped: Vec<&String> = old
                .iter()
                .filter(|t| !local.contains(t) && !new.contains(t))
                .collect();
            for path in &members {
                let keep = needed.get(path);
                let remove: Vec<String> = dropped
                    .iter()
                    .filter(|t| keep.is_none_or(|keep| !keep.contains(**t)))
                    .map(|t| (*t).clone())
                    .collect();
                if !remove.is_empty() {
                    batches.push(TagChange {
                        paths: vec![path.clone()],
                        tags: remove,
                        remove: true,
                    });
                }
            }
            batches.push(TagChange {
                paths: members,
                tags: add,
                remove: false,
            });
        }
        tags.change_tag_batches_checked(&batches)?;
        Ok(())
    })();
    if let Err(error) = update {
        if let Err(rollback) = index.reparent_collection(id, old_parent) {
            crate::bench_error!(
                "collection.reparent.rollback.fail",
                serde_json::json!({"error":rollback.to_string()})
            );
            return Err(format!("{error}; hierarchy rollback failed: {rollback}"));
        }
        return Err(error);
    }
    Ok(name)
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
            Self {
                index: LibraryIndex::open_in_memory().unwrap(),
                tags: TagDatabase::open_in_memory().unwrap(),
                root,
            }
        }

        fn image(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            std::fs::write(&path, b"image").unwrap();
            path
        }

        fn bucket(&self, name: &str, parent: Option<i64>) -> i64 {
            self.index
                .create_collection("test", parent, name, &[], None, None)
                .unwrap()
                .id
        }

        fn members(&self, id: i64) -> Vec<PathBuf> {
            let effective = self.index.collection_effective_tags(id).unwrap();
            self.tags.paths_for_all_tags_checked(&effective).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn removing_from_a_child_keeps_parent_and_sibling_membership() {
        let f = Fixture::new("remove-child");
        let trips = f.bucket("Trips", None);
        let iceland = f.bucket("Iceland", Some(trips));
        let norway = f.bucket("Norway", Some(trips));
        let photo = f.image("fjord.jpg");
        add_to_bucket(&f.index, &f.tags, iceland, std::slice::from_ref(&photo)).unwrap();
        add_to_bucket(&f.index, &f.tags, norway, std::slice::from_ref(&photo)).unwrap();

        remove_from_bucket(&f.index, &f.tags, iceland, std::slice::from_ref(&photo)).unwrap();

        assert!(f.members(iceland).is_empty());
        assert_eq!(f.members(norway), vec![photo.clone()]);
        assert_eq!(f.members(trips), vec![photo]);
    }

    #[test]
    fn removing_from_a_parent_also_removes_from_its_sub_buckets() {
        let f = Fixture::new("remove-parent");
        let art = f.bucket("Art", None);
        let diffusion = f.bucket("Diffusion", Some(art));
        let image = f.image("render.png");
        add_to_bucket(&f.index, &f.tags, diffusion, std::slice::from_ref(&image)).unwrap();

        remove_from_bucket(&f.index, &f.tags, art, std::slice::from_ref(&image)).unwrap();

        assert!(f.members(art).is_empty());
        assert!(f.members(diffusion).is_empty());
        assert_eq!(f.tags.tags_for_path(&image), vec!["diffusion".to_string()]);
    }

    #[test]
    fn reparent_keeps_tags_that_sibling_buckets_still_need() {
        let f = Fixture::new("reparent-sibling");
        let trips = f.bucket("Trips", None);
        let iceland = f.bucket("Iceland", Some(trips));
        let norway = f.bucket("Norway", Some(trips));
        let art = f.bucket("Art", None);
        let photo = f.image("fjord.jpg");
        let only_iceland = f.image("geyser.jpg");
        add_to_bucket(&f.index, &f.tags, iceland, std::slice::from_ref(&photo)).unwrap();
        add_to_bucket(&f.index, &f.tags, norway, std::slice::from_ref(&photo)).unwrap();
        add_to_bucket(
            &f.index,
            &f.tags,
            iceland,
            std::slice::from_ref(&only_iceland),
        )
        .unwrap();

        reparent_bucket(&f.index, &f.tags, iceland, Some(art), Some(&f.root)).unwrap();

        assert_eq!(f.members(norway), vec![photo.clone()]);
        assert_eq!(f.members(trips), vec![photo.clone()]);
        let mut moved = f.members(iceland);
        moved.sort();
        let mut expected = vec![photo, only_iceland.clone()];
        expected.sort();
        assert_eq!(moved, expected);
        assert!(!f
            .tags
            .tags_for_path(&only_iceland)
            .contains(&"trips".to_string()));
    }

    #[test]
    fn rename_and_reparent_carry_members_whose_files_are_missing() {
        let f = Fixture::new("missing-members");
        let trips = f.bucket("Trips", None);
        let iceland = f.bucket("Iceland", Some(trips));
        let art = f.bucket("Art", None);
        let present = f.image("present.jpg");
        let away = f.image("away.jpg");
        add_to_bucket(&f.index, &f.tags, iceland, &[present.clone(), away.clone()]).unwrap();
        // Unplugged drive, disabled folder or a file moved out for a while.
        std::fs::remove_file(&away).unwrap();

        rename_bucket(&f.index, &f.tags, trips, "Travel", Some(&f.root)).unwrap();
        reparent_bucket(&f.index, &f.tags, iceland, Some(art), Some(&f.root)).unwrap();
        std::fs::write(&away, b"image").unwrap();

        let mut members = f.members(iceland);
        members.sort();
        let mut expected = vec![present, away.clone()];
        expected.sort();
        assert_eq!(members, expected);
        assert!(
            f.members(trips).is_empty(),
            "moved images leave the old parent"
        );
        let mut tags = f.tags.tags_for_path(&away);
        tags.sort();
        assert_eq!(tags, vec!["art".to_string(), "iceland".to_string()]);
    }
}
