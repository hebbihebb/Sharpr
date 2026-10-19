use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;

use crate::config::settings::LibraryConfig;
use crate::library_index::Collection;
use crate::model::walk;
use crate::ui::state::AppState;

type FolderSelectedCallback = Box<dyn Fn(PathBuf) + 'static>;
type FolderIgnoredChangedCallback = Box<dyn Fn(PathBuf, bool) + 'static>;
type CollectionSelectedCallback = Box<dyn Fn(i64) + 'static>;
type CollectionAddRequestedCallback = Box<dyn Fn() + 'static>;
type CollectionRenameRequestedCallback = Box<dyn Fn(i64) + 'static>;
type CollectionColorRequestedCallback = Box<dyn Fn(i64) + 'static>;
type CollectionPromoteRequestedCallback = Box<dyn Fn(i64) + 'static>;
type CollectionMoveRequestedCallback = Box<dyn Fn(i64) + 'static>;
type CollectionReparentRequestedCallback = Box<dyn Fn(i64, i64) + 'static>;
type CollectionDeleteRequestedCallback = Box<dyn Fn(i64) + 'static>;
type DropPathsToCollectionCallback = Box<dyn Fn(i64, Vec<std::path::PathBuf>) + 'static>;

/// How often a running folder scan hands what it found to the sidebar.
const FOLDER_SCAN_BATCH: Duration = Duration::from_millis(200);

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jxl", "png", "gif", "webp", "tiff", "tif", "bmp", "ico", "avif", "heic", "heif",
];

#[derive(Clone, Debug)]
struct FolderListEntry {
    path: PathBuf,
    label: String,
    depth: u32,
    has_children: bool,
}

#[derive(Clone, Debug)]
struct FolderNode {
    path: PathBuf,
    children: Vec<FolderNode>,
}

mod imp {
    use super::*;

    pub struct SidebarPane {
        pub toolbar_view: libadwaita::ToolbarView,
        pub header_bar: RefCell<Option<libadwaita::HeaderBar>>,
        pub folder_generation: Cell<u64>,
        pub list_box: gtk4::ListBox,
        pub collection_list: gtk4::ListBox,
        pub bucket_box: gtk4::Box,
        pub active_library_label: gtk4::Label,
        pub folder_selected_cb: RefCell<Option<FolderSelectedCallback>>,
        pub folder_ignored_changed_cb: RefCell<Option<FolderIgnoredChangedCallback>>,
        pub collection_selected_cb: RefCell<Option<CollectionSelectedCallback>>,
        pub collection_add_requested_cb: RefCell<Option<CollectionAddRequestedCallback>>,
        pub collection_rename_requested_cb: RefCell<Option<CollectionRenameRequestedCallback>>,
        pub collection_color_requested_cb: RefCell<Option<CollectionColorRequestedCallback>>,
        pub collection_promote_requested_cb: RefCell<Option<CollectionPromoteRequestedCallback>>,
        pub collection_move_requested_cb: RefCell<Option<CollectionMoveRequestedCallback>>,
        pub collection_reparent_requested_cb: RefCell<Option<CollectionReparentRequestedCallback>>,
        pub collection_delete_requested_cb: RefCell<Option<CollectionDeleteRequestedCallback>>,
        pub drop_paths_to_collection_cb: RefCell<Option<DropPathsToCollectionCallback>>,
        pub suppress_folder_signal: Cell<bool>,
        pub suppress_collection_signal: Cell<bool>,
        pub collapsed_folder_paths: RefCell<HashSet<PathBuf>>,
        pub(super) folder_entries: RefCell<Vec<FolderListEntry>>,
        pub(super) folder_tree: RefCell<Vec<FolderNode>>,
        /// Folders listed since the last refresh, so only new ones start collapsed.
        pub(super) known_folder_paths: RefCell<HashSet<PathBuf>>,
        pub(super) folder_scan_canceled: RefCell<Option<Arc<AtomicBool>>>,
        /// A scan (the first, or a rescan) is walking the roots.
        pub(super) scan_running: Cell<bool>,
        /// Something changed on disk while a scan ran; scan again when it ends.
        pub(super) rescan_wanted: Cell<bool>,
        /// Runs after a scan has listed the folders (the first or a rescan).
        pub(super) folders_scanned_cb: RefCell<Option<Box<dyn Fn()>>>,
        /// The library's roots, first first, as of the last scan (D41).
        pub(super) library_roots: RefCell<Vec<PathBuf>>,
        pub ignored_folders: RefCell<Vec<PathBuf>>,
        pub collapsed_collection_ids: RefCell<HashSet<i64>>,
        pub collections: RefCell<Vec<Collection>>,
    }

    impl Default for SidebarPane {
        fn default() -> Self {
            Self {
                toolbar_view: libadwaita::ToolbarView::new(),
                header_bar: RefCell::new(None),
                folder_generation: Cell::new(0),
                list_box: gtk4::ListBox::new(),
                collection_list: gtk4::ListBox::new(),
                bucket_box: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
                active_library_label: gtk4::Label::new(None),
                folder_selected_cb: RefCell::new(None),
                folder_ignored_changed_cb: RefCell::new(None),
                collection_selected_cb: RefCell::new(None),
                collection_add_requested_cb: RefCell::new(None),
                collection_rename_requested_cb: RefCell::new(None),
                collection_color_requested_cb: RefCell::new(None),
                collection_promote_requested_cb: RefCell::new(None),
                collection_move_requested_cb: RefCell::new(None),
                collection_reparent_requested_cb: RefCell::new(None),
                collection_delete_requested_cb: RefCell::new(None),
                drop_paths_to_collection_cb: RefCell::new(None),
                suppress_folder_signal: Cell::new(false),
                suppress_collection_signal: Cell::new(false),
                collapsed_folder_paths: RefCell::new(HashSet::new()),
                library_roots: RefCell::new(Vec::new()),
                folder_entries: RefCell::new(Vec::new()),
                folder_tree: RefCell::new(Vec::new()),
                known_folder_paths: RefCell::new(HashSet::new()),
                folder_scan_canceled: RefCell::new(None),
                scan_running: Cell::new(false),
                rescan_wanted: Cell::new(false),
                folders_scanned_cb: RefCell::new(None),
                ignored_folders: RefCell::new(Vec::new()),
                collapsed_collection_ids: RefCell::new(HashSet::new()),
                collections: RefCell::new(Vec::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SidebarPane {
        const NAME: &'static str = "SharprSidebarPane";
        type Type = super::SidebarPane;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for SidebarPane {
        fn dispose(&self) {
            self.toolbar_view.unparent();
        }
    }

    impl WidgetImpl for SidebarPane {}
}

glib::wrapper! {
    pub struct SidebarPane(ObjectSubclass<imp::SidebarPane>)
        @extends gtk4::Widget,
                 @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Actionable;
}

impl SidebarPane {
    pub fn new(state: Rc<RefCell<AppState>>) -> Self {
        let widget: Self = glib::Object::new();
        widget.build_ui(state);
        widget
    }

    fn build_ui(&self, state: Rc<RefCell<AppState>>) {
        let imp = self.imp();

        let header = libadwaita::HeaderBar::new();
        header.set_show_end_title_buttons(false);

        imp.toolbar_view.add_top_bar(&header);
        *imp.header_bar.borrow_mut() = Some(header.clone());

        imp.list_box.add_css_class("navigation-sidebar");
        imp.list_box.set_activate_on_single_click(true);
        imp.list_box.set_selection_mode(gtk4::SelectionMode::Single);

        let widget_weak = self.downgrade();
        imp.list_box.connect_selected_rows_changed(move |list_box| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            if widget.imp().suppress_folder_signal.get() {
                return;
            }
            let Some(row) = list_box.selected_row() else {
                return;
            };
            let Some(folder_row) = row.downcast_ref::<FolderRow>() else {
                return;
            };
            if folder_row.ignored() {
                widget.imp().suppress_folder_signal.set(true);
                list_box.unselect_row(&row);
                widget.imp().suppress_folder_signal.set(false);
                return;
            }
            crate::bench_event!(
                "sidebar.folder_selected_ui",
                serde_json::json!({
                    "path": folder_row.path().display().to_string(),
                    "has_children": folder_row.has_children(),
                    "ignored": false,
                }),
            );
            widget.clear_collection_selection();
            widget.emit_folder_selected(folder_row.path());
        });

        let widget_weak = self.downgrade();
        imp.list_box.connect_row_activated(move |_, row| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(folder_row) = row.downcast_ref::<FolderRow>() else {
                return;
            };
            crate::bench_event!(
                "sidebar.folder_row_activated",
                serde_json::json!({
                    "path": folder_row.path().display().to_string(),
                    "has_children": folder_row.has_children(),
                }),
            );
            if folder_row_activation_toggles_collapse(folder_row.has_children()) {
                widget.toggle_folder_collapsed(folder_row.path());
            }
        });

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&imp.list_box));
        let active_library_label = imp.active_library_label.clone();
        active_library_label.add_css_class("caption-heading");
        set_bold_label_text(&active_library_label, "Folders");
        active_library_label.set_halign(gtk4::Align::Start);
        active_library_label.set_margin_start(12);
        active_library_label.set_margin_top(8);
        active_library_label.set_margin_bottom(4);

        self.populate_default_folders(state.clone());

        // Collections section header: label + "New Bucket" + button
        let collections_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        let collections_label = section_label("Buckets");
        collections_label.set_hexpand(true);
        let new_collection_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        new_collection_btn.add_css_class("flat");
        new_collection_btn.set_tooltip_text(Some("New Bucket"));
        new_collection_btn.set_valign(gtk4::Align::Center);
        new_collection_btn.set_margin_end(8);
        collections_header.append(&collections_label);
        collections_header.append(&new_collection_btn);

        imp.collection_list.add_css_class("navigation-sidebar");
        imp.collection_list
            .set_selection_mode(gtk4::SelectionMode::Single);

        let widget_weak = self.downgrade();
        new_collection_btn.connect_clicked(move |_| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            widget.emit_collection_add_requested();
        });

        let widget_weak = self.downgrade();
        imp.collection_list
            .connect_selected_rows_changed(move |list| {
                let Some(widget) = widget_weak.upgrade() else {
                    return;
                };
                if widget.imp().suppress_collection_signal.get() {
                    return;
                }
                let Some(row) = list.selected_row() else {
                    return;
                };
                let Some(coll_row) = row.downcast_ref::<CollectionRow>() else {
                    return;
                };
                widget.clear_folder_selection();
                widget.emit_collection_selected(coll_row.collection_id());
            });

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        vbox.append(&active_library_label);
        vbox.append(&scroll);
        imp.bucket_box.append(&collections_header);
        imp.bucket_box.append(&imp.collection_list);
        vbox.append(&imp.bucket_box);

        imp.toolbar_view.set_content(Some(&vbox));
        imp.toolbar_view.set_parent(self);
    }

    /// Scans the library roots on a worker, one after another, and lists
    /// folders as it finds them, so a large root such as Home fills in instead
    /// of showing nothing until the walk ends. Each root is a top-level entry,
    /// in root order. The first folder is chosen once the scan is done.
    fn populate_default_folders(&self, state: Rc<RefCell<AppState>>) {
        let generation = self.imp().folder_generation.get().wrapping_add(1);
        self.imp().folder_generation.set(generation);
        let canceled = Arc::new(AtomicBool::new(false));
        if let Some(previous) = self
            .imp()
            .folder_scan_canceled
            .replace(Some(canceled.clone()))
        {
            previous.store(true, Ordering::Relaxed);
        }
        self.imp().scan_running.set(true);
        let (tx, rx) = async_channel::unbounded::<Vec<PathBuf>>();
        let active_library = state.borrow().settings.active_library().cloned();
        let roots = active_library
            .as_ref()
            .map(LibraryConfig::roots)
            .unwrap_or_default();
        *self.imp().library_roots.borrow_mut() = roots.clone();
        std::thread::spawn(move || {
            let Some(library) = active_library else {
                return;
            };
            let pending = RefCell::new((Vec::<PathBuf>::new(), Instant::now()));
            let flush = |force: bool| {
                let mut pending = pending.borrow_mut();
                if !pending.0.is_empty() && (force || pending.1.elapsed() >= FOLDER_SCAN_BATCH) {
                    let _ = tx.send_blocking(std::mem::take(&mut pending.0));
                    pending.1 = Instant::now();
                }
            };
            for root in library.roots() {
                scan_image_folders(
                    &root,
                    &library.ignored_folders,
                    &|| {
                        flush(false);
                        canceled.load(Ordering::Relaxed) || tx.is_closed()
                    },
                    &mut |path| pending.borrow_mut().0.push(path.to_path_buf()),
                );
            }
            flush(true);
        });

        let widget_weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let mut found = HashSet::new();
            while let Ok(batch) = rx.recv().await {
                let Some(widget) = widget_weak.upgrade() else {
                    return;
                };
                if widget.imp().folder_generation.get() != generation {
                    return;
                }
                found.extend(batch);
                let tree: Vec<FolderNode> = roots
                    .iter()
                    .flat_map(|root| folder_tree_from(root, &found))
                    .collect();
                let ignored_folders = state.borrow().disabled_folders.clone();
                widget.set_folder_tree(&tree, &ignored_folders);

                // Open the folder the view was on as soon as its row exists,
                // unless the user has picked another in the meantime.
                if let crate::ui::state::ViewScope::Folder(path) = state.borrow().scope.clone() {
                    if widget.imp().list_box.selected_row().is_none() {
                        widget.select_folder(&path);
                    }
                }
            }
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            if widget.imp().folder_generation.get() != generation {
                return;
            }
            let on_a_folder =
                matches!(state.borrow().scope, crate::ui::state::ViewScope::Folder(_));
            if !on_a_folder && widget.imp().list_box.selected_row().is_none() {
                if let Some(path) = widget.preferred_initial_folder_path() {
                    widget.select_folder(&path);
                    widget.emit_folder_selected(path);
                }
            }
            widget.scan_finished(state);
        });
    }

    /// Lists the folders again after something changed on disk, keeping the
    /// selection and scroll position: the roots are walked on a worker and the
    /// list is updated once, with rows added in place. A scan that is already
    /// running is asked to go round again when it ends.
    pub fn rescan_folders(&self, state: Rc<RefCell<AppState>>) {
        let imp = self.imp();
        if imp.scan_running.get() {
            imp.rescan_wanted.set(true);
            return;
        }
        let roots = imp.library_roots.borrow().clone();
        let Some(library) = state.borrow().settings.active_library().cloned() else {
            return;
        };
        if roots.is_empty() {
            return;
        }
        imp.scan_running.set(true);
        let generation = imp.folder_generation.get();
        let canceled = Arc::new(AtomicBool::new(false));
        if let Some(previous) = imp.folder_scan_canceled.replace(Some(canceled.clone())) {
            previous.store(true, Ordering::Relaxed);
        }
        let (tx, rx) = async_channel::bounded::<Vec<PathBuf>>(1);
        let walked = roots.clone();
        std::thread::spawn(move || {
            let mut found = Vec::new();
            for root in &walked {
                scan_image_folders(
                    root,
                    &library.ignored_folders,
                    &|| canceled.load(Ordering::Relaxed) || tx.is_closed(),
                    &mut |path| found.push(path.to_path_buf()),
                );
            }
            if !canceled.load(Ordering::Relaxed) {
                let _ = tx.send_blocking(found);
            }
        });
        let widget_weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let found = rx.recv().await.ok();
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            // A new first scan (another root, say) owns the list now.
            if widget.imp().folder_generation.get() != generation {
                return;
            }
            if let Some(found) = found {
                let found: HashSet<PathBuf> = found.into_iter().collect();
                let tree: Vec<FolderNode> = roots
                    .iter()
                    .flat_map(|root| folder_tree_from(root, &found))
                    .collect();
                let ignored_folders = state.borrow().disabled_folders.clone();
                widget.set_folder_tree(&tree, &ignored_folders);
                if let crate::ui::state::ViewScope::Folder(path) = state.borrow().scope.clone() {
                    widget.select_folder(&path);
                }
            }
            widget.scan_finished(state);
        });
    }

    fn scan_finished(&self, state: Rc<RefCell<AppState>>) {
        self.imp().scan_running.set(false);
        if let Some(callback) = self.imp().folders_scanned_cb.borrow().as_ref() {
            callback();
        }
        if self.imp().rescan_wanted.replace(false) {
            self.rescan_folders(state);
        }
    }

    pub fn connect_folders_scanned<F: Fn() + 'static>(&self, f: F) {
        *self.imp().folders_scanned_cb.borrow_mut() = Some(Box::new(f));
    }

    /// The library roots and the folders listed under them, none of them
    /// disabled: where a change on disk can add a folder or an image.
    pub fn watchable_folders(&self) -> Vec<PathBuf> {
        let ignored = self.imp().ignored_folders.borrow();
        let mut folders = self.imp().library_roots.borrow().clone();
        for entry in self.imp().folder_entries.borrow().iter() {
            if !row_is_ignored(&entry.path, &ignored) && !folders.contains(&entry.path) {
                folders.push(entry.path.clone());
            }
        }
        folders
    }

    /// Shows `entries`. With `grow_in_place`, rows that were only added since
    /// the last call are inserted where they belong, keeping the selection and
    /// scroll position while a scan fills the list; otherwise, or when rows
    /// went away, the list is rebuilt.
    fn replace_folder_rows(
        &self,
        entries: &[FolderListEntry],
        ignored_folders: &[PathBuf],
        grow_in_place: bool,
    ) {
        let previous = self.imp().folder_entries.replace(entries.to_vec());
        *self.imp().ignored_folders.borrow_mut() = ignored_folders.to_vec();
        let added = rows_added(&previous, entries).filter(|_| grow_in_place);
        let in_place = added.is_some();
        if !in_place {
            while let Some(child) = self.imp().list_box.first_child() {
                self.imp().list_box.remove(&child);
            }
        }
        let added: HashSet<usize> = added.into_iter().flatten().collect();
        let mut existing = self.imp().list_box.first_child();

        for (index, entry) in entries.iter().enumerate() {
            let collapsed = self
                .imp()
                .collapsed_folder_paths
                .borrow()
                .contains(&entry.path);
            if in_place && !added.contains(&index) {
                if let Some(row) = existing.take() {
                    existing = row.next_sibling();
                    if let Ok(row) = row.downcast::<FolderRow>() {
                        row.set_has_children(entry.has_children, collapsed);
                    }
                }
                continue;
            }
            let row = FolderRow::new(
                entry.path.clone(),
                &entry.label,
                entry.depth,
                entry.has_children,
                collapsed,
                root_color(&self.imp().library_roots.borrow(), &entry.path),
            );
            row.set_ignored(row_is_ignored(&row.path(), ignored_folders));
            self.attach_folder_row_menu(&row);
            self.imp().list_box.insert(&row, index as i32);
        }
    }

    fn set_folder_tree(&self, tree: &[FolderNode], ignored_folders: &[PathBuf]) {
        self.show_folder_tree(tree, ignored_folders, true);
    }

    fn show_folder_tree(&self, tree: &[FolderNode], ignored_folders: &[PathBuf], grow: bool) {
        {
            // Folders seen for the first time start collapsed (below the
            // root); ones the user has already opened or closed keep their state.
            let mut known = self.imp().known_folder_paths.borrow_mut();
            let mut collapsed = self.imp().collapsed_folder_paths.borrow_mut();
            collapsed.extend(
                default_collapsed_folder_paths(tree)
                    .into_iter()
                    .filter(|path| !known.contains(path)),
            );
            collect_folder_paths(tree, &mut known);
        }
        *self.imp().folder_tree.borrow_mut() = tree.to_vec();
        let entries = visible_folder_entries(tree, &self.imp().collapsed_folder_paths.borrow());
        self.replace_folder_rows(&entries, ignored_folders, grow);
    }

    pub fn refresh_active_library(&self, state: Rc<RefCell<AppState>>) {
        self.imp().active_library_label.set_tooltip_text(Some(
            state
                .borrow()
                .settings
                .library_roots()
                .iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
                .as_str(),
        ));
        // A new scan lists folders afresh: below the root they start collapsed.
        self.imp().known_folder_paths.borrow_mut().clear();
        self.set_folder_tree(&[], &state.borrow().disabled_folders);
        self.populate_default_folders(state);
    }

    pub fn connect_folder_selected<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.imp().folder_selected_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_folder_ignored_changed<F: Fn(PathBuf, bool) + 'static>(&self, f: F) {
        *self.imp().folder_ignored_changed_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_folder_ignored(&self, path: &Path, ignored: bool) {
        let mut child = self.imp().list_box.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Ok(row) = widget.downcast::<FolderRow>() {
                if row.path() == path {
                    row.set_ignored(ignored);
                    break;
                }
            }
            child = next;
        }
    }

    pub fn set_ignored_folders(&self, ignored_folders: &[PathBuf]) {
        let mut child = self.imp().list_box.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Ok(row) = widget.downcast::<FolderRow>() {
                row.set_ignored(row_is_ignored(&row.path(), ignored_folders));
            }
            child = next;
        }
    }

    /// Returns the path of the first folder row in the sidebar list, if any.
    /// Look prototype (Phase 9): an empty, flat header over the sidebar, so
    /// the top row lines up with the columns below. Open Folder, the library
    /// switcher and the "Library" title go (one library root replaces them).
    pub fn quiet_header(&self) {
        let imp = self.imp();
        if let Some(header) = imp.header_bar.borrow().as_ref() {
            header.set_title_widget(Some(&gtk4::Box::new(gtk4::Orientation::Horizontal, 0)));
            header.add_css_class("sharpr-top");
        }
        self.add_css_class("sharpr-sidebar");
    }

    pub fn first_folder_path(&self) -> Option<PathBuf> {
        self.preferred_initial_folder_path()
    }

    pub fn select_folder(&self, path: &Path) {
        let mut child = self.imp().list_box.first_child();
        let mut matched = false;
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Ok(row) = widget.downcast::<FolderRow>() {
                if row.path() == path {
                    self.imp().suppress_folder_signal.set(true);
                    self.imp().list_box.select_row(Some(&row));
                    self.imp().suppress_folder_signal.set(false);
                    matched = true;
                    break;
                }
            }
            child = next;
        }
        crate::bench_event!(
            "sidebar.select_folder",
            serde_json::json!({
                "path": path.display().to_string(),
                "matched": matched,
            }),
        );
    }

    fn clear_folder_selection(&self) {
        self.imp().suppress_folder_signal.set(true);
        self.imp().list_box.unselect_all();
        self.imp().suppress_folder_signal.set(false);
    }

    fn preferred_initial_folder_path(&self) -> Option<PathBuf> {
        preferred_initial_folder_path(
            &self.imp().folder_tree.borrow(),
            &self.imp().folder_entries.borrow(),
            &self.imp().ignored_folders.borrow(),
        )
    }

    fn emit_folder_selected(&self, path: PathBuf) {
        crate::bench_event!(
            "sidebar.folder_selected_emit",
            serde_json::json!({
                "path": path.display().to_string(),
            }),
        );
        if let Some(cb) = self.imp().folder_selected_cb.borrow().as_ref() {
            cb(path);
        }
    }

    /// Disables or enables a folder as its row's menu does, from elsewhere
    /// (Preferences lists the roots).
    pub fn change_folder_ignored(&self, path: PathBuf, ignored: bool) {
        self.set_folder_ignored(&path, ignored);
        self.emit_folder_ignored_changed(path, ignored);
    }

    fn emit_folder_ignored_changed(&self, path: PathBuf, ignored: bool) {
        if let Some(cb) = self.imp().folder_ignored_changed_cb.borrow().as_ref() {
            cb(path, ignored);
        }
    }

    // ---- Collection callbacks ----

    pub fn connect_collection_selected<F: Fn(i64) + 'static>(&self, f: F) {
        *self.imp().collection_selected_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_collection_add_requested<F: Fn() + 'static>(&self, f: F) {
        *self.imp().collection_add_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_collection_rename_requested<F: Fn(i64) + 'static>(&self, f: F) {
        *self.imp().collection_rename_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_collection_color_requested<F: Fn(i64) + 'static>(&self, f: F) {
        *self.imp().collection_color_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_collection_promote_requested<F: Fn(i64) + 'static>(&self, f: F) {
        *self.imp().collection_promote_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_collection_move_requested<F: Fn(i64) + 'static>(&self, f: F) {
        *self.imp().collection_move_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_collection_reparent_requested<F: Fn(i64, i64) + 'static>(&self, f: F) {
        *self.imp().collection_reparent_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_collection_delete_requested<F: Fn(i64) + 'static>(&self, f: F) {
        *self.imp().collection_delete_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn clear_collection_selection(&self) {
        self.imp().suppress_collection_signal.set(true);
        self.imp().collection_list.unselect_all();
        self.imp().suppress_collection_signal.set(false);
    }

    pub fn set_collection_selected(&self, id: i64) {
        let ancestors = {
            let collections = self.imp().collections.borrow();
            let by_id: HashMap<i64, Option<i64>> =
                collections.iter().map(|c| (c.id, c.parent_id)).collect();
            let mut ancestors = Vec::new();
            let mut current = by_id.get(&id).copied().flatten();
            while let Some(parent_id) = current {
                ancestors.push(parent_id);
                current = by_id.get(&parent_id).copied().flatten();
            }
            ancestors
        };
        if !ancestors.is_empty() {
            {
                let mut collapsed = self.imp().collapsed_collection_ids.borrow_mut();
                for ancestor in ancestors {
                    collapsed.remove(&ancestor);
                }
            }
            let collections = self.imp().collections.borrow().clone();
            self.refresh_collections(&collections);
        }
        self.imp().suppress_collection_signal.set(true);
        let mut child = self.imp().collection_list.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Ok(row) = widget.downcast::<CollectionRow>() {
                if row.collection_id() == id {
                    self.imp().collection_list.select_row(Some(&row));
                    break;
                }
            }
            child = next;
        }
        self.imp().suppress_collection_signal.set(false);
    }

    /// Rebuild the collection list from the given slice. Safe to call at any time.
    pub fn refresh_collections(&self, collections: &[Collection]) {
        let imp = self.imp();
        *imp.collections.borrow_mut() = collections.to_vec();
        let root_colors = collection_root_colors(collections);
        let selected_id = imp
            .collection_list
            .selected_row()
            .and_then(|row| row.downcast::<CollectionRow>().ok())
            .map(|row| row.collection_id());
        // A focused row that is removed makes GTK move the focus on as Tab
        // does, which can select a row elsewhere and open it. Let go of the
        // focus first and give it to the reselected row afterwards.
        let root = self.root();
        let had_focus = root
            .as_ref()
            .and_then(|root| root.focus())
            .is_some_and(|focus| focus.is_ancestor(&imp.collection_list));
        if had_focus {
            if let Some(root) = &root {
                root.set_focus(None::<&gtk4::Widget>);
            }
        }
        imp.suppress_collection_signal.set(true);
        imp.collection_list.unselect_all();
        while let Some(child) = imp.collection_list.first_child() {
            imp.collection_list.remove(&child);
        }
        for coll in visible_collections(collections, &imp.collapsed_collection_ids.borrow()) {
            let has_children = collections.iter().any(|c| c.parent_id == Some(coll.id));
            let is_collapsed = imp.collapsed_collection_ids.borrow().contains(&coll.id);
            let effective_color = coll
                .color
                .as_deref()
                .or_else(|| {
                    root_colors
                        .get(&root_collection_id(coll.id, collections))
                        .map(String::as_str)
                })
                .unwrap_or_else(|| fallback_collection_color(coll.id));
            let row = CollectionRow::new(
                &coll,
                effective_color,
                collection_depth(coll.id, collections),
                has_children,
                is_collapsed,
            );
            self.attach_collection_row_menu(&row);
            imp.collection_list.append(&row);
        }
        if let Some(selected_id) = selected_id {
            let mut child = imp.collection_list.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if let Ok(row) = widget.downcast::<CollectionRow>() {
                    if row.collection_id() == selected_id {
                        imp.collection_list.select_row(Some(&row));
                        if had_focus {
                            row.grab_focus();
                        }
                        break;
                    }
                }
                child = next;
            }
        }
        imp.suppress_collection_signal.set(false);
    }

    fn attach_collection_row_menu(&self, row: &CollectionRow) {
        if let Some(disclosure) = row.disclosure_button() {
            let row_weak = row.downgrade();
            let widget_weak = self.downgrade();
            disclosure.connect_clicked(move |_| {
                let Some(widget) = widget_weak.upgrade() else {
                    return;
                };
                let Some(row) = row_weak.upgrade() else {
                    return;
                };
                widget.toggle_collection_collapsed(row.collection_id());
            });
        }

        if !row.has_children() {
            let drag_source = gtk4::DragSource::new();
            drag_source.set_actions(gdk4::DragAction::MOVE);
            let collection_id = row.collection_id();
            drag_source.connect_prepare(move |_, _, _| {
                Some(gdk4::ContentProvider::for_value(
                    &format!("collection:{collection_id}").to_value(),
                ))
            });
            row.add_controller(drag_source);
        }

        // Drop target: accept path strings dragged from the filmstrip.
        let id = row.collection_id();
        let widget_weak = self.downgrade();
        let drop_target = gtk4::DropTarget::new(
            glib::Type::STRING,
            gdk4::DragAction::COPY | gdk4::DragAction::MOVE,
        );
        drop_target.connect_drop(move |_, value, _, _| {
            let paths_str = match value.get::<String>() {
                Ok(s) => s,
                Err(_) => return false,
            };
            if let Some(source_id) = paths_str
                .strip_prefix("collection:")
                .and_then(|id| id.parse::<i64>().ok())
            {
                if source_id == id {
                    return false;
                }
                if let Some(widget) = widget_weak.upgrade() {
                    widget.emit_collection_reparent_requested(source_id, id);
                }
                return true;
            }
            let paths: Vec<std::path::PathBuf> = paths_str
                .lines()
                .filter(|l| !l.is_empty())
                .map(std::path::PathBuf::from)
                .collect();
            if paths.is_empty() {
                return false;
            }
            if let Some(widget) = widget_weak.upgrade() {
                widget.emit_drop_paths_to_collection(id, paths);
            }
            true
        });
        row.add_controller(drop_target);

        let menu = gio::Menu::new();
        let edit_section = gio::Menu::new();
        edit_section.append(Some("Rename\u{2026}"), Some("collection.rename"));
        edit_section.append(
            Some("Change Color\u{2026}"),
            Some("collection.change-color"),
        );
        menu.append_section(None, &edit_section);

        let move_section = gio::Menu::new();
        if row.has_parent() {
            move_section.append(Some("Promote to Top Level"), Some("collection.promote"));
        }
        move_section.append(
            Some("Move into Bucket\u{2026}"),
            Some("collection.move-into"),
        );
        menu.append_section(None, &move_section);

        let delete_section = gio::Menu::new();
        delete_section.append(Some("Delete Bucket"), Some("collection.delete"));
        menu.append_section(None, &delete_section);

        let popover = gtk4::PopoverMenu::from_model(Some(&menu));
        popover.set_has_arrow(true);
        popover.set_position(gtk4::PositionType::Right);
        popover.set_parent(row);

        let group = gio::SimpleActionGroup::new();

        let widget_weak = self.downgrade();
        let row_weak = row.downgrade();
        let rename = gio::SimpleAction::new("rename", None);
        rename.connect_activate(move |_, _| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            widget.emit_collection_rename_requested(row.collection_id());
        });
        group.add_action(&rename);

        let widget_weak = self.downgrade();
        let row_weak = row.downgrade();
        let change_color = gio::SimpleAction::new("change-color", None);
        change_color.connect_activate(move |_, _| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            widget.emit_collection_color_requested(row.collection_id());
        });
        group.add_action(&change_color);

        let widget_weak = self.downgrade();
        let row_weak = row.downgrade();
        let promote = gio::SimpleAction::new("promote", None);
        promote.connect_activate(move |_, _| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            widget.emit_collection_promote_requested(row.collection_id());
        });
        group.add_action(&promote);

        let widget_weak = self.downgrade();
        let row_weak = row.downgrade();
        let move_into = gio::SimpleAction::new("move-into", None);
        move_into.connect_activate(move |_, _| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            widget.emit_collection_move_requested(row.collection_id());
        });
        group.add_action(&move_into);

        let widget_weak = self.downgrade();
        let row_weak = row.downgrade();
        let delete = gio::SimpleAction::new("delete", None);
        delete.connect_activate(move |_, _| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            let id = row.collection_id();
            let name = row.collection_name();
            let body = format!(
                "\u{201c}{name}\u{201d} and any child buckets will be removed from the sidebar. The images themselves are not touched."
            );
            let dialog = libadwaita::AlertDialog::new(Some("Delete Bucket?"), Some(&body));
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("delete", "Delete");
            dialog.set_close_response("cancel");
            dialog.set_response_appearance("delete", libadwaita::ResponseAppearance::Destructive);
            let widget_weak2 = widget.downgrade();
            dialog.connect_response(None, move |_, response| {
                if response == "delete" {
                    let widget_weak3 = widget_weak2.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(w) = widget_weak3.upgrade() {
                            w.emit_collection_delete_requested(id);
                        }
                    });
                }
            });
            if let Some(root) = row.root() {
                if let Some(window) = root.downcast_ref::<gtk4::Window>() {
                    dialog.present(Some(window));
                } else {
                    dialog.present(None::<&gtk4::Window>);
                }
            }
        });
        group.add_action(&delete);

        row.insert_action_group("collection", Some(&group));

        // Right-click gesture to show popover
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(3);
        let popover_clone = popover.clone();
        gesture.connect_released(move |gesture, _, x, y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            popover_clone.set_pointing_to(Some(&rect));
            popover_clone.popup();
        });
        row.add_controller(gesture);
    }

    pub(crate) fn emit_collection_selected(&self, id: i64) {
        if let Some(cb) = self.imp().collection_selected_cb.borrow().as_ref() {
            cb(id);
        }
    }

    fn emit_collection_add_requested(&self) {
        if let Some(cb) = self.imp().collection_add_requested_cb.borrow().as_ref() {
            cb();
        }
    }

    fn emit_collection_rename_requested(&self, id: i64) {
        if let Some(cb) = self.imp().collection_rename_requested_cb.borrow().as_ref() {
            cb(id);
        }
    }

    fn emit_collection_color_requested(&self, id: i64) {
        if let Some(cb) = self.imp().collection_color_requested_cb.borrow().as_ref() {
            cb(id);
        }
    }

    fn emit_collection_promote_requested(&self, id: i64) {
        if let Some(cb) = self.imp().collection_promote_requested_cb.borrow().as_ref() {
            cb(id);
        }
    }

    fn emit_collection_move_requested(&self, id: i64) {
        if let Some(cb) = self.imp().collection_move_requested_cb.borrow().as_ref() {
            cb(id);
        }
    }

    fn emit_collection_reparent_requested(&self, source_id: i64, target_parent_id: i64) {
        if let Some(cb) = self
            .imp()
            .collection_reparent_requested_cb
            .borrow()
            .as_ref()
        {
            cb(source_id, target_parent_id);
        }
    }

    fn emit_collection_delete_requested(&self, id: i64) {
        if let Some(cb) = self.imp().collection_delete_requested_cb.borrow().as_ref() {
            cb(id);
        }
    }

    pub fn connect_drop_paths_to_collection<F: Fn(i64, Vec<std::path::PathBuf>) + 'static>(
        &self,
        f: F,
    ) {
        *self.imp().drop_paths_to_collection_cb.borrow_mut() = Some(Box::new(f));
    }

    fn emit_drop_paths_to_collection(&self, id: i64, paths: Vec<std::path::PathBuf>) {
        if let Some(cb) = self.imp().drop_paths_to_collection_cb.borrow().as_ref() {
            cb(id, paths);
        }
    }

    fn toggle_collection_collapsed(&self, id: i64) {
        let imp = self.imp();
        {
            let mut collapsed = imp.collapsed_collection_ids.borrow_mut();
            if !collapsed.insert(id) {
                collapsed.remove(&id);
            }
        }
        let collections = imp.collections.borrow().clone();
        self.refresh_collections(&collections);
    }

    fn attach_folder_row_menu(&self, row: &FolderRow) {
        if let Some(disclosure) = row.disclosure_button() {
            let row_weak = row.downgrade();
            let widget_weak = self.downgrade();
            disclosure.connect_clicked(move |_| {
                let Some(widget) = widget_weak.upgrade() else {
                    return;
                };
                let Some(row) = row_weak.upgrade() else {
                    return;
                };
                widget.toggle_folder_collapsed(row.path());
            });
        }

        let menu = gio::Menu::new();
        menu.append(Some("Disable Folder"), Some("folder.toggle-ignore"));

        let popover = gtk4::PopoverMenu::from_model(Some(&menu));
        popover.set_has_arrow(true);
        popover.set_position(gtk4::PositionType::Right);
        popover.set_parent(row);

        let group = gio::SimpleActionGroup::new();
        let widget_weak = self.downgrade();
        let row_weak = row.downgrade();
        let toggle_ignore = gio::SimpleAction::new("toggle-ignore", None);
        toggle_ignore.connect_activate(move |_, _| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            let ignored = !row.ignored();
            row.set_ignored(ignored);
            widget.emit_folder_ignored_changed(row.path(), ignored);
        });
        group.add_action(&toggle_ignore);
        row.insert_action_group("folder", Some(&group));

        let gesture = gtk4::GestureClick::new();
        gesture.set_button(0);
        gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let popover_clone = popover.clone();
        let row_weak = row.downgrade();
        let widget_weak = self.downgrade();
        gesture.connect_pressed(move |gesture, _, x, y| {
            let button = gesture.current_button();
            let modifiers = gesture.current_event_state();
            let ctrl_click =
                button == 1 && modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
            if button != 3 && !ctrl_click {
                return;
            }
            gesture.set_state(gtk4::EventSequenceState::Claimed);

            if let Some(row) = row_weak.upgrade() {
                toggle_ignore.set_enabled(true);
                // We update the menu item label dynamically
                menu.remove_all();
                menu.append(
                    Some(if row.ignored() {
                        "Enable Folder"
                    } else {
                        "Disable Folder"
                    }),
                    Some("folder.toggle-ignore"),
                );
                // A root can be forgotten while another remains (D41).
                let roots = widget_weak
                    .upgrade()
                    .map(|widget| widget.imp().library_roots.borrow().clone())
                    .unwrap_or_default();
                if roots.len() > 1 && roots.contains(&row.path()) {
                    let item = gio::MenuItem::new(Some("Remove from Library"), None);
                    item.set_action_and_target_value(
                        Some("win.remove-library-root"),
                        Some(&row.path().to_string_lossy().to_variant()),
                    );
                    menu.append_item(&item);
                }
            }

            let rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            popover_clone.set_pointing_to(Some(&rect));
            popover_clone.popup();
        });
        row.add_controller(gesture);
    }

    fn toggle_folder_collapsed(&self, path: PathBuf) {
        let imp = self.imp();
        let now_collapsed = {
            let mut collapsed = imp.collapsed_folder_paths.borrow_mut();
            if !collapsed.insert(path.clone()) {
                collapsed.remove(&path);
                false
            } else {
                true
            }
        };
        crate::bench_event!(
            "sidebar.folder_toggle",
            serde_json::json!({
                "path": path.display().to_string(),
                "collapsed": now_collapsed,
            }),
        );
        let tree = imp.folder_tree.borrow().clone();
        let ignored_folders = imp.ignored_folders.borrow().clone();
        self.show_folder_tree(&tree, &ignored_folders, false);
        self.select_folder(&path);
    }
}

fn set_bold_label_text(label: &gtk4::Label, text: &str) {
    let escaped = glib::markup_escape_text(text);
    label.set_markup(&format!("<b>{escaped}</b>"));
}

fn section_label(text: &str) -> gtk4::Label {
    let lbl = gtk4::Label::new(Some(text));
    lbl.add_css_class("caption-heading");
    lbl.set_halign(gtk4::Align::Start);
    lbl.set_margin_start(12);
    lbl.set_margin_top(8);
    lbl.set_margin_bottom(4);
    lbl
}

fn visible_collections(
    collections: &[Collection],
    collapsed_ids: &HashSet<i64>,
) -> Vec<Collection> {
    let by_parent = collection_children(collections);
    let mut out = Vec::new();
    append_visible_collections(None, &by_parent, collapsed_ids, &mut out);
    out
}

fn append_visible_collections(
    parent_id: Option<i64>,
    by_parent: &HashMap<Option<i64>, Vec<Collection>>,
    collapsed_ids: &HashSet<i64>,
    out: &mut Vec<Collection>,
) {
    if let Some(children) = by_parent.get(&parent_id) {
        for child in children {
            out.push(child.clone());
            if !collapsed_ids.contains(&child.id) {
                append_visible_collections(Some(child.id), by_parent, collapsed_ids, out);
            }
        }
    }
}

fn collection_children(collections: &[Collection]) -> HashMap<Option<i64>, Vec<Collection>> {
    let mut by_parent: HashMap<Option<i64>, Vec<Collection>> = HashMap::new();
    for collection in collections {
        by_parent
            .entry(collection.parent_id)
            .or_default()
            .push(collection.clone());
    }
    for children in by_parent.values_mut() {
        children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }
    by_parent
}

fn collection_depth(id: i64, collections: &[Collection]) -> u32 {
    let by_id: HashMap<i64, Option<i64>> =
        collections.iter().map(|c| (c.id, c.parent_id)).collect();
    let mut depth = 0u32;
    let mut current = by_id.get(&id).copied().flatten();
    while let Some(parent_id) = current {
        depth += 1;
        current = by_id.get(&parent_id).copied().flatten();
    }
    depth
}

fn root_collection_id(id: i64, collections: &[Collection]) -> i64 {
    let by_id: HashMap<i64, Option<i64>> =
        collections.iter().map(|c| (c.id, c.parent_id)).collect();
    let mut root_id = id;
    let mut current = by_id.get(&id).copied().flatten();
    while let Some(parent_id) = current {
        root_id = parent_id;
        current = by_id.get(&parent_id).copied().flatten();
    }
    root_id
}

fn collection_root_colors(collections: &[Collection]) -> HashMap<i64, String> {
    collections
        .iter()
        .filter(|collection| collection.parent_id.is_none())
        .map(|collection| {
            (
                collection.id,
                collection
                    .color
                    .clone()
                    .unwrap_or_else(|| fallback_collection_color(collection.id).to_string()),
            )
        })
        .collect()
}

fn fallback_collection_color(collection_id: i64) -> &'static str {
    use crate::palette::BUCKETS as PALETTE;
    PALETTE[(collection_id as usize) % PALETTE.len()]
}

fn apply_icon_color(image: &gtk4::Image, color: &str) {
    image.add_css_class(&super::look::color_class(color, false));
}

/// Walks the library root and reports each folder the sidebar should list: one
/// with images of its own, or a disabled one (shown dimmed, not entered). The
/// walk stays on the root's filesystem and skips hidden folders (`model::walk`),
/// so a root such as Home does not crawl network shares or `.cache`. Ancestors of
/// a reported folder are listed too; `folder_tree_from` adds them.
fn scan_image_folders(
    root: &Path,
    disabled: &[PathBuf],
    canceled: &dyn Fn() -> bool,
    found: &mut dyn FnMut(&Path),
) {
    let device = walk::device_of(root);
    scan_image_folders_below(root, device, disabled, canceled, found);
}

fn scan_image_folders_below(
    path: &Path,
    device: Option<u64>,
    disabled: &[PathBuf],
    canceled: &dyn Fn() -> bool,
    found: &mut dyn FnMut(&Path),
) {
    if canceled() {
        return;
    }
    if row_is_ignored(path, disabled) {
        found(path);
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    let mut has_images = false;
    for entry in entries.filter_map(Result::ok) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file() && is_image_file(&entry.path()) {
            has_images = true;
        } else if file_type.is_dir() && walk::may_descend_into(&entry, device) {
            scan_image_folders_below(&entry.path(), device, disabled, canceled, found);
        }
    }
    if has_images {
        found(path);
    }
}

/// The sidebar tree for the folders found so far: each one under `root`, with
/// the folders between it and the root, children in path order.
fn folder_tree_from(root: &Path, found: &HashSet<PathBuf>) -> Vec<FolderNode> {
    let mut children: HashMap<&Path, HashSet<&Path>> = HashMap::new();
    let mut listed: HashSet<&Path> = HashSet::new();
    for folder in found.iter().filter(|folder| folder.starts_with(root)) {
        let mut current = folder.as_path();
        while listed.insert(current) {
            if current == root {
                break;
            }
            let Some(parent) = current.parent() else {
                break;
            };
            children.entry(parent).or_default().insert(current);
            current = parent;
        }
    }
    if !listed.contains(root) {
        return Vec::new();
    }
    fn node(path: &Path, children: &HashMap<&Path, HashSet<&Path>>) -> FolderNode {
        let mut below: Vec<FolderNode> = children
            .get(path)
            .into_iter()
            .flatten()
            .map(|child| node(child, children))
            .collect();
        below.sort_by(|a, b| path_sort_key(&a.path, &b.path));
        FolderNode {
            path: path.to_path_buf(),
            children: below,
        }
    }
    vec![node(root, &children)]
}

/// Which rows of `new` are not in `old`, if `new` is `old` with rows added
/// (same order and depths), so the list can grow in place and keep its
/// selection and scroll position. `None` means the rows must be rebuilt.
fn rows_added(old: &[FolderListEntry], new: &[FolderListEntry]) -> Option<Vec<usize>> {
    let mut kept = old.iter().peekable();
    let mut added = Vec::new();
    for (index, entry) in new.iter().enumerate() {
        match kept.peek() {
            Some(row) if row.path == entry.path && row.depth == entry.depth => {
                kept.next();
            }
            _ => added.push(index),
        }
    }
    kept.peek().is_none().then_some(added)
}

fn visible_folder_entries(
    tree: &[FolderNode],
    collapsed_paths: &HashSet<PathBuf>,
) -> Vec<FolderListEntry> {
    let mut rows = Vec::new();
    append_visible_folder_nodes(tree, 0, collapsed_paths, &mut rows);
    rows
}

fn append_visible_folder_nodes(
    nodes: &[FolderNode],
    depth: u32,
    collapsed: &HashSet<PathBuf>,
    out: &mut Vec<FolderListEntry>,
) {
    for node in nodes {
        let has_children = !node.children.is_empty();
        out.push(FolderListEntry {
            label: folder_label(node.path.as_path()),
            path: node.path.clone(),
            depth,
            has_children,
        });
        if !collapsed.contains(&node.path) {
            append_visible_folder_nodes(&node.children, depth + 1, collapsed, out);
        }
    }
}

fn collect_folder_paths(nodes: &[FolderNode], out: &mut HashSet<PathBuf>) {
    for node in nodes {
        out.insert(node.path.clone());
        collect_folder_paths(&node.children, out);
    }
}

fn default_collapsed_folder_paths(tree: &[FolderNode]) -> HashSet<PathBuf> {
    let mut collapsed = HashSet::new();
    collect_default_collapsed_paths(tree, 0, &mut collapsed);
    collapsed
}

fn collect_default_collapsed_paths(
    nodes: &[FolderNode],
    depth: u32,
    collapsed: &mut HashSet<PathBuf>,
) {
    for node in nodes {
        if depth >= 1 {
            collapsed.insert(node.path.clone());
        }
        collect_default_collapsed_paths(&node.children, depth + 1, collapsed);
    }
}

fn preferred_initial_folder_path(
    tree: &[FolderNode],
    entries: &[FolderListEntry],
    ignored_folders: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(path) = tree
        .first()
        .filter(|root| tree.len() == 1 && !root.children.is_empty())
        .and_then(|root| {
            root.children
                .iter()
                .find(|child| !row_is_ignored(&child.path, ignored_folders))
                .map(|child| child.path.clone())
        })
    {
        return Some(path);
    }

    entries
        .iter()
        .find(|entry| !row_is_ignored(&entry.path, ignored_folders))
        .map(|entry| entry.path.clone())
}

fn folder_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

fn path_sort_key(a: &Path, b: &Path) -> std::cmp::Ordering {
    a.to_string_lossy()
        .to_lowercase()
        .cmp(&b.to_string_lossy().to_lowercase())
}

fn is_image_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.contains(".pending-") || name.contains(".ncnn-intermediate") {
        return false;
    }
    path.extension()
        .map(|ext| {
            let ext = ext.to_string_lossy().to_lowercase();
            IMAGE_EXTENSIONS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

fn row_is_ignored(path: &Path, ignored_folders: &[PathBuf]) -> bool {
    ignored_folders
        .iter()
        .any(|folder| path.starts_with(folder))
}

fn folder_row_activation_toggles_collapse(_has_children: bool) -> bool {
    false
}

fn folder_disclosure_icon_name(is_collapsed: bool) -> &'static str {
    if is_collapsed {
        "pan-end-symbolic"
    } else {
        "pan-down-symbolic"
    }
}

mod folder_row_imp {
    use super::*;

    #[derive(Default)]
    pub struct FolderRow {
        pub path: RefCell<PathBuf>,
        pub ignored: Cell<bool>,
        pub has_children: Cell<bool>,
        pub disclosure_button: RefCell<Option<gtk4::Button>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FolderRow {
        const NAME: &'static str = "SharprFolderRow";
        type Type = super::FolderRow;
        type ParentType = gtk4::ListBoxRow;
    }

    impl ObjectImpl for FolderRow {}
    impl WidgetImpl for FolderRow {}
    impl ListBoxRowImpl for FolderRow {}
}

glib::wrapper! {
    pub struct FolderRow(ObjectSubclass<folder_row_imp::FolderRow>)
        @extends gtk4::ListBoxRow, gtk4::Widget,
                 @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Actionable;
}

impl FolderRow {
    pub fn new(
        path: PathBuf,
        label: &str,
        depth: u32,
        has_children: bool,
        is_collapsed: bool,
        color: Option<&str>,
    ) -> Self {
        let row: Self = glib::Object::new();
        *row.imp().path.borrow_mut() = path;
        row.imp().has_children.set(has_children);

        let icon_name = folder_icon_name(&row.path());
        let icon = gtk4::Image::from_icon_name(icon_name);
        if let Some(color) = color {
            apply_icon_color(&icon, color);
        }
        let disclosure = gtk4::Button::new();
        disclosure.add_css_class("flat");
        disclosure.add_css_class("collection-disclosure");
        disclosure.set_icon_name(folder_disclosure_icon_name(is_collapsed));
        disclosure.set_sensitive(has_children);
        disclosure.set_opacity(if has_children { 1.0 } else { 0.0 });
        disclosure.set_tooltip_text(Some("Expand or collapse"));
        *row.imp().disclosure_button.borrow_mut() = Some(disclosure.clone());
        let name_label = gtk4::Label::new(Some(label));
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        hbox.set_margin_start(8 + (depth as i32 * 18));
        hbox.set_margin_end(8);
        hbox.set_margin_top(3);
        hbox.set_margin_bottom(3);
        hbox.append(&disclosure);
        hbox.append(&icon);
        hbox.append(&name_label);

        row.set_child(Some(&hbox));
        row
    }

    pub fn path(&self) -> PathBuf {
        self.imp().path.borrow().clone()
    }

    pub fn ignored(&self) -> bool {
        self.imp().ignored.get()
    }

    pub fn has_children(&self) -> bool {
        self.imp().has_children.get()
    }

    pub fn disclosure_button(&self) -> Option<gtk4::Button> {
        self.imp().disclosure_button.borrow().clone()
    }

    /// A folder that gains its first subfolder (or loses its last) gets its
    /// disclosure arrow without the row being rebuilt.
    pub fn set_has_children(&self, has_children: bool, is_collapsed: bool) {
        if self.imp().has_children.replace(has_children) == has_children {
            return;
        }
        if let Some(disclosure) = self.disclosure_button() {
            disclosure.set_icon_name(folder_disclosure_icon_name(is_collapsed));
            disclosure.set_sensitive(has_children);
            disclosure.set_opacity(if has_children { 1.0 } else { 0.0 });
        }
    }

    pub fn set_ignored(&self, ignored: bool) {
        self.imp().ignored.set(ignored);
        self.set_opacity(if ignored { 0.45 } else { 1.0 });
        self.set_tooltip_text(if ignored {
            Some("Folder disabled")
        } else {
            None
        });
    }
}

/// The folder-icon colour of the root `path` lies under (D41), by root order.
fn root_color(roots: &[PathBuf], path: &Path) -> Option<&'static str> {
    let index = roots.iter().position(|root| path.starts_with(root))?;
    Some(crate::palette::ROOTS[index % crate::palette::ROOTS.len()])
}

fn folder_icon_name(path: &Path) -> &'static str {
    let s = path.to_string_lossy();
    if s.contains("/Pictures") || s.contains("/photos") || s.contains("/Photos") {
        "folder-pictures-symbolic"
    } else if s.contains("/Downloads") || s.contains("/downloads") {
        "folder-download-symbolic"
    } else if s.contains("/Videos") || s.contains("/videos") {
        "folder-videos-symbolic"
    } else if s.contains("/Music") || s.contains("/music") {
        "folder-music-symbolic"
    } else if s.contains("/Documents") || s.contains("/documents") {
        "folder-documents-symbolic"
    } else {
        "folder-symbolic"
    }
}

// ---------------------------------------------------------------------------
// CollectionRow GObject
// ---------------------------------------------------------------------------

mod collection_row_imp {
    use super::*;

    #[derive(Default)]
    pub struct CollectionRow {
        pub collection_id: Cell<i64>,
        pub collection_name: RefCell<String>,
        pub item_count: Cell<usize>,
        pub depth: Cell<u32>,
        pub has_children: Cell<bool>,
        pub disclosure_button: RefCell<Option<gtk4::Button>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CollectionRow {
        const NAME: &'static str = "SharprCollectionRow";
        type Type = super::CollectionRow;
        type ParentType = gtk4::ListBoxRow;
    }

    impl ObjectImpl for CollectionRow {}
    impl WidgetImpl for CollectionRow {}
    impl ListBoxRowImpl for CollectionRow {}
}

glib::wrapper! {
    pub struct CollectionRow(ObjectSubclass<collection_row_imp::CollectionRow>)
        @extends gtk4::ListBoxRow, gtk4::Widget,
                 @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Actionable;
}

impl CollectionRow {
    pub fn new(
        collection: &Collection,
        effective_color: &str,
        depth: u32,
        has_children: bool,
        is_collapsed: bool,
    ) -> Self {
        let row: Self = glib::Object::new();
        row.imp().collection_id.set(collection.id);
        *row.imp().collection_name.borrow_mut() = collection.name.clone();
        row.imp().item_count.set(collection.item_count);
        row.imp().depth.set(depth);
        row.imp().has_children.set(has_children);

        let disclosure = gtk4::Button::new();
        disclosure.add_css_class("flat");
        disclosure.add_css_class("collection-disclosure");
        disclosure.set_icon_name(if is_collapsed {
            "pan-end-symbolic"
        } else {
            "pan-down-symbolic"
        });
        disclosure.set_sensitive(has_children);
        disclosure.set_opacity(if has_children { 1.0 } else { 0.0 });
        disclosure.set_tooltip_text(Some("Expand or collapse"));
        *row.imp().disclosure_button.borrow_mut() = Some(disclosure.clone());

        // Keep collections on the same symbolic icon coloring path as folders.
        // The previous background-painted badge could regress into a broken icon.
        let badge = gtk4::Image::from_icon_name("sharpr-bucket-symbolic");
        badge.set_pixel_size(15);
        badge.set_halign(gtk4::Align::Center);
        badge.set_valign(gtk4::Align::Center);
        apply_icon_color(&badge, effective_color);
        badge.set_opacity(if depth == 0 { 1.0 } else { 0.78 });

        let name_label = gtk4::Label::new(Some(&collection.name));
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let mut count_buf = itoa::Buffer::new();
        let count_label = gtk4::Label::new(Some(count_buf.format(collection.item_count)));
        count_label.add_css_class("dim-label");
        count_label.add_css_class("caption");
        count_label.set_tooltip_text(Some(&format!(
            "{} image{}",
            collection.item_count,
            if collection.item_count == 1 { "" } else { "s" }
        )));

        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        hbox.set_margin_start(8 + (depth as i32 * 18));
        hbox.set_margin_end(8);
        hbox.set_margin_top(3);
        hbox.set_margin_bottom(3);
        hbox.append(&disclosure);
        hbox.append(&badge);
        hbox.append(&name_label);
        hbox.append(&count_label);
        if depth > 0 {
            row.add_css_class("collection-child-row");
        } else {
            row.add_css_class("collection-root-row");
        }

        row.set_child(Some(&hbox));
        row
    }

    pub fn collection_id(&self) -> i64 {
        self.imp().collection_id.get()
    }

    pub fn collection_name(&self) -> String {
        self.imp().collection_name.borrow().clone()
    }

    pub fn disclosure_button(&self) -> Option<gtk4::Button> {
        self.imp().disclosure_button.borrow().clone()
    }

    pub fn has_children(&self) -> bool {
        self.imp().has_children.get()
    }

    pub fn has_parent(&self) -> bool {
        self.imp().depth.get() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(path: &str, children: Vec<FolderNode>) -> FolderNode {
        FolderNode {
            path: PathBuf::from(path),
            children,
        }
    }

    #[test]
    fn drilldown_defaults_to_root_expanded_with_children_collapsed() {
        let tree = vec![node(
            "/Pictures",
            vec![
                node(
                    "/Pictures/Art",
                    vec![node("/Pictures/Art/Sketches", vec![])],
                ),
                node("/Pictures/Backup", vec![]),
            ],
        )];

        let collapsed = default_collapsed_folder_paths(&tree);

        assert!(!collapsed.contains(&PathBuf::from("/Pictures")));
        assert!(collapsed.contains(&PathBuf::from("/Pictures/Art")));
        assert!(collapsed.contains(&PathBuf::from("/Pictures/Art/Sketches")));
        assert!(collapsed.contains(&PathBuf::from("/Pictures/Backup")));
    }

    #[test]
    fn drilldown_prefers_first_direct_child_for_initial_selection() {
        let tree = vec![node(
            "/Pictures",
            vec![
                node(
                    "/Pictures/Art",
                    vec![node("/Pictures/Art/Sketches", vec![])],
                ),
                node("/Pictures/Backup", vec![]),
            ],
        )];
        let entries = visible_folder_entries(&tree, &default_collapsed_folder_paths(&tree));

        let selected = preferred_initial_folder_path(&tree, &entries, &[]).unwrap();

        assert_eq!(selected, PathBuf::from("/Pictures/Art"));
    }

    #[test]
    fn initial_selection_skips_ignored_direct_children() {
        let tree = vec![node(
            "/Pictures",
            vec![
                node("/Pictures/Art", vec![]),
                node("/Pictures/Backup", vec![]),
            ],
        )];
        let entries = visible_folder_entries(&tree, &default_collapsed_folder_paths(&tree));
        let ignored = vec![PathBuf::from("/Pictures/Art")];

        let selected = preferred_initial_folder_path(&tree, &entries, &ignored).unwrap();

        assert_eq!(selected, PathBuf::from("/Pictures/Backup"));
    }

    fn scratch_folder(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("sharpr-sidebar-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    fn scanned(root: &Path, disabled: &[PathBuf]) -> HashSet<PathBuf> {
        let mut found = HashSet::new();
        scan_image_folders(root, disabled, &|| false, &mut |path| {
            found.insert(path.to_path_buf());
        });
        found
    }

    #[test]
    fn scan_lists_nested_image_folders_and_skips_hidden_ones() {
        let root = scratch_folder("scan");
        touch(&root.join("Pictures/a.jpg"));
        touch(&root.join("Pictures/Trips/Rome/b.png"));
        touch(&root.join("Documents/notes.txt"));
        touch(&root.join(".cache/thumbs/c.jpg"));
        touch(&root.join("Pictures/.thumbnails/d.jpg"));

        let found = scanned(&root, &[]);

        assert_eq!(
            found,
            HashSet::from([root.join("Pictures"), root.join("Pictures/Trips/Rome")])
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scan_lists_a_disabled_folder_without_entering_it() {
        let root = scratch_folder("disabled");
        touch(&root.join("Art/a.jpg"));
        touch(&root.join("Art/Old/b.jpg"));

        let found = scanned(&root, &[root.join("Art/Old")]);

        assert_eq!(
            found,
            HashSet::from([root.join("Art"), root.join("Art/Old")])
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scan_stops_when_canceled() {
        let root = scratch_folder("canceled");
        touch(&root.join("Art/a.jpg"));
        let mut found = HashSet::new();
        scan_image_folders(&root, &[], &|| true, &mut |path| {
            found.insert(path.to_path_buf());
        });
        assert!(found.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tree_from_found_folders_adds_the_folders_between() {
        let root = PathBuf::from("/home/u");
        let found = HashSet::from([
            PathBuf::from("/home/u/Pictures/Trips/Rome"),
            PathBuf::from("/home/u/Art"),
            PathBuf::from("/home/u/Pictures"),
            PathBuf::from("/elsewhere/Stray"),
        ]);

        let tree = folder_tree_from(&root, &found);

        let entries = visible_folder_entries(&tree, &HashSet::new());
        let listed: Vec<_> = entries.iter().map(|e| (e.path.clone(), e.depth)).collect();
        assert_eq!(
            listed,
            vec![
                (PathBuf::from("/home/u"), 0),
                (PathBuf::from("/home/u/Art"), 1),
                (PathBuf::from("/home/u/Pictures"), 1),
                (PathBuf::from("/home/u/Pictures/Trips"), 2),
                (PathBuf::from("/home/u/Pictures/Trips/Rome"), 3),
            ]
        );
        assert!(folder_tree_from(&root, &HashSet::new()).is_empty());
        assert!(folder_tree_from(&root, &HashSet::from([PathBuf::from("/other/x")])).is_empty());
    }

    #[test]
    fn rows_added_recognises_a_list_that_only_grew() {
        let row = |path: &str, depth: u32| FolderListEntry {
            path: PathBuf::from(path),
            label: String::new(),
            depth,
            has_children: false,
        };
        let old = [row("/r", 0), row("/r/b", 1)];

        let grown = [row("/r", 0), row("/r/a", 1), row("/r/b", 1), row("/r/c", 1)];
        assert_eq!(rows_added(&old, &grown), Some(vec![1, 3]));
        assert_eq!(rows_added(&[], &grown), Some(vec![0, 1, 2, 3]));
        assert_eq!(rows_added(&old, &old), Some(vec![]));

        let shrunk = [row("/r", 0)];
        assert_eq!(rows_added(&old, &shrunk), None, "a row went away");
        let moved = [row("/r", 0), row("/r/b", 2)];
        assert_eq!(rows_added(&old, &moved), None, "a row changed depth");
    }

    #[test]
    fn selecting_folder_with_children_does_not_toggle_collapse() {
        assert!(!folder_row_activation_toggles_collapse(true));
        assert!(!folder_row_activation_toggles_collapse(false));
    }

    #[test]
    fn disclosure_toggle_keeps_expand_and_collapse_icons() {
        assert_eq!(folder_disclosure_icon_name(true), "pan-end-symbolic");
        assert_eq!(folder_disclosure_icon_name(false), "pan-down-symbolic");
    }
}
