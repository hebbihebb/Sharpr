use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::ui::window::SharprWindow;
use gtk4::subclass::prelude::ObjectSubclassIsExt;

pub fn build_preferences_dialog(parent: &SharprWindow) -> libadwaita::PreferencesDialog {
    let dialog = libadwaita::PreferencesDialog::new();
    dialog.set_title("Preferences");
    dialog.add_css_class("sharpr-sheet");

    let library_page = libadwaita::PreferencesPage::new();
    library_page.set_title("Library");
    library_page.set_icon_name(Some("folder-symbolic"));

    let library_group = libadwaita::PreferencesGroup::new();
    library_group.set_title("Library Roots");
    library_group.set_description(Some(
        "Browse these folders and all their subfolders. Turning a root off or \
         removing it never touches its files.",
    ));
    let add = gtk4::Button::with_label("Add…");
    add.add_css_class("flat");
    add.set_valign(gtk4::Align::Center);
    add.set_tooltip_text(Some("Add another library root"));
    add.set_action_name(Some("win.add-library-root"));
    library_group.set_header_suffix(Some(&add));
    library_page.add(&library_group);
    // Retain the controls while a root's on/off state changes, including
    // keyboard focus on its switch. Only a changed root list needs rebuilding.
    let rows: Rc<RefCell<Vec<LibraryRootRow>>> = Rc::default();
    let shown: Rc<RefCell<Option<RootsShown>>> = Rc::default();
    let refresh = {
        let group = library_group.downgrade();
        let parent = parent.downgrade();
        move || {
            let (Some(group), Some(parent)) = (group.upgrade(), parent.upgrade()) else {
                return false;
            };
            let now = roots_shown(&parent);
            let same_roots = shown.borrow().as_ref().is_some_and(|previous| {
                previous.len() == now.len() && previous.iter().zip(&now).all(|(a, b)| a.0 == b.0)
            });
            if same_roots {
                for (row, (_, enabled)) in rows.borrow().iter().zip(&now) {
                    row.syncing.set(true);
                    row.switch.set_active(*enabled);
                    row.syncing.set(false);
                }
                *shown.borrow_mut() = Some(now);
                return true;
            }
            for row in rows.borrow_mut().drain(..) {
                group.remove(&row.row);
            }
            for (index, (root, enabled)) in now.iter().enumerate() {
                let row = root_row(&parent, root, *enabled, index == 0, now.len() > 1);
                group.add(&row.row);
                rows.borrow_mut().push(row);
            }
            *shown.borrow_mut() = Some(now);
            true
        }
    };
    refresh();
    gtk4::glib::timeout_add_seconds_local(1, move || {
        if refresh() {
            gtk4::glib::ControlFlow::Continue
        } else {
            gtk4::glib::ControlFlow::Break
        }
    });

    library_page.add(&build_visual_search_group(parent));
    library_page.add(&build_edit_buckets_group(parent));

    dialog.add(&library_page);

    dialog
}

/// Indexing progress, and where to install the model (D23).
fn build_visual_search_group(parent: &SharprWindow) -> libadwaita::PreferencesGroup {
    let group = libadwaita::PreferencesGroup::new();
    group.set_title("Visual Search");
    group.set_description(Some(
        "Find images by describing them. Indexing runs in the background at the lowest \
         priority; Main Menu → Library → Library Overview shows its progress.",
    ));

    let gpu_row = libadwaita::SwitchRow::new();
    gpu_row.set_title("Use Graphics Card");
    gpu_row.set_subtitle(&gpu_subtitle());
    gpu_row.set_active(parent.app_state().borrow().settings.visual_search_gpu);
    {
        let parent = parent.clone();
        gpu_row.connect_active_notify(move |row| {
            parent.set_visual_search_gpu(row.is_active());
        });
    }
    group.add(&gpu_row);

    group
}

/// Copies and their originals collected under Edits (D35).
fn build_edit_buckets_group(parent: &SharprWindow) -> libadwaita::PreferencesGroup {
    let group = libadwaita::PreferencesGroup::new();
    group.set_title("Edits");
    let row = libadwaita::SwitchRow::new();
    row.set_title("Collect Edits in Buckets");
    row.set_subtitle(
        "Crop Copy, export and Duplicate add the original and the copy to \
         Edits › Cropped, Exported or Duplicated, ready to compare",
    );
    row.set_active(parent.app_state().borrow().settings.edit_buckets);
    {
        let parent = parent.clone();
        row.connect_active_notify(move |row| {
            parent
                .app_state()
                .borrow_mut()
                .settings
                .set_edit_buckets(row.is_active());
        });
    }
    group.add(&row);
    group
}

/// Whether a GPU build is installed; which card it picks shows while
/// indexing runs.
fn gpu_subtitle() -> String {
    let installed = crate::visual_search::files::locate()
        .ok()
        .and_then(|files| files.gpu_server)
        .is_some();
    if installed {
        "Indexes about 17 times faster than the processor, using less energy per image. \
         Falls back to the processor when no card has room."
            .into()
    } else {
        "No GPU build of llama-server in the model folder's runtime-gpu; indexing uses the processor"
            .into()
    }
}

/// Each root of the active library, first first, and whether it is on.
type RootsShown = Vec<(PathBuf, bool)>;

struct LibraryRootRow {
    row: libadwaita::ActionRow,
    switch: gtk4::Switch,
    /// Updating a switch from settings must not write those settings again.
    syncing: Rc<Cell<bool>>,
}

fn roots_shown(parent: &SharprWindow) -> RootsShown {
    let state = parent.app_state();
    let state = state.borrow();
    state
        .settings
        .library_roots()
        .into_iter()
        .map(|root| {
            let off = state.disabled_folders.iter().any(|d| root.starts_with(d));
            (root, !off)
        })
        .collect()
}

/// A root's row: its name and path, a switch that turns it off and on as
/// "Disable Folder" in the sidebar does, "Choose…" on the first root and
/// "Remove" while another root remains (D41).
fn root_row(
    parent: &SharprWindow,
    root: &Path,
    enabled: bool,
    first: bool,
    removable: bool,
) -> LibraryRootRow {
    let row = libadwaita::ActionRow::new();
    row.set_title(
        &root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string()),
    );
    row.set_subtitle(&root.display().to_string());
    if first {
        let choose = gtk4::Button::with_label("Choose…");
        choose.add_css_class("flat");
        choose.set_valign(gtk4::Align::Center);
        choose.set_tooltip_text(Some("Change the first library root"));
        choose.set_action_name(Some("win.choose-library-root"));
        row.add_suffix(&choose);
    }
    if removable {
        let remove = gtk4::Button::with_label("Remove");
        remove.add_css_class("flat");
        remove.set_valign(gtk4::Align::Center);
        remove.set_tooltip_text(Some("Forget this root; its files are untouched"));
        remove.set_action_name(Some("win.remove-library-root"));
        remove.set_action_target_value(Some(&root.to_string_lossy().to_variant()));
        row.add_suffix(&remove);
    }
    let switch = gtk4::Switch::new();
    switch.set_active(enabled);
    switch.set_valign(gtk4::Align::Center);
    switch.set_tooltip_text(Some("Include this root in the library"));
    let parent = parent.downgrade();
    let root = root.to_path_buf();
    let syncing = Rc::new(Cell::new(false));
    let syncing_notify = syncing.clone();
    switch.connect_active_notify(move |switch| {
        if syncing_notify.get() {
            return;
        }
        let Some(parent) = parent.upgrade() else {
            return;
        };
        let sidebar = parent.imp().sidebar.borrow().clone();
        if let Some(sidebar) = sidebar {
            sidebar.change_folder_ignored(root.clone(), !switch.is_active());
        }
    });
    row.add_suffix(&switch);
    row.set_activatable_widget(Some(&switch));
    LibraryRootRow {
        row,
        switch,
        syncing,
    }
}
