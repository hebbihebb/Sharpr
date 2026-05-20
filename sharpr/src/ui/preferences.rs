use std::path::PathBuf;
use std::rc::Rc;

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::config::{AppSettings, FolderMode, LibraryConfig};
use crate::ui::window::SharprWindow;

fn is_local_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return true;
    }
    let stripped = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    stripped.starts_with("localhost")
        || stripped.starts_with("127.0.0.1")
        || stripped.starts_with("::1")
}

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
    add_library_row.set_subtitle("Create another library root and folder mode.");
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
    library_page.add(&collections_group);

    let smart_group = libadwaita::PreferencesGroup::new();
    smart_group.set_title("Smart Tagging");

    let smart_model_row = libadwaita::ComboRow::new();
    smart_model_row.set_title("Smart tagger model");
    let smart_models = [
        crate::tags::smart::SmartModel::Fast,
        crate::tags::smart::SmartModel::Balanced,
        crate::tags::smart::SmartModel::Best,
    ];
    let smart_model_labels: Vec<_> = smart_models
        .iter()
        .map(|model| model.display_name())
        .collect();
    let smart_model_choices = gtk4::StringList::new(&smart_model_labels);
    smart_model_row.set_model(Some(&smart_model_choices));
    let selected_model = crate::tags::smart::SmartModel::from_id(&settings.smart_tagger_model);
    let selected_idx = smart_models
        .iter()
        .position(|model| *model == selected_model)
        .unwrap_or(1);
    smart_model_row.set_selected(selected_idx as u32);

    {
        let parent_c = parent.clone();
        let available_smart_models = smart_models;
        smart_model_row.connect_selected_notify(move |row| {
            let model = available_smart_models
                .get(row.selected() as usize)
                .copied()
                .unwrap_or(crate::tags::smart::SmartModel::Balanced);
            let model_id = model.id();
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_smart_tagger_model(model_id);
            parent_c.reload_smart_tagger_model(model);
        });
    }
    smart_group.add(&smart_model_row);
    library_page.add(&smart_group);

    window.add(&library_page);

    let upscaler_page = libadwaita::PreferencesPage::new();
    upscaler_page.set_title("Upscaler");
    upscaler_page.set_icon_name(Some("image-x-generic-symbolic"));

    let upscaler_group = libadwaita::PreferencesGroup::new();
    upscaler_group.set_title("AI Upscale (Vulkan backend)");

    let binary_row = libadwaita::EntryRow::new();
    binary_row.set_title("Binary path");
    // EntryRow does not support set_subtitle in this version.
    binary_row.set_text(
        &settings
            .upscaler_binary_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );

    {
        let parent_c = parent.clone();
        binary_row.connect_changed(move |row| {
            let text = row.text().trim().to_string();
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_upscaler_binary_path(if text.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(text))
                });
        });
    }

    let gpu_row = libadwaita::SpinRow::with_range(-1.0, 16.0, 1.0);
    gpu_row.set_title("GPU ID");
    gpu_row.set_subtitle("-1 means auto");
    gpu_row.set_value(settings.upscaler_gpu_id as f64);

    {
        let parent_c = parent.clone();
        gpu_row.connect_notify_local(Some("value"), move |row, _| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_upscaler_gpu_id(row.value() as i32);
        });
    }

    upscaler_group.add(&binary_row);
    upscaler_group.add(&gpu_row);
    upscaler_page.add(&upscaler_group);

    let comfy_group = libadwaita::PreferencesGroup::new();
    comfy_group.set_title("ComfyUI (External Server)");

    let comfy_enabled_row = libadwaita::SwitchRow::new();
    comfy_enabled_row.set_title("Enable ComfyUI backend");
    comfy_enabled_row.set_subtitle("Requires an external ComfyUI server running with API access");
    comfy_enabled_row.set_active(settings.comfyui_enabled);

    {
        let parent_c = parent.clone();
        comfy_enabled_row.connect_active_notify(move |row| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_comfyui_enabled(row.is_active());
        });
    }

    let comfy_url_row = libadwaita::EntryRow::new();
    comfy_url_row.set_title("Server URL");
    comfy_url_row.set_text(&settings.comfyui_url);

    let privacy_banner =
        libadwaita::Banner::new("This URL is outside your machine — images will leave your device");
    privacy_banner.set_revealed(!is_local_url(&settings.comfyui_url));

    {
        let parent_c = parent.clone();
        let privacy_banner_c = privacy_banner.clone();
        comfy_url_row.connect_changed(move |row| {
            let text = row.text();
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_comfyui_url(text.as_str());
            privacy_banner_c.set_revealed(!is_local_url(text.as_str()));
        });
    }

    let comfy_workflow_row = libadwaita::ComboRow::new();
    comfy_workflow_row.set_title("Workflow preset");
    comfy_workflow_row.set_subtitle("Choose which bundled ComfyUI prompt Sharpr submits");
    let comfy_workflow_choices = gtk4::StringList::new(&["ESRGAN", "SeedVR2"]);
    comfy_workflow_row.set_model(Some(&comfy_workflow_choices));
    comfy_workflow_row.set_selected(if settings.comfyui_workflow == "seedvr2" {
        1
    } else {
        0
    });

    {
        let parent_c = parent.clone();
        comfy_workflow_row.connect_selected_notify(move |row| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_comfyui_workflow(if row.selected() == 1 {
                    "seedvr2"
                } else {
                    "esrgan"
                });
        });
    }

    let test_row = libadwaita::ActionRow::new();
    let test_button = gtk4::Button::with_label("Test Connection");
    test_row.add_suffix(&test_button);

    {
        let parent_c = parent.clone();
        let url_row_c = comfy_url_row.clone();
        test_button.connect_clicked(move |_| {
            let url = url_row_c.text().to_string();
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_comfyui_url(&url);
            let client = crate::upscale::backends::comfyui::ComfyUiClient::new(url);
            let parent_inner = parent_c.clone();

            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let result = client.health_check();
                let _ = tx.send_blocking(result);
            });

            glib::MainContext::default().spawn_local(async move {
                if let Ok(result) = rx.recv().await {
                    let body = match result {
                        Ok(_) => "ComfyUI is reachable!".to_string(),
                        Err(e) => e,
                    };

                    let toast = libadwaita::Toast::new(&body);
                    toast.set_timeout(3);
                    parent_inner.add_toast(toast);
                }
            });
        });
    }

    comfy_group.add(&comfy_enabled_row);
    comfy_group.add(&comfy_url_row);
    comfy_group.add(&comfy_workflow_row);
    comfy_group.add(&test_row);
    upscaler_page.add(&comfy_group);

    let privacy_group = libadwaita::PreferencesGroup::new();
    privacy_group.add(&privacy_banner);
    upscaler_page.add(&privacy_group);

    window.add(&upscaler_page);

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
    dialog.set_close_response("save");
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
