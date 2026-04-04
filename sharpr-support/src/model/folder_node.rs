use std::cell::RefCell;
use std::path::PathBuf;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

// ---------------------------------------------------------------------------
// GObject subclass
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct FolderNode {
        pub path: RefCell<PathBuf>,
        pub display_name: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FolderNode {
        const NAME: &'static str = "SharprFolderNode";
        type Type = super::FolderNode;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for FolderNode {}
}

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

glib::wrapper! {
    pub struct FolderNode(ObjectSubclass<imp::FolderNode>);
}

impl FolderNode {
    pub fn new(path: PathBuf, display_name: impl Into<String>) -> Self {
        let node: Self = glib::Object::new();
        *node.imp().path.borrow_mut() = path;
        *node.imp().display_name.borrow_mut() = display_name.into();
        node
    }

    pub fn path(&self) -> PathBuf {
        self.imp().path.borrow().clone()
    }

    pub fn display_name(&self) -> String {
        self.imp().display_name.borrow().clone()
    }

    /// Returns child directories of this node's path (non-recursive).
    pub fn child_directories(&self) -> Vec<FolderNode> {
        let path = self.imp().path.borrow().clone();
        let Ok(entries) = std::fs::read_dir(&path) else {
            return Vec::new();
        };

        let mut children: Vec<FolderNode> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter(|e| {
                // Skip hidden directories
                !e.file_name().to_string_lossy().starts_with('.')
            })
            .map(|e| {
                let child_path = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                FolderNode::new(child_path, name)
            })
            .collect();

        children.sort_by(|a, b| {
            a.display_name()
                .to_lowercase()
                .cmp(&b.display_name().to_lowercase())
        });
        children
    }
}
