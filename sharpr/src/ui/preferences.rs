use std::path::PathBuf;
use std::rc::Rc;

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::config::{AppSettings, FolderMode, LibraryConfig};
use crate::ui::window::SharprWindow;

pub fn build_preferences_window(
    settings: &AppSettings,
    parent: &SharprWindow,
) -> libadwaita::PreferencesWindow {
    let window = libadwaita::PreferencesWindow::new();
    window.set_title(Some("Preferences"));
    window.set_transient_for(Some(parent.upcast_ref::<gtk4::Window>()));
    window.set_modal(true);

    let library_page = libadwaita::PreferencesPage::new();
    library_page.set_title("Library");
    library_page.set_icon_name(Some("folder-symbolic"));

    let library_group = libadwaita::PreferencesGroup::new();
    library_group.set_title("Libraries");

    let library_group_rc = Rc::new(library_group.clone());
    for library in &settings.libraries {
        let row = build_library_row(library, parent, library_group_rc.clone());
        library_group.add(&row);
    }

    let add_library_row = libadwaita::ActionRow::new();
    add_library_row.set_title("Add Library");
    add_library_row.set_subtitle("Create another library root and folder mode");
    let add_button = gtk4::Button::with_label("Create…");
    add_library_row.add_suffix(&add_button);
    add_library_row.set_activatable_widget(Some(&add_button));
    {
        let parent_c = parent.clone();
        let group_c = library_group_rc.clone();
        add_button.connect_clicked(move |_| {
            present_library_editor(None, None, &parent_c, group_c.clone());
        });
    }
    library_group.add(&add_library_row);
    library_page.add(&library_group);

    let collections_group = libadwaita::PreferencesGroup::new();
    collections_group.set_title("Collections");

    let coll_export_row = libadwaita::ActionRow::new();
    coll_export_row.set_title("Export Collections");
    coll_export_row.set_subtitle("Save collection hierarchy and image assignments to a JSON file");
    let coll_export_button = gtk4::Button::with_label("Export…");
    coll_export_row.add_suffix(&coll_export_button);
    coll_export_row.set_activatable_widget(Some(&coll_export_button));
    {
        let parent_c = parent.clone();
        coll_export_button.connect_clicked(move |_| {
            parent_c.handle_collection_export_requested();
        });
    }

    let coll_import_row = libadwaita::ActionRow::new();
    coll_import_row.set_title("Import Collections");
    coll_import_row
        .set_subtitle("Restore from a previously exported file; existing collections are kept");
    let coll_import_button = gtk4::Button::with_label("Import…");
    coll_import_row.add_suffix(&coll_import_button);
    coll_import_row.set_activatable_widget(Some(&coll_import_button));
    {
        let parent_c = parent.clone();
        coll_import_button.connect_clicked(move |_| {
            parent_c.handle_collection_import_requested();
        });
    }

    collections_group.add(&coll_export_row);
    collections_group.add(&coll_import_row);

    // Fingerprints let buckets follow images moved or renamed outside Sharpr.
    let fingerprint_row = libadwaita::ActionRow::new();
    fingerprint_row.set_title("Image Fingerprints");
    fingerprint_row.set_subtitle(&fingerprint_status(parent));
    collections_group.add(&fingerprint_row);
    {
        let row = fingerprint_row.downgrade();
        let parent = parent.downgrade();
        gtk4::glib::timeout_add_seconds_local(1, move || {
            let (Some(row), Some(parent)) = (row.upgrade(), parent.upgrade()) else {
                return gtk4::glib::ControlFlow::Break;
            };
            row.set_subtitle(&fingerprint_status(&parent));
            gtk4::glib::ControlFlow::Continue
        });
    }
    library_page.add(&collections_group);
    library_page.add(&build_visual_search_group(parent));

    window.add(&library_page);

    let appearance_page = libadwaita::PreferencesPage::new();
    appearance_page.set_title("Appearance");
    appearance_page.set_icon_name(Some("preferences-desktop-appearance-symbolic"));

    let appearance_group = libadwaita::PreferencesGroup::new();
    appearance_group.set_title("Viewer");

    let metadata_row = libadwaita::SwitchRow::new();
    metadata_row.set_title("Show metadata overlay");
    metadata_row.set_subtitle("EXIF data shown in the bottom-right corner");
    metadata_row.set_active(
        action_state_bool(parent, "show-metadata").unwrap_or(settings.metadata_visible),
    );

    {
        let parent_c = parent.clone();
        metadata_row.connect_active_notify(move |row| {
            let desired = row.is_active();
            if action_state_bool(&parent_c, "show-metadata") != Some(desired) {
                gtk4::prelude::ActionGroupExt::activate_action(&parent_c, "show-metadata", None);
            }
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_metadata_visible(desired);
        });
    }

    let cache_row = libadwaita::SpinRow::with_range(100.0, 2000.0, 100.0);
    cache_row.set_title("Thumbnail cache size");
    cache_row.set_subtitle("Maximum images held in memory");
    cache_row.set_value(settings.thumbnail_cache_max as f64);

    {
        let parent_c = parent.clone();
        cache_row.connect_notify_local(Some("value"), move |row, _| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_thumbnail_cache_max(row.value() as i32);
        });
    }

    appearance_group.add(&metadata_row);
    appearance_group.add(&cache_row);
    appearance_page.add(&appearance_group);
    window.add(&appearance_page);

    window
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

fn fingerprint_status(window: &SharprWindow) -> String {
    match window.fingerprint_progress() {
        None => "Starts shortly after launch, in the background".into(),
        Some(p) if p.done < p.total => format!("Fingerprinting {} of {} images…", p.done, p.total),
        Some(p) => format!("All {} images have fingerprints", p.total),
    }
}

fn build_library_row(
    library: &LibraryConfig,
    parent: &SharprWindow,
    group: Rc<libadwaita::PreferencesGroup>,
) -> libadwaita::ActionRow {
    let row = libadwaita::ActionRow::new();
    row.set_title(&library.name);
    row.set_subtitle(&library_subtitle(library));
    let edit_button = gtk4::Button::with_label("Edit…");
    row.add_suffix(&edit_button);
    row.set_activatable_widget(Some(&edit_button));
    let library_id = library.id.clone();
    let parent_c = parent.clone();
    let row_c = row.clone();
    edit_button.connect_clicked(move |_| {
        present_library_editor(
            Some(library_id.clone()),
            Some(row_c.clone()),
            &parent_c,
            group.clone(),
        );
    });
    row
}

fn present_library_editor(
    library_id: Option<String>,
    row: Option<libadwaita::ActionRow>,
    parent: &SharprWindow,
    group: Rc<libadwaita::PreferencesGroup>,
) {
    let window = parent.clone().upcast::<gtk4::Window>();
    let existing = library_id.as_ref().and_then(|id| {
        parent
            .app_state()
            .borrow()
            .settings
            .libraries
            .iter()
            .find(|library| library.id == *id)
            .cloned()
    });

    let dialog = libadwaita::AlertDialog::new(
        Some(if existing.is_some() {
            "Edit Library"
        } else {
            "Create Library"
        }),
        None,
    );
    dialog.add_response("save", if existing.is_some() { "Save" } else { "Create" });
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("save", libadwaita::ResponseAppearance::Suggested);

    let name_entry = gtk4::Entry::new();
    name_entry.set_placeholder_text(Some("Library name"));
    if let Some(library) = existing.as_ref() {
        name_entry.set_text(&library.name);
    }

    let root_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let root_entry = gtk4::Entry::new();
    root_entry.set_hexpand(true);
    root_entry.set_editable(false);
    if let Some(library) = existing.as_ref() {
        root_entry.set_text(&library.root.to_string_lossy());
    }
    let choose_button = gtk4::Button::with_label("Choose…");
    root_row.append(&root_entry);
    root_row.append(&choose_button);

    let top_level = gtk4::CheckButton::with_label("Top level only");
    let drill_down = gtk4::CheckButton::with_label("Drill into subfolders");
    drill_down.set_group(Some(&top_level));
    match existing
        .as_ref()
        .map(|library| library.folder_mode)
        .unwrap_or(FolderMode::TopLevel)
    {
        FolderMode::TopLevel => top_level.set_active(true),
        FolderMode::DrillDown => drill_down.set_active(true),
    }

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    box_.set_margin_top(6);
    box_.append(&name_entry);
    box_.append(&root_row);
    box_.append(&top_level);
    box_.append(&drill_down);
    dialog.set_extra_child(Some(&box_));

    {
        let root_entry_c = root_entry.clone();
        let parent_window = window.clone();
        choose_button.connect_clicked(move |_| {
            let chooser = gtk4::FileDialog::new();
            chooser.set_title("Choose Library Root");
            let root_entry_inner = root_entry_c.clone();
            chooser.select_folder(
                Some(&parent_window),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            root_entry_inner.set_text(&path.to_string_lossy());
                        }
                    }
                },
            );
        });
    }

    let pref_window = group
        .root()
        .and_then(|r| r.downcast::<gtk4::Window>().ok())
        .unwrap_or_else(|| window.clone());

    let parent_c = parent.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }
        let folder_mode = if drill_down.is_active() {
            FolderMode::DrillDown
        } else {
            FolderMode::TopLevel
        };
        let root = PathBuf::from(root_entry.text().as_str());
        let name = name_entry.text().to_string();
        let (result, updated_settings) = {
            let mut settings = parent_c.app_state().borrow().settings.clone();
            if let Some(id) = library_id.as_deref() {
                let result = settings.update_library(id, &name, root, folder_mode);
                (result, settings)
            } else {
                let result = settings.add_library(&name, root, folder_mode).map(|_| ());
                (result, settings)
            }
        };

        match result {
            Ok(()) => {
                let libraries = updated_settings.libraries.clone();
                let disabled_folders = updated_settings
                    .active_library()
                    .map(|library| library.ignored_folders.clone())
                    .unwrap_or_default();
                {
                    let app_state_ref = parent_c.app_state();
                    let mut state = app_state_ref.borrow_mut();
                    state.settings = updated_settings;
                    state.disabled_folders = disabled_folders;
                }
                if let Some(row) = row.as_ref() {
                    if let Some(library) = libraries
                        .iter()
                        .find(|library| library_id.as_deref() == Some(library.id.as_str()))
                    {
                        row.set_title(&library.name);
                        row.set_subtitle(&library_subtitle(library));
                    }
                } else if let Some(library) = libraries.last() {
                    let row = build_library_row(library, &parent_c, group.clone());
                    if let Some(add_row) = group.last_child() {
                        group.remove(&add_row);
                        group.add(&row);
                        group.add(&add_row);
                    } else {
                        group.add(&row);
                    }
                }
            }
            Err(err) => {
                let error =
                    libadwaita::AlertDialog::new(Some("Could not save library"), Some(&err));
                error.add_response("ok", "OK");
                error.present(Some(parent_c.upcast_ref::<gtk4::Window>()));
            }
        }
    });

    dialog.present(Some(&pref_window));
}

fn library_subtitle(library: &LibraryConfig) -> String {
    format!(
        "{}  •  {}",
        library.root.to_string_lossy(),
        match library.folder_mode {
            FolderMode::TopLevel => "Top level only",
            FolderMode::DrillDown => "Drill into subfolders",
        }
    )
}

fn action_state_bool(window: &SharprWindow, action_name: &str) -> Option<bool> {
    window
        .lookup_action(action_name)
        .and_then(|action| action.state())
        .and_then(|state| state.get::<bool>())
}
