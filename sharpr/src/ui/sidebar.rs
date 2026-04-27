use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Once;

use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;

use crate::library_index::Collection;
use crate::ui::window::AppState;

type FolderSelectedCallback = Box<dyn Fn(PathBuf) + 'static>;
type FolderIgnoredChangedCallback = Box<dyn Fn(PathBuf, bool) + 'static>;
type CollectionSelectedCallback = Box<dyn Fn(i64) + 'static>;
type CollectionAddRequestedCallback = Box<dyn Fn() + 'static>;
type CollectionChildAddRequestedCallback = Box<dyn Fn(i64) + 'static>;
type CollectionEditRequestedCallback = Box<dyn Fn(i64) + 'static>;
type CollectionMoveRequestedCallback = Box<dyn Fn(i64) + 'static>;
type CollectionReparentRequestedCallback = Box<dyn Fn(i64, i64) + 'static>;
type CollectionDeleteRequestedCallback = Box<dyn Fn(i64) + 'static>;
type DropPathsToCollectionCallback = Box<dyn Fn(i64, Vec<std::path::PathBuf>) + 'static>;
type TagPromotedToCollectionCallback = Box<dyn Fn(String) + 'static>;

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "tiff", "tif", "bmp", "ico", "avif", "heic", "heif",
];

mod imp {
    use super::*;

    pub struct SidebarPane {
        pub toolbar_view: libadwaita::ToolbarView,
        pub list_box: gtk4::ListBox,
        pub collection_list: gtk4::ListBox,
        pub folder_selected_cb: RefCell<Option<FolderSelectedCallback>>,
        pub folder_ignored_changed_cb: RefCell<Option<FolderIgnoredChangedCallback>>,
        pub collection_selected_cb: RefCell<Option<CollectionSelectedCallback>>,
        pub collection_add_requested_cb: RefCell<Option<CollectionAddRequestedCallback>>,
        pub collection_child_add_requested_cb: RefCell<Option<CollectionChildAddRequestedCallback>>,
        pub collection_edit_requested_cb: RefCell<Option<CollectionEditRequestedCallback>>,
        pub collection_move_requested_cb: RefCell<Option<CollectionMoveRequestedCallback>>,
        pub collection_reparent_requested_cb: RefCell<Option<CollectionReparentRequestedCallback>>,
        pub collection_delete_requested_cb: RefCell<Option<CollectionDeleteRequestedCallback>>,
        pub drop_paths_to_collection_cb: RefCell<Option<DropPathsToCollectionCallback>>,
        pub tag_promoted_to_collection_cb: RefCell<Option<TagPromotedToCollectionCallback>>,
        pub suppress_folder_signal: Cell<bool>,
        pub suppress_collection_signal: Cell<bool>,
        pub collapsed_collection_ids: RefCell<HashSet<i64>>,
        pub collections: RefCell<Vec<Collection>>,
    }

    impl Default for SidebarPane {
        fn default() -> Self {
            Self {
                toolbar_view: libadwaita::ToolbarView::new(),
                list_box: gtk4::ListBox::new(),
                collection_list: gtk4::ListBox::new(),
                folder_selected_cb: RefCell::new(None),
                folder_ignored_changed_cb: RefCell::new(None),
                collection_selected_cb: RefCell::new(None),
                collection_add_requested_cb: RefCell::new(None),
                collection_child_add_requested_cb: RefCell::new(None),
                collection_edit_requested_cb: RefCell::new(None),
                collection_move_requested_cb: RefCell::new(None),
                collection_reparent_requested_cb: RefCell::new(None),
                collection_delete_requested_cb: RefCell::new(None),
                drop_paths_to_collection_cb: RefCell::new(None),
                tag_promoted_to_collection_cb: RefCell::new(None),
                suppress_folder_signal: Cell::new(false),
                suppress_collection_signal: Cell::new(false),
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
        @extends gtk4::Widget;
}

impl SidebarPane {
    pub fn new(state: Rc<RefCell<AppState>>) -> Self {
        install_collection_css();
        let widget: Self = glib::Object::new();
        widget.build_ui(state);
        widget
    }

    fn build_ui(&self, state: Rc<RefCell<AppState>>) {
        let imp = self.imp();

        let header = libadwaita::HeaderBar::new();
        header.set_show_end_title_buttons(false);

        let open_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
        open_btn.set_tooltip_text(Some("Open Folder"));
        header.pack_start(&open_btn);

        let widget_weak = self.downgrade();
        open_btn.connect_clicked(move |btn| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(root) = btn.root() else { return };
            let Some(window) = root.downcast_ref::<gtk4::Window>() else {
                return;
            };

            let chooser = gtk4::FileDialog::new();
            chooser.set_title("Open Image Folder");

            let widget_weak2 = widget.downgrade();
            let window_clone = window.clone();
            chooser.select_folder(
                Some(&window_clone),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            if let Some(widget) = widget_weak2.upgrade() {
                                widget.select_folder(&path);
                                widget.emit_folder_selected(path);
                            }
                        }
                    }
                },
            );
        });

        imp.toolbar_view.add_top_bar(&header);

        imp.list_box.add_css_class("navigation-sidebar");
        imp.list_box.set_selection_mode(gtk4::SelectionMode::Single);
        let (library_root, ignored_folders) = {
            let state = state.borrow();
            (
                state.settings.library_root.clone(),
                state.disabled_folders.clone(),
            )
        };
        self.populate_default_folders(library_root, &ignored_folders);

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
            widget.clear_collection_selection();
            widget.emit_folder_selected(folder_row.path());
        });

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&imp.list_box));

        let folders_label = section_label("Folders");

        // Collections section header: label + "New Collection" + button
        let collections_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        let collections_label = section_label("Collections");
        collections_label.set_hexpand(true);
        let new_collection_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        new_collection_btn.add_css_class("flat");
        new_collection_btn.set_tooltip_text(Some("New Collection"));
        new_collection_btn.set_valign(gtk4::Align::Center);
        new_collection_btn.set_margin_end(8);
        collections_header.append(&collections_label);
        collections_header.append(&new_collection_btn);

        let drop_target = gtk4::DropTarget::new(glib::Type::STRING, gdk4::DragAction::COPY);
        let widget_weak = self.downgrade();
        drop_target.connect_drop(move |_, value, _, _| {
            let Ok(s) = value.get::<String>() else {
                return false;
            };
            if let Some(tag) = s.strip_prefix("tag:") {
                if let Some(widget) = widget_weak.upgrade() {
                    widget.emit_tag_promoted_to_collection(tag.to_string());
                }
                return true;
            }
            false
        });
        collections_header.add_controller(drop_target);

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
        vbox.append(&folders_label);
        vbox.append(&scroll);
        vbox.append(&collections_header);
        vbox.append(&imp.collection_list);

        imp.toolbar_view.set_content(Some(&vbox));
        imp.toolbar_view.set_parent(self);
    }

    fn populate_default_folders(&self, library_root: Option<PathBuf>, ignored_folders: &[PathBuf]) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home".into());
        let home = PathBuf::from(&home);

        // If the user configured a custom library root, scan only that path.
        // Otherwise fall back to the default trio.
        let entries: Vec<(PathBuf, String)> = if let Some(root) = library_root {
            let name = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Library".to_string());
            vec![(root, name)]
        } else {
            vec![
                (home.join("Pictures"), "Pictures".into()),
                (home.join("Downloads"), "Downloads".into()),
                (home.clone(), "Home".into()),
            ]
        };

        let mut seen = HashSet::new();

        for (root_path, root_name) in entries {
            if !root_path.is_dir() {
                continue;
            }

            let mut child_folders = discover_image_child_folders(&root_path, &root_name);
            child_folders.sort_by(|(_, a), (_, b)| a.to_lowercase().cmp(&b.to_lowercase()));

            if directory_contains_images(&root_path) && !seen.contains(&root_path) {
                seen.insert(root_path.clone());
                let row = FolderRow::new(root_path, &root_name);
                row.set_ignored(row_is_ignored(&row.path(), ignored_folders));
                self.attach_folder_row_menu(&row);
                self.imp().list_box.append(&row);
            }

            for (path, label) in child_folders {
                if !seen.contains(&path) {
                    seen.insert(path.clone());
                    let row = FolderRow::new(path, &label);
                    row.set_ignored(row_is_ignored(&row.path(), ignored_folders));
                    self.attach_folder_row_menu(&row);
                    self.imp().list_box.append(&row);
                }
            }
        }
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
    pub fn first_folder_path(&self) -> Option<PathBuf> {
        self.imp()
            .list_box
            .first_child()?
            .downcast::<FolderRow>()
            .ok()
            .map(|row| row.path())
    }

    pub fn select_folder(&self, path: &Path) {
        let mut child = self.imp().list_box.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Ok(row) = widget.downcast::<FolderRow>() {
                if row.path() == path {
                    self.imp().suppress_folder_signal.set(true);
                    self.imp().list_box.select_row(Some(&row));
                    self.imp().suppress_folder_signal.set(false);
                    break;
                }
            }
            child = next;
        }
    }

    fn clear_folder_selection(&self) {
        self.imp().suppress_folder_signal.set(true);
        self.imp().list_box.unselect_all();
        self.imp().suppress_folder_signal.set(false);
    }

    fn emit_folder_selected(&self, path: PathBuf) {
        if let Some(cb) = self.imp().folder_selected_cb.borrow().as_ref() {
            cb(path);
        }
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

    pub fn connect_collection_child_add_requested<F: Fn(i64) + 'static>(&self, f: F) {
        *self.imp().collection_child_add_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_collection_edit_requested<F: Fn(i64) + 'static>(&self, f: F) {
        *self.imp().collection_edit_requested_cb.borrow_mut() = Some(Box::new(f));
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

    pub fn connect_tag_promoted_to_collection<F: Fn(String) + 'static>(&self, f: F) {
        *self.imp().tag_promoted_to_collection_cb.borrow_mut() = Some(Box::new(f));
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
        let selected_id = imp
            .collection_list
            .selected_row()
            .and_then(|row| row.downcast::<CollectionRow>().ok())
            .map(|row| row.collection_id());
        imp.suppress_collection_signal.set(true);
        while let Some(child) = imp.collection_list.first_child() {
            imp.collection_list.remove(&child);
        }
        for coll in visible_collections(collections, &imp.collapsed_collection_ids.borrow()) {
            let has_children = collections.iter().any(|c| c.parent_id == Some(coll.id));
            let is_collapsed = imp.collapsed_collection_ids.borrow().contains(&coll.id);
            let row = CollectionRow::new(
                &coll,
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

        let popover = gtk4::Popover::new();
        popover.set_autohide(true);
        popover.set_has_arrow(true);
        popover.set_position(gtk4::PositionType::Right);

        let btn_child = gtk4::Button::with_label("New Child Collection");
        btn_child.add_css_class("flat");
        let btn_edit = gtk4::Button::with_label("Edit Collection");
        btn_edit.add_css_class("flat");
        let btn_move = gtk4::Button::with_label("Assign As Child Of…");
        btn_move.add_css_class("flat");
        btn_move.set_visible(!row.has_children());
        let btn_delete = gtk4::Button::with_label("Delete Collection");
        btn_delete.add_css_class("flat");
        btn_delete.add_css_class("destructive-action");

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        vbox.set_margin_top(4);
        vbox.set_margin_bottom(4);
        vbox.append(&btn_child);
        vbox.append(&btn_edit);
        vbox.append(&btn_move);
        vbox.append(&btn_delete);
        popover.set_child(Some(&vbox));
        popover.set_parent(row);

        let widget_weak = self.downgrade();
        let row_weak = row.downgrade();
        let popover_clone = popover.clone();
        btn_child.connect_clicked(move |_| {
            popover_clone.popdown();
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            widget.emit_collection_child_add_requested(row.collection_id());
        });

        let widget_weak = self.downgrade();
        let row_weak = row.downgrade();
        let popover_clone = popover.clone();
        btn_edit.connect_clicked(move |_| {
            popover_clone.popdown();
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            widget.emit_collection_edit_requested(row.collection_id());
        });

        let widget_weak = self.downgrade();
        let row_weak = row.downgrade();
        let popover_clone = popover.clone();
        btn_move.connect_clicked(move |_| {
            popover_clone.popdown();
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            widget.emit_collection_move_requested(row.collection_id());
        });

        // Delete button
        let widget_weak = self.downgrade();
        let row_weak = row.downgrade();
        let popover_clone = popover.clone();
        btn_delete.connect_clicked(move |_| {
            popover_clone.popdown();
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            let id = row.collection_id();
            let name = row.collection_name();
            let body = format!(
                "\u{201c}{name}\u{201d} and any child collections will be removed from the sidebar. Image tags are left unchanged."
            );
            let dialog = libadwaita::AlertDialog::new(Some("Delete Collection?"), Some(&body));
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("delete", "Delete");
            dialog.set_close_response("cancel");
            dialog.set_response_appearance("delete", libadwaita::ResponseAppearance::Destructive);
            let widget_weak2 = widget.downgrade();
            dialog.connect_response(None, move |_, response| {
                if response == "delete" {
                    if let Some(w) = widget_weak2.upgrade() {
                        w.emit_collection_delete_requested(id);
                    }
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

    fn emit_collection_selected(&self, id: i64) {
        if let Some(cb) = self.imp().collection_selected_cb.borrow().as_ref() {
            cb(id);
        }
    }

    fn emit_collection_add_requested(&self) {
        if let Some(cb) = self.imp().collection_add_requested_cb.borrow().as_ref() {
            cb();
        }
    }

    fn emit_collection_child_add_requested(&self, id: i64) {
        if let Some(cb) = self
            .imp()
            .collection_child_add_requested_cb
            .borrow()
            .as_ref()
        {
            cb(id);
        }
    }

    fn emit_collection_edit_requested(&self, id: i64) {
        if let Some(cb) = self.imp().collection_edit_requested_cb.borrow().as_ref() {
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

    fn emit_tag_promoted_to_collection(&self, tag: String) {
        if let Some(cb) = self.imp().tag_promoted_to_collection_cb.borrow().as_ref() {
            cb(tag);
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
        let popover = gtk4::Popover::new();
        popover.set_autohide(true);
        popover.set_has_arrow(true);
        popover.set_position(gtk4::PositionType::Right);

        let toggle_btn = gtk4::Button::new();
        toggle_btn.add_css_class("flat");
        let row_weak = row.downgrade();
        toggle_btn.connect_clicked({
            let widget_weak = self.downgrade();
            let popover = popover.clone();
            move |_| {
                popover.popdown();
                let Some(widget) = widget_weak.upgrade() else {
                    return;
                };
                let Some(row) = row_weak.upgrade() else {
                    return;
                };
                let ignored = !row.ignored();
                row.set_ignored(ignored);
                widget.emit_folder_ignored_changed(row.path(), ignored);
            }
        });

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        vbox.set_margin_top(4);
        vbox.set_margin_bottom(4);
        vbox.append(&toggle_btn);
        popover.set_child(Some(&vbox));
        popover.set_parent(row);

        let gesture = gtk4::GestureClick::new();
        gesture.set_button(0);
        gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let popover_clone = popover.clone();
        let row_weak = row.downgrade();
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
                toggle_btn.set_label(if row.ignored() {
                    "Enable Folder"
                } else {
                    "Disable Folder"
                });
            }
            let rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            popover_clone.set_pointing_to(Some(&rect));
            popover_clone.popup();
        });
        row.add_controller(gesture);
    }
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

fn install_collection_css() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(
            "
            .collection-child-row {
                opacity: 0.94;
            }
            .collection-root-row {
                opacity: 1.0;
            }
            .collection-disclosure {
                min-width: 20px;
                min-height: 20px;
                padding: 0;
            }
            .collection-root-icon {
                color: @accent_color;
            }
            .collection-child-icon {
                color: alpha(@accent_color, 0.72);
            }
            ",
        );
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
}

fn discover_image_child_folders(root: &Path, root_name: &str) -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut folders = Vec::new();

    for entry in entries.filter_map(Result::ok) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let path = entry.path();
        if !directory_contains_images(&path) {
            continue;
        }

        let child_name = entry.file_name().to_string_lossy().into_owned();
        folders.push((path, format!("{root_name} / {child_name}")));
    }

    folders
}

fn directory_contains_images(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };

    entries.filter_map(Result::ok).any(|entry| {
        entry.file_type().map(|t| t.is_file()).unwrap_or(false) && is_image_file(&entry.path())
    })
}

fn is_image_file(path: &Path) -> bool {
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

mod folder_row_imp {
    use super::*;

    #[derive(Default)]
    pub struct FolderRow {
        pub path: RefCell<PathBuf>,
        pub ignored: Cell<bool>,
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
        @extends gtk4::ListBoxRow, gtk4::Widget;
}

impl FolderRow {
    pub fn new(path: PathBuf, label: &str) -> Self {
        let row: Self = glib::Object::new();
        *row.imp().path.borrow_mut() = path;

        let icon = gtk4::Image::from_icon_name("folder-symbolic");
        let name_label = gtk4::Label::new(Some(label));
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_hexpand(true);

        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);
        hbox.set_margin_top(6);
        hbox.set_margin_bottom(6);
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
        @extends gtk4::ListBoxRow, gtk4::Widget;
}

impl CollectionRow {
    pub fn new(
        collection: &Collection,
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
        *row.imp().disclosure_button.borrow_mut() = Some(disclosure.clone());

        let icon = gtk4::Image::from_icon_name(if depth == 0 {
            "tag-symbolic"
        } else {
            "tag-new-symbolic"
        });
        icon.add_css_class(if depth == 0 {
            "collection-root-icon"
        } else {
            "collection-child-icon"
        });
        let name_label = gtk4::Label::new(Some(&collection.name));
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let mut count_buf = itoa::Buffer::new();
        let count_label = gtk4::Label::new(Some(count_buf.format(collection.item_count)));
        count_label.add_css_class("dim-label");
        count_label.add_css_class("caption");
        count_label.set_tooltip_text(Some(&format!(
            "{} tag-matched image{}",
            collection.item_count,
            if collection.item_count == 1 { "" } else { "s" }
        )));

        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        hbox.set_margin_start(8 + (depth as i32 * 18));
        hbox.set_margin_end(8);
        hbox.set_margin_top(6);
        hbox.set_margin_bottom(6);
        hbox.append(&disclosure);
        hbox.append(&icon);
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
}
