use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;

use crate::library_index::Collection;
use crate::quality::QualityClass;
use crate::ui::window::AppState;

type FolderSelectedCallback = Box<dyn Fn(PathBuf) + 'static>;
type DuplicatesSelectedCallback = Box<dyn Fn() + 'static>;
type TagsSelectedCallback = Box<dyn Fn() + 'static>;
type SearchActivatedCallback = Box<dyn Fn() + 'static>;
type QualitySelectedCallback = Box<dyn Fn(QualityClass) + 'static>;
type CollectionSelectedCallback = Box<dyn Fn(i64) + 'static>;
type CollectionAddRequestedCallback = Box<dyn Fn() + 'static>;
type CollectionRenameRequestedCallback = Box<dyn Fn(i64, String) + 'static>;
type CollectionDeleteRequestedCallback = Box<dyn Fn(i64) + 'static>;
type DropPathsToCollectionCallback = Box<dyn Fn(i64, Vec<std::path::PathBuf>) + 'static>;

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "tiff", "tif", "bmp", "ico", "avif", "heic", "heif",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum SmartFolderSelection {
    None,
    Duplicates,
    Tags,
    Search,
    Quality(QualityClass),
}

mod imp {
    use super::*;

    pub struct SidebarPane {
        pub toolbar_view: libadwaita::ToolbarView,
        pub list_box: gtk4::ListBox,
        pub smart_list: gtk4::ListBox,
        pub quality_list: gtk4::ListBox,
        pub collection_list: gtk4::ListBox,
        pub duplicates_row: gtk4::ListBoxRow,
        pub tags_row: gtk4::ListBoxRow,
        pub search_row: gtk4::ListBoxRow,
        pub excellent_row: gtk4::ListBoxRow,
        pub good_row: gtk4::ListBoxRow,
        pub fair_row: gtk4::ListBoxRow,
        pub poor_row: gtk4::ListBoxRow,
        pub needs_upscale_row: gtk4::ListBoxRow,
        pub folder_selected_cb: RefCell<Option<FolderSelectedCallback>>,
        pub duplicates_selected_cb: RefCell<Option<DuplicatesSelectedCallback>>,
        pub tags_selected_cb: RefCell<Option<TagsSelectedCallback>>,
        pub search_activated_cb: RefCell<Option<SearchActivatedCallback>>,
        pub quality_selected_cb: RefCell<Option<QualitySelectedCallback>>,
        pub collection_selected_cb: RefCell<Option<CollectionSelectedCallback>>,
        pub collection_add_requested_cb: RefCell<Option<CollectionAddRequestedCallback>>,
        pub collection_rename_requested_cb: RefCell<Option<CollectionRenameRequestedCallback>>,
        pub collection_delete_requested_cb: RefCell<Option<CollectionDeleteRequestedCallback>>,
        pub drop_paths_to_collection_cb: RefCell<Option<DropPathsToCollectionCallback>>,
        pub suppress_folder_signal: Cell<bool>,
        pub suppress_smart_signal: Cell<bool>,
        pub suppress_collection_signal: Cell<bool>,
    }

    impl Default for SidebarPane {
        fn default() -> Self {
            Self {
                toolbar_view: libadwaita::ToolbarView::new(),
                list_box: gtk4::ListBox::new(),
                smart_list: gtk4::ListBox::new(),
                quality_list: gtk4::ListBox::new(),
                collection_list: gtk4::ListBox::new(),
                duplicates_row: gtk4::ListBoxRow::new(),
                tags_row: gtk4::ListBoxRow::new(),
                search_row: gtk4::ListBoxRow::new(),
                excellent_row: gtk4::ListBoxRow::new(),
                good_row: gtk4::ListBoxRow::new(),
                fair_row: gtk4::ListBoxRow::new(),
                poor_row: gtk4::ListBoxRow::new(),
                needs_upscale_row: gtk4::ListBoxRow::new(),
                folder_selected_cb: RefCell::new(None),
                duplicates_selected_cb: RefCell::new(None),
                tags_selected_cb: RefCell::new(None),
                search_activated_cb: RefCell::new(None),
                quality_selected_cb: RefCell::new(None),
                collection_selected_cb: RefCell::new(None),
                collection_add_requested_cb: RefCell::new(None),
                collection_rename_requested_cb: RefCell::new(None),
                collection_delete_requested_cb: RefCell::new(None),
                drop_paths_to_collection_cb: RefCell::new(None),
                suppress_folder_signal: Cell::new(false),
                suppress_smart_signal: Cell::new(false),
                suppress_collection_signal: Cell::new(false),
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
        let library_root = state.borrow().settings.library_root.clone();
        self.populate_default_folders(library_root);

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
            widget.set_smart_selection(SmartFolderSelection::None);
            widget.clear_collection_selection();
            widget.emit_folder_selected(folder_row.path());
        });

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&imp.list_box));

        let smart_label = section_label("Smart Folders");
        let quality_label = section_label("Quality");
        let folders_label = section_label("Folders");

        imp.smart_list.add_css_class("navigation-sidebar");
        imp.smart_list
            .set_selection_mode(gtk4::SelectionMode::Single);
        imp.quality_list.add_css_class("navigation-sidebar");
        imp.quality_list
            .set_selection_mode(gtk4::SelectionMode::Single);

        configure_smart_row(&imp.duplicates_row, "edit-find-symbolic", "Duplicates");
        configure_smart_row(&imp.tags_row, "bookmark-new-symbolic", "Tags");
        configure_smart_row(&imp.search_row, "system-search-symbolic", "Search");
        configure_smart_row(
            &imp.excellent_row,
            "object-select-symbolic",
            QualityClass::Excellent.label(),
        );
        configure_smart_row(
            &imp.good_row,
            "starred-symbolic",
            QualityClass::Good.label(),
        );
        configure_smart_row(
            &imp.fair_row,
            "image-x-generic-symbolic",
            QualityClass::Fair.label(),
        );
        configure_smart_row(
            &imp.poor_row,
            "dialog-warning-symbolic",
            QualityClass::Poor.label(),
        );
        configure_smart_row(
            &imp.needs_upscale_row,
            "zoom-in-symbolic",
            QualityClass::NeedsUpscale.label(),
        );

        imp.smart_list.append(&imp.duplicates_row);
        imp.smart_list.append(&imp.tags_row);
        imp.smart_list.append(&imp.search_row);
        imp.quality_list.append(&imp.excellent_row);
        imp.quality_list.append(&imp.good_row);
        imp.quality_list.append(&imp.fair_row);
        imp.quality_list.append(&imp.poor_row);
        imp.quality_list.append(&imp.needs_upscale_row);

        let widget_weak = self.downgrade();
        imp.smart_list.connect_selected_rows_changed(move |list| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            if widget.imp().suppress_smart_signal.get() {
                return;
            }
            let Some(row) = list.selected_row() else {
                return;
            };
            widget.clear_quality_selection();
            widget.clear_folder_selection();
            widget.clear_collection_selection();
            if row == widget.imp().duplicates_row {
                widget.emit_duplicates_selected();
            } else if row == widget.imp().tags_row {
                widget.emit_tags_selected();
            } else if row == widget.imp().search_row {
                widget.emit_search_activated();
            }
        });

        let widget_weak = self.downgrade();
        imp.quality_list.connect_selected_rows_changed(move |list| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            if widget.imp().suppress_smart_signal.get() {
                return;
            }
            let Some(row) = list.selected_row() else {
                return;
            };
            widget.clear_smart_list_selection();
            widget.clear_folder_selection();
            widget.clear_collection_selection();
            let class = if row == widget.imp().excellent_row {
                QualityClass::Excellent
            } else if row == widget.imp().good_row {
                QualityClass::Good
            } else if row == widget.imp().fair_row {
                QualityClass::Fair
            } else if row == widget.imp().poor_row {
                QualityClass::Poor
            } else {
                QualityClass::NeedsUpscale
            };
            widget.emit_quality_selected(class);
        });

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
                widget.clear_smart_list_selection();
                widget.clear_quality_selection();
                widget.emit_collection_selected(coll_row.collection_id());
            });

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        vbox.append(&folders_label);
        vbox.append(&scroll);
        vbox.append(&smart_label);
        vbox.append(&imp.smart_list);
        vbox.append(&quality_label);
        vbox.append(&imp.quality_list);
        vbox.append(&collections_header);
        vbox.append(&imp.collection_list);

        imp.toolbar_view.set_content(Some(&vbox));
        imp.toolbar_view.set_parent(self);
    }

    fn populate_default_folders(&self, library_root: Option<PathBuf>) {
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

            if directory_contains_images(&root_path) && seen.insert(root_path.clone()) {
                let row = FolderRow::new(root_path.clone(), &root_name);
                self.imp().list_box.append(&row);
            }

            let mut child_folders = discover_image_child_folders(&root_path, &root_name);
            child_folders.sort_by(|(_, a), (_, b)| a.to_lowercase().cmp(&b.to_lowercase()));

            for (path, label) in child_folders {
                if seen.insert(path.clone()) {
                    let row = FolderRow::new(path, &label);
                    self.imp().list_box.append(&row);
                }
            }
        }
    }

    pub fn connect_folder_selected<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.imp().folder_selected_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_duplicates_selected<F: Fn() + 'static>(&self, f: F) {
        *self.imp().duplicates_selected_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_tags_selected<F: Fn() + 'static>(&self, f: F) {
        *self.imp().tags_selected_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_search_activated<F: Fn() + 'static>(&self, f: F) {
        *self.imp().search_activated_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_quality_selected<F: Fn(QualityClass) + 'static>(&self, f: F) {
        *self.imp().quality_selected_cb.borrow_mut() = Some(Box::new(f));
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

    pub fn set_duplicates_selected(&self, selected: bool) {
        if selected {
            self.set_smart_selection(SmartFolderSelection::Duplicates);
        } else if self.current_smart_selection() == SmartFolderSelection::Duplicates {
            self.set_smart_selection(SmartFolderSelection::None);
        }
    }

    pub fn set_search_selected(&self, selected: bool) {
        if selected {
            self.set_smart_selection(SmartFolderSelection::Search);
        } else if self.current_smart_selection() == SmartFolderSelection::Search {
            self.set_smart_selection(SmartFolderSelection::None);
        }
    }

    pub fn set_tags_selected(&self, selected: bool) {
        if selected {
            self.set_smart_selection(SmartFolderSelection::Tags);
        } else if self.current_smart_selection() == SmartFolderSelection::Tags {
            self.set_smart_selection(SmartFolderSelection::None);
        }
    }

    pub fn set_quality_selected(&self, class: Option<QualityClass>) {
        match class {
            Some(class) => self.set_smart_selection(SmartFolderSelection::Quality(class)),
            None => {
                if matches!(
                    self.current_smart_selection(),
                    SmartFolderSelection::Quality(_)
                ) {
                    self.set_smart_selection(SmartFolderSelection::None);
                }
            }
        }
    }

    pub fn select_folder(&self, path: &Path) {
        self.clear_smart_selection();

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

    fn clear_smart_selection(&self) {
        self.set_smart_selection(SmartFolderSelection::None);
    }

    fn clear_smart_list_selection(&self) {
        self.imp().suppress_smart_signal.set(true);
        self.imp().smart_list.unselect_all();
        self.imp().suppress_smart_signal.set(false);
    }

    fn clear_quality_selection(&self) {
        self.imp().suppress_smart_signal.set(true);
        self.imp().quality_list.unselect_all();
        self.imp().suppress_smart_signal.set(false);
    }

    fn set_smart_selection(&self, selection: SmartFolderSelection) {
        self.imp().suppress_smart_signal.set(true);
        match selection {
            SmartFolderSelection::None => {
                self.imp().smart_list.unselect_all();
                self.imp().quality_list.unselect_all();
            }
            SmartFolderSelection::Duplicates => {
                self.imp()
                    .smart_list
                    .select_row(Some(&self.imp().duplicates_row));
                self.imp().quality_list.unselect_all();
            }
            SmartFolderSelection::Tags => {
                self.imp().smart_list.select_row(Some(&self.imp().tags_row));
                self.imp().quality_list.unselect_all();
            }
            SmartFolderSelection::Search => {
                self.imp()
                    .smart_list
                    .select_row(Some(&self.imp().search_row));
                self.imp().quality_list.unselect_all();
            }
            SmartFolderSelection::Quality(class) => {
                self.imp().smart_list.unselect_all();
                let row = match class {
                    QualityClass::Excellent => &self.imp().excellent_row,
                    QualityClass::Good => &self.imp().good_row,
                    QualityClass::Fair => &self.imp().fair_row,
                    QualityClass::Poor => &self.imp().poor_row,
                    QualityClass::NeedsUpscale => &self.imp().needs_upscale_row,
                };
                self.imp().quality_list.select_row(Some(row));
            }
        }
        self.imp().suppress_smart_signal.set(false);
    }

    fn current_smart_selection(&self) -> SmartFolderSelection {
        if let Some(row) = self.imp().smart_list.selected_row() {
            if row == self.imp().duplicates_row {
                return SmartFolderSelection::Duplicates;
            }
            if row == self.imp().tags_row {
                return SmartFolderSelection::Tags;
            }
            if row == self.imp().search_row {
                return SmartFolderSelection::Search;
            }
        }
        if let Some(row) = self.imp().quality_list.selected_row() {
            if row == self.imp().excellent_row {
                return SmartFolderSelection::Quality(QualityClass::Excellent);
            }
            if row == self.imp().good_row {
                return SmartFolderSelection::Quality(QualityClass::Good);
            }
            if row == self.imp().fair_row {
                return SmartFolderSelection::Quality(QualityClass::Fair);
            }
            if row == self.imp().poor_row {
                return SmartFolderSelection::Quality(QualityClass::Poor);
            }
            if row == self.imp().needs_upscale_row {
                return SmartFolderSelection::Quality(QualityClass::NeedsUpscale);
            }
        }
        SmartFolderSelection::None
    }

    fn emit_folder_selected(&self, path: PathBuf) {
        if let Some(cb) = self.imp().folder_selected_cb.borrow().as_ref() {
            cb(path);
        }
    }

    fn emit_duplicates_selected(&self) {
        if let Some(cb) = self.imp().duplicates_selected_cb.borrow().as_ref() {
            cb();
        }
    }

    fn emit_tags_selected(&self) {
        if let Some(cb) = self.imp().tags_selected_cb.borrow().as_ref() {
            cb();
        }
    }

    fn emit_search_activated(&self) {
        if let Some(cb) = self.imp().search_activated_cb.borrow().as_ref() {
            cb();
        }
    }

    fn emit_quality_selected(&self, class: QualityClass) {
        if let Some(cb) = self.imp().quality_selected_cb.borrow().as_ref() {
            cb(class);
        }
    }

    // ---- Collection callbacks ----

    pub fn connect_collection_selected<F: Fn(i64) + 'static>(&self, f: F) {
        *self.imp().collection_selected_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_collection_add_requested<F: Fn() + 'static>(&self, f: F) {
        *self.imp().collection_add_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_collection_rename_requested<F: Fn(i64, String) + 'static>(&self, f: F) {
        *self.imp().collection_rename_requested_cb.borrow_mut() = Some(Box::new(f));
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
        imp.suppress_collection_signal.set(true);
        while let Some(child) = imp.collection_list.first_child() {
            imp.collection_list.remove(&child);
        }
        for coll in collections {
            let row = CollectionRow::new(coll.id, &coll.name, coll.item_count);
            self.attach_collection_row_menu(&row);
            imp.collection_list.append(&row);
        }
        imp.suppress_collection_signal.set(false);
    }

    fn attach_collection_row_menu(&self, row: &CollectionRow) {
        // Drop target: accept path strings dragged from the filmstrip.
        let id = row.collection_id();
        let widget_weak = self.downgrade();
        let drop_target =
            gtk4::DropTarget::new(glib::Type::STRING, gdk4::DragAction::COPY);
        drop_target.connect_drop(move |_, value, _, _| {
            let paths_str = match value.get::<String>() {
                Ok(s) => s,
                Err(_) => return false,
            };
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

        let btn_rename = gtk4::Button::with_label("Rename Collection");
        btn_rename.add_css_class("flat");
        let btn_delete = gtk4::Button::with_label("Delete Collection");
        btn_delete.add_css_class("flat");
        btn_delete.add_css_class("destructive-action");

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        vbox.set_margin_top(4);
        vbox.set_margin_bottom(4);
        vbox.append(&btn_rename);
        vbox.append(&btn_delete);
        popover.set_child(Some(&vbox));
        popover.set_parent(row);

        // Rename button
        let widget_weak = self.downgrade();
        let row_weak = row.downgrade();
        let popover_clone = popover.clone();
        btn_rename.connect_clicked(move |_| {
            popover_clone.popdown();
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            let id = row.collection_id();
            let current_name = row.collection_name();
            let dialog = libadwaita::AlertDialog::new(Some("Rename Collection"), None);
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("rename", "Rename");
            dialog.set_default_response(Some("rename"));
            dialog.set_close_response("cancel");
            dialog.set_response_appearance("rename", libadwaita::ResponseAppearance::Suggested);
            let entry = gtk4::Entry::new();
            entry.set_text(&current_name);
            entry.select_region(0, -1);
            let entry_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            entry_box.set_margin_top(6);
            entry_box.append(&entry);
            dialog.set_extra_child(Some(&entry_box));
            let entry_clone = entry.clone();
            let widget_weak2 = widget.downgrade();
            dialog.connect_response(None, move |_, response| {
                if response == "rename" {
                    let new_name = entry_clone.text().to_string();
                    if let Some(w) = widget_weak2.upgrade() {
                        w.emit_collection_rename_requested(id, new_name);
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
            let body = format!("\u{201c}{name}\u{201d} and all its image assignments will be removed. Images themselves are not deleted.");
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

    fn emit_collection_rename_requested(&self, id: i64, name: String) {
        if let Some(cb) = self.imp().collection_rename_requested_cb.borrow().as_ref() {
            cb(id, name);
        }
    }

    fn emit_collection_delete_requested(&self, id: i64) {
        if let Some(cb) = self.imp().collection_delete_requested_cb.borrow().as_ref() {
            cb(id);
        }
    }

    pub fn connect_drop_paths_to_collection<
        F: Fn(i64, Vec<std::path::PathBuf>) + 'static,
    >(
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
}

fn configure_smart_row(row: &gtk4::ListBoxRow, icon_name: &str, label_text: &str) {
    let icon = gtk4::Image::from_icon_name(icon_name);
    let label = gtk4::Label::new(Some(label_text));
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(true);

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(6);
    hbox.append(&icon);
    hbox.append(&label);
    row.set_child(Some(&hbox));
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

mod folder_row_imp {
    use super::*;

    #[derive(Default)]
    pub struct FolderRow {
        pub path: RefCell<PathBuf>,
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
    pub fn new(id: i64, name: &str, item_count: usize) -> Self {
        let row: Self = glib::Object::new();
        row.imp().collection_id.set(id);
        *row.imp().collection_name.borrow_mut() = name.to_string();
        row.imp().item_count.set(item_count);

        let icon = gtk4::Image::from_icon_name("folder-saved-search-symbolic");
        let name_label = gtk4::Label::new(Some(name));
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let count_label = gtk4::Label::new(Some(&item_count.to_string()));
        count_label.add_css_class("dim-label");
        count_label.add_css_class("caption");

        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);
        hbox.set_margin_top(6);
        hbox.set_margin_bottom(6);
        hbox.append(&icon);
        hbox.append(&name_label);
        hbox.append(&count_label);

        row.set_child(Some(&hbox));
        row
    }

    pub fn collection_id(&self) -> i64 {
        self.imp().collection_id.get()
    }

    pub fn collection_name(&self) -> String {
        self.imp().collection_name.borrow().clone()
    }
}
