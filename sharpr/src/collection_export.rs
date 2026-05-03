use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use glib::DateTime;
use serde::{Deserialize, Serialize};

use crate::library_index::{Collection, LibraryIndex};
use crate::tags::TagDatabase;

#[derive(Serialize, Deserialize, Debug)]
pub struct CollectionNode {
    pub name: String,
    pub primary_tag: String,
    pub extra_tags: Vec<String>,
    pub color: Option<String>,
    pub icon_name: Option<String>,
    pub children: Vec<CollectionNode>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TagAssignment {
    pub path: PathBuf,
    pub tags: Vec<String>,
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
    pub paths_tagged: usize,
}

pub fn export_collections(
    index: &LibraryIndex,
    tags_db: &TagDatabase,
    library_id: &str,
) -> Result<CollectionExport, Box<dyn std::error::Error>> {
    let collections = index.list_collections_for_library(Some(library_id))?;

    // 1. Collect all collection-owned tags
    let mut collection_tags = HashSet::new();
    for c in &collections {
        collection_tags.insert(c.primary_tag.clone());
        for t in &c.extra_tags {
            collection_tags.insert(t.clone());
        }
    }

    // 2. Build set of all paths that have any collection tag
    let mut path_to_tags: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    for tag in &collection_tags {
        let paths = tags_db.paths_for_tag(tag);
        for path in paths {
            path_to_tags.entry(path).or_default().insert(tag.clone());
        }
    }

    let mut tag_assignments: Vec<TagAssignment> = path_to_tags
        .into_iter()
        .map(|(path, tags)| TagAssignment {
            path,
            tags: {
                let mut v: Vec<_> = tags.into_iter().collect();
                v.sort();
                v
            },
        })
        .collect();
    tag_assignments.sort_by(|a, b| a.path.cmp(&b.path));

    // 3. Build collection tree
    let tree = build_collection_tree(&collections, None);

    let exported_at = DateTime::now_utc()
        .and_then(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    Ok(CollectionExport {
        version: 1,
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
    tags_db: &TagDatabase,
    data: &CollectionExport,
    library_id: &str,
) -> Result<ImportSummary, Box<dyn std::error::Error>> {
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

    // 2. Apply tag assignments
    let mut paths_tagged = 0;
    for assignment in &data.tag_assignments {
        tags_db.add_tags_to_paths(std::slice::from_ref(&assignment.path), &assignment.tags);
        paths_tagged += 1;
    }

    Ok(ImportSummary {
        collections_added,
        paths_tagged,
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
