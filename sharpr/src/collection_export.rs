use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use glib::DateTime;
use serde::{Deserialize, Serialize};

use crate::library_index::{Collection, LibraryIndex};

#[derive(Serialize, Deserialize, Debug)]
pub struct CollectionNode {
    pub name: String,
    pub primary_tag: String,
    pub extra_tags: Vec<String>,
    pub color: Option<String>,
    pub icon_name: Option<String>,
    pub children: Vec<CollectionNode>,
}

/// One image and the buckets it belongs to, named by their primary tags.
/// Version 1 files listed every tag of the image instead; both import.
#[derive(Serialize, Deserialize, Debug)]
pub struct TagAssignment {
    pub path: PathBuf,
    pub tags: Vec<String>,
    /// Content fingerprint, so an import can find the image if it moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CollectionExport {
    pub version: u32,
    pub app: String,
    pub exported_at: String,
    pub library_id: String,
    pub collections: Vec<CollectionNode>,
    pub tag_assignments: Vec<TagAssignment>,
}

pub struct ImportSummary {
    pub collections_added: usize,
    pub paths_added: usize,
}

pub fn export_collections(
    index: &LibraryIndex,
    library_id: &str,
) -> Result<CollectionExport, Box<dyn std::error::Error>> {
    let collections = index.list_collections_for_library(Some(library_id))?;
    let tag_of: HashMap<i64, &str> = collections
        .iter()
        .map(|c| (c.id, c.primary_tag.as_str()))
        .collect();
    let ids: Vec<i64> = collections.iter().map(|c| c.id).collect();

    // Each stored member, missing ones included, with its own buckets.
    let mut by_path: HashMap<PathBuf, (HashSet<String>, Option<String>)> = HashMap::new();
    for row in index.bucket_members(&ids)? {
        let entry = by_path.entry(row.last_path).or_default();
        if let Some(tag) = tag_of.get(&row.collection_id) {
            entry.0.insert((*tag).to_string());
        }
        if entry.1.is_none() {
            entry.1 = row.fingerprint;
        }
    }
    let mut tag_assignments: Vec<TagAssignment> = by_path
        .into_iter()
        .map(|(path, (tags, fingerprint))| {
            let mut tags: Vec<_> = tags.into_iter().collect();
            tags.sort();
            TagAssignment {
                path,
                tags,
                fingerprint,
            }
        })
        .collect();
    tag_assignments.sort_by(|a, b| a.path.cmp(&b.path));

    let tree = build_collection_tree(&collections, None);
    let exported_at = DateTime::now_utc()
        .and_then(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    Ok(CollectionExport {
        version: 2,
        app: "sharpr".to_string(),
        exported_at,
        library_id: library_id.to_string(),
        collections: tree,
        tag_assignments,
    })
}

fn build_collection_tree(
    collections: &[Collection],
    parent_id: Option<i64>,
) -> Vec<CollectionNode> {
    collections
        .iter()
        .filter(|c| c.parent_id == parent_id)
        .map(|c| CollectionNode {
            name: c.name.clone(),
            primary_tag: c.primary_tag.clone(),
            extra_tags: c.extra_tags.clone(),
            color: c.color.clone(),
            icon_name: c.icon_name.clone(),
            children: build_collection_tree(collections, Some(c.id)),
        })
        .collect()
}

pub fn import_collections(
    index: &LibraryIndex,
    data: &CollectionExport,
    library_id: &str,
) -> Result<ImportSummary, Box<dyn std::error::Error>> {
    if !index.is_membership_migrated()? {
        return Err("buckets have not been upgraded yet; restart Sharpr and try again".into());
    }
    let existing_collections = index.list_collections_for_library(Some(library_id))?;
    let mut tag_to_id: HashMap<String, i64> = existing_collections
        .into_iter()
        .map(|c| (c.primary_tag, c.id))
        .collect();

    let mut collections_added = 0;

    // 1. Walk the tree and create missing collections
    import_node_recursive(
        index,
        &data.collections,
        None,
        &mut tag_to_id,
        &mut collections_added,
        library_id,
    )?;

    // 2. Add each image to the buckets named by its tags; other tags are
    // ignored, and no tags are written.
    let mut members: HashMap<i64, Vec<(PathBuf, Option<String>)>> = HashMap::new();
    let mut paths = HashSet::new();
    for assignment in &data.tag_assignments {
        for tag in &assignment.tags {
            if let Some(id) = tag_to_id.get(tag) {
                members
                    .entry(*id)
                    .or_default()
                    .push((assignment.path.clone(), assignment.fingerprint.clone()));
                paths.insert(assignment.path.clone());
            }
        }
    }
    for (id, members) in members {
        index.add_bucket_members(id, &members)?;
    }

    Ok(ImportSummary {
        collections_added,
        paths_added: paths.len(),
    })
}

fn import_node_recursive(
    index: &LibraryIndex,
    nodes: &[CollectionNode],
    parent_id: Option<i64>,
    tag_to_id: &mut HashMap<String, i64>,
    added_count: &mut usize,
    library_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for node in nodes {
        let collection_id = if let Some(&id) = tag_to_id.get(&node.primary_tag) {
            id
        } else {
            let collection = index.create_collection(
                library_id,
                parent_id,
                &node.name,
                &node.extra_tags,
                node.color.as_deref(),
                node.icon_name.as_deref(),
            )?;
            tag_to_id.insert(node.primary_tag.clone(), collection.id);
            *added_count += 1;
            collection.id
        };

        import_node_recursive(
            index,
            &node.children,
            Some(collection_id),
            tag_to_id,
            added_count,
            library_id,
        )?;
    }
    Ok(())
}
