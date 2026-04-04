use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::ui::window::AppState;

type FolderSelectedCallback = Box<dyn Fn(PathBuf) + 'static>;

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "tiff", "tif", "bmp", "ico", "avif", "heic", "heif",
];

// ---------------------------------------------------------------------------
// SidebarPane
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    pub struct SidebarPane {
        pub toolbar_view: libadwaita::ToolbarView,
        pub list_box: gtk4::ListBox,
        pub folder_selected_cb: RefCell<Option<FolderSelectedCallback>>,
    }

    impl Default for SidebarPane {
        fn default() -> Self {
            Self {
                toolbar_view: libadwaita::ToolbarView::new(),
                list_box: gtk4::ListBox::new(),
                folder_selected_cb: RefCell::new(None),
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

    fn build_ui(&self, _state: Rc<RefCell<AppState>>) {
        let imp = self.imp();

        // -----------------------------------------------------------------------
        // Header bar
        // -----------------------------------------------------------------------
        let header = libadwaita::HeaderBar::new();
        header.set_show_end_title_buttons(false);

        // "Open Folder" button — uses GtkFileDialog (GTK 4.10+, portal-friendly).
        let open_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
        open_btn.set_tooltip_text(Some("Open Folder"));
        header.pack_start(&open_btn);

        let widget_weak = self.downgrade();
        open_btn.connect_clicked(move |btn| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            // Walk up the widget tree to find the window.
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
                            if let Some(w) = widget_weak2.upgrade() {
                                w.emit_folder_selected(path);
                            }
                        }
                    }
                },
            );
        });

        imp.toolbar_view.add_top_bar(&header);

        // -----------------------------------------------------------------------
        // Folder list
        // -----------------------------------------------------------------------
        let list_box = &imp.list_box;
        list_box.add_css_class("navigation-sidebar");
        list_box.set_selection_mode(gtk4::SelectionMode::Single);

        self.populate_default_folders();

        // Single-click folder selection should behave the same way as the
        // explicit folder picker in the header.
        let widget_weak = self.downgrade();
        list_box.connect_selected_rows_changed(move |list_box| {
            let Some(row) = list_box.selected_row() else {
                return;
            };
            // `row` is a `&gtk4::ListBoxRow` but is actually our `FolderRow` subclass.
            if let Some(folder_row) = row.downcast_ref::<FolderRow>() {
                if let Some(w) = widget_weak.upgrade() {
                    w.emit_folder_selected(folder_row.path());
                }
            }
        });

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&imp.list_box));

        // Section headers.
        let folders_label = section_label("Folders");
        let tags_label = section_label("Tags");

        let recent_row = gtk4::Label::new(Some("Recent"));
        recent_row.set_halign(gtk4::Align::Start);
        recent_row.set_margin_start(24);
        recent_row.set_margin_bottom(4);
        recent_row.add_css_class("dim-label");

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        vbox.append(&folders_label);
        vbox.append(&scroll);
        vbox.append(&tags_label);
        vbox.append(&recent_row);

        imp.toolbar_view.set_content(Some(&vbox));
        imp.toolbar_view.set_parent(self);
    }

    fn populate_default_folders(&self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home".into());
        let home = PathBuf::from(&home);

        let entries = [
            (home.join("Pictures"), "Pictures"),
            (home.join("Downloads"), "Downloads"),
            (home.clone(), "Home"),
        ];

        let mut seen = HashSet::new();

        for (root_path, root_name) in entries {
            if !root_path.is_dir() {
                continue;
            }

            if directory_contains_images(&root_path) && seen.insert(root_path.clone()) {
                let row = FolderRow::new(root_path.clone(), root_name);
                self.imp().list_box.append(&row);
            }

            let mut child_folders = discover_image_child_folders(&root_path, root_name);
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

    fn emit_folder_selected(&self, path: PathBuf) {
        if let Some(cb) = self.imp().folder_selected_cb.borrow().as_ref() {
            cb(path);
        }
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

fn discover_image_child_folders(root: &std::path::Path, root_name: &str) -> Vec<(PathBuf, String)> {
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

fn directory_contains_images(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };

    entries.filter_map(Result::ok).any(|entry| {
        entry.file_type().map(|t| t.is_file()).unwrap_or(false) && is_image_file(&entry.path())
    })
}

fn is_image_file(path: &std::path::Path) -> bool {
    path.extension()
        .map(|ext| {
            let ext = ext.to_string_lossy().to_lowercase();
            IMAGE_EXTENSIONS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// FolderRow — ListBoxRow subclass that carries a PathBuf
// ---------------------------------------------------------------------------

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

        // Build widget content inline (managed by ListBoxRow::set_child).
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
