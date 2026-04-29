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
    add_library_row.set_subtitle("Create another library root and folder mode.");
    let add_button = gtk4::Button::with_label("Create…");
    add_library_row.add_suffix(&add_button);
    add_library_row.set_activatable_widget(Some(&add_button));
    {
        let parent_c = parent.clone();
        let group_c = library_group_rc.clone();
        add_button.connect_clicked(move |_| {
            present_library_editor(None, &parent_c, group_c.clone());
        });
    }
    library_group.add(&add_library_row);
    library_page.add(&library_group);

    let output_group = libadwaita::PreferencesGroup::new();
    output_group.set_title("Output folders");

    let upscaled_row = libadwaita::ActionRow::new();
    upscaled_row.set_title("Upscaled output folder");
    upscaled_row.set_subtitle(&output_folder_subtitle(
        settings.upscaled_output_dir.as_ref(),
        crate::export::OutputFolderKind::Upscaled,
    ));
    let upscaled_choose = gtk4::Button::with_label("Choose…");
    let upscaled_reset = gtk4::Button::with_label("Reset");
    upscaled_row.add_suffix(&upscaled_choose);
    upscaled_row.add_suffix(&upscaled_reset);
    upscaled_row.set_activatable_widget(Some(&upscaled_choose));

    {
        let parent_c = parent.clone();
        let row_c = upscaled_row.clone();
        let parent_window = parent.clone().upcast::<gtk4::Window>();
        upscaled_choose.connect_clicked(move |_| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Choose Upscaled Output Folder");
            let parent_inner = parent_c.clone();
            let row_inner = row_c.clone();
            dialog.select_folder(
                Some(&parent_window),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            parent_inner
                                .app_state()
                                .borrow_mut()
                                .settings
                                .set_upscaled_output_dir(Some(path.clone()));
                            row_inner.set_subtitle(&output_folder_subtitle(
                                Some(&path),
                                crate::export::OutputFolderKind::Upscaled,
                            ));
                        }
                    }
                },
            );
        });
    }

    {
        let parent_c = parent.clone();
        let row_c = upscaled_row.clone();
        upscaled_reset.connect_clicked(move |_| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_upscaled_output_dir(None);
            row_c.set_subtitle(&output_folder_subtitle(
                None,
                crate::export::OutputFolderKind::Upscaled,
            ));
        });
    }

    let export_row = libadwaita::ActionRow::new();
    export_row.set_title("Export output folder");
    export_row.set_subtitle(&output_folder_subtitle(
        settings.export_output_dir.as_ref(),
        crate::export::OutputFolderKind::Export,
    ));
    let export_choose = gtk4::Button::with_label("Choose…");
    let export_reset = gtk4::Button::with_label("Reset");
    export_row.add_suffix(&export_choose);
    export_row.add_suffix(&export_reset);
    export_row.set_activatable_widget(Some(&export_choose));

    {
        let parent_c = parent.clone();
        let row_c = export_row.clone();
        let parent_window = parent.clone().upcast::<gtk4::Window>();
        export_choose.connect_clicked(move |_| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Choose Export Output Folder");
            let parent_inner = parent_c.clone();
            let row_inner = row_c.clone();
            dialog.select_folder(
                Some(&parent_window),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            parent_inner
                                .app_state()
                                .borrow_mut()
                                .settings
                                .set_export_output_dir(Some(path.clone()));
                            row_inner.set_subtitle(&output_folder_subtitle(
                                Some(&path),
                                crate::export::OutputFolderKind::Export,
                            ));
                        }
                    }
                },
            );
        });
    }

    {
        let parent_c = parent.clone();
        let row_c = export_row.clone();
        export_reset.connect_clicked(move |_| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_export_output_dir(None);
            row_c.set_subtitle(&output_folder_subtitle(
                None,
                crate::export::OutputFolderKind::Export,
            ));
        });
    }

    output_group.add(&upscaled_row);
    output_group.add(&export_row);
    library_page.add(&output_group);

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

    let advanced_group = libadwaita::PreferencesGroup::new();
    advanced_group.set_title("Advanced");

    let show_upscale_row = libadwaita::SwitchRow::new();
    show_upscale_row.set_title("Show AI Upscale");
    show_upscale_row.set_subtitle("Expose AI upscaling in the main menu");
    show_upscale_row.set_active(settings.show_upscale_ui);

    {
        let parent_c = parent.clone();
        show_upscale_row.connect_active_notify(move |row| {
            let enabled = row.is_active();
            {
                parent_c
                    .app_state()
                    .borrow_mut()
                    .settings
                    .set_show_upscale_ui(enabled);
            }

            if let Some(action) = parent_c.lookup_action("upscale") {
                if let Ok(action) = action.downcast::<gio::SimpleAction>() {
                    action.set_enabled(enabled);
                }
            }
        });
    }

    advanced_group.add(&show_upscale_row);
    library_page.add(&advanced_group);

    window.add(&library_page);

    let upscaler_page = libadwaita::PreferencesPage::new();
    upscaler_page.set_title("Upscaler");
    upscaler_page.set_icon_name(Some("image-x-generic-symbolic"));

    let upscaler_group = libadwaita::PreferencesGroup::new();
    upscaler_group.set_title("AI Upscale (Vulkan backend)");

    let binary_row = libadwaita::ActionRow::new();
    binary_row.set_title("Binary path");
    binary_row.set_subtitle("Leave blank to auto-detect upscayl-bin or realesrgan-ncnn-vulkan");

    let binary_entry = gtk4::Entry::new();
    binary_entry.set_hexpand(true);
    binary_entry.set_width_chars(28);
    binary_entry.set_text(
        &settings
            .upscaler_binary_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    binary_row.add_suffix(&binary_entry);
    binary_row.set_activatable_widget(Some(&binary_entry));

    {
        let parent_c = parent.clone();
        binary_entry.connect_changed(move |entry| {
            let text = entry.text().trim().to_string();
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

    let model_row = libadwaita::ComboRow::new();
    model_row.set_title("Default model");
    let model_choices = gtk4::StringList::new(&["Standard (Photo)", "Anime / Art"]);
    model_row.set_model(Some(&model_choices));
    model_row.set_selected(if settings.upscaler_default_model == "anime" {
        1
    } else {
        0
    });

    {
        let parent_c = parent.clone();
        model_row.connect_selected_notify(move |row| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_upscaler_default_model(if row.selected() == 1 {
                    "anime"
                } else {
                    "standard"
                });
        });
    }

    let output_row = libadwaita::ComboRow::new();
    output_row.set_title("Output format");
    output_row.set_subtitle("Auto keeps Sharpr in charge of the final save format");
    let output_choices =
        gtk4::StringList::new(&["Auto", "JPEG (lossy)", "WebP (lossless)", "PNG (lossless)"]);
    output_row.set_model(Some(&output_choices));
    output_row.set_selected(match settings.upscaler_output_format.as_str() {
        "jpeg" => 1,
        "webp" => 2,
        "png" => 3,
        _ => 0,
    });

    {
        let parent_c = parent.clone();
        output_row.connect_selected_notify(move |row| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_upscaler_output_format(match row.selected() {
                    1 => "jpeg",
                    2 => "webp",
                    3 => "png",
                    _ => "auto",
                });
        });
    }

    let compression_row = libadwaita::ComboRow::new();
    compression_row.set_title("Compression");
    let compression_choices = gtk4::StringList::new(&["Auto", "Prefer lossy", "Prefer lossless"]);
    compression_row.set_model(Some(&compression_choices));
    compression_row.set_selected(match settings.upscaler_compression_mode.as_str() {
        "lossy" => 1,
        "lossless" => 2,
        _ => 0,
    });

    {
        let parent_c = parent.clone();
        compression_row.connect_selected_notify(move |row| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_upscaler_compression_mode(match row.selected() {
                    1 => "lossy",
                    2 => "lossless",
                    _ => "auto",
                });
        });
    }

    let quality_row = libadwaita::ActionRow::new();
    quality_row.set_title("Lossy quality");
    quality_row.set_subtitle("Used when Sharpr saves the final result as JPEG");
    let quality_adj =
        gtk4::Adjustment::new(settings.upscaler_quality as f64, 50.0, 100.0, 1.0, 5.0, 0.0);
    let quality_spin = gtk4::SpinButton::new(Some(&quality_adj), 1.0, 0);
    quality_spin.set_numeric(true);
    quality_row.add_suffix(&quality_spin);
    quality_row.set_activatable_widget(Some(&quality_spin));

    {
        let parent_c = parent.clone();
        quality_spin.connect_value_changed(move |spin| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_upscaler_quality(spin.value() as i32);
        });
    }

    let tile_row = libadwaita::ActionRow::new();
    tile_row.set_title("Tile size");
    tile_row.set_subtitle("0 means auto; raise only if the GPU has headroom");
    let tile_adj = gtk4::Adjustment::new(
        settings.upscaler_tile_size as f64,
        0.0,
        4096.0,
        32.0,
        64.0,
        0.0,
    );
    let tile_spin = gtk4::SpinButton::new(Some(&tile_adj), 1.0, 0);
    tile_spin.set_numeric(true);
    tile_row.add_suffix(&tile_spin);
    tile_row.set_activatable_widget(Some(&tile_spin));

    {
        let parent_c = parent.clone();
        tile_spin.connect_value_changed(move |spin| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_upscaler_tile_size(spin.value() as i32);
        });
    }

    let gpu_row = libadwaita::ActionRow::new();
    gpu_row.set_title("GPU ID");
    gpu_row.set_subtitle("-1 means auto");
    let gpu_adj = gtk4::Adjustment::new(settings.upscaler_gpu_id as f64, -1.0, 16.0, 1.0, 1.0, 0.0);
    let gpu_spin = gtk4::SpinButton::new(Some(&gpu_adj), 1.0, 0);
    gpu_spin.set_numeric(true);
    gpu_row.add_suffix(&gpu_spin);
    gpu_row.set_activatable_widget(Some(&gpu_spin));

    {
        let parent_c = parent.clone();
        gpu_spin.connect_value_changed(move |spin| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_upscaler_gpu_id(spin.value() as i32);
        });
    }

    upscaler_group.add(&binary_row);
    upscaler_group.add(&model_row);
    upscaler_group.add(&output_row);
    upscaler_group.add(&compression_row);
    upscaler_group.add(&quality_row);
    upscaler_group.add(&tile_row);
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

    {
        let parent_c = parent.clone();
        comfy_url_row.connect_changed(move |row| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_comfyui_url(row.text().as_str());
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
    comfy_group.add(&test_row);
    upscaler_page.add(&comfy_group);

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

    let cache_row = libadwaita::ActionRow::new();
    cache_row.set_title("Thumbnail cache size");
    cache_row.set_subtitle("Maximum images held in memory");

    let cache_spin = gtk4::SpinButton::with_range(100.0, 2000.0, 100.0);
    cache_spin.set_value(settings.thumbnail_cache_max as f64);
    cache_row.add_suffix(&cache_spin);
    cache_row.set_activatable_widget(Some(&cache_spin));

    {
        let parent_c = parent.clone();
        cache_spin.connect_value_changed(move |spin| {
            parent_c
                .app_state()
                .borrow_mut()
                .settings
                .set_thumbnail_cache_max(spin.value_as_int());
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
    edit_button.connect_clicked(move |_| {
        present_library_editor(Some(library_id.clone()), &parent_c, group.clone());
    });
    row
}

fn present_library_editor(
    library_id: Option<String>,
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
    dialog.add_response("cancel", "Cancel");
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
        let app_state_ref = parent_c.app_state();
        let mut state = app_state_ref.borrow_mut();
        let result = if let Some(id) = library_id.as_deref() {
            state
                .settings
                .update_library(id, name_entry.text().as_str(), root, folder_mode)
        } else {
            state
                .settings
                .add_library(name_entry.text().as_str(), root, folder_mode)
                .map(|_| ())
        };

        match result {
            Ok(()) => {
                while let Some(child) = group.first_child() {
                    group.remove(&child);
                }
                for library in state.settings.libraries.clone() {
                    group.add(&build_library_row(&library, &parent_c, group.clone()));
                }
                let add_row = libadwaita::ActionRow::new();
                add_row.set_title("Add Library");
                add_row.set_subtitle("Create another library root and folder mode.");
                let add_button = gtk4::Button::with_label("Create…");
                add_row.add_suffix(&add_button);
                add_row.set_activatable_widget(Some(&add_button));
                let group_c = group.clone();
                let parent_cc = parent_c.clone();
                add_button.connect_clicked(move |_| {
                    present_library_editor(None, &parent_cc, group_c.clone());
                });
                group.add(&add_row);
            }
            Err(err) => {
                let error = libadwaita::AlertDialog::new(Some("Could not save library"), Some(&err));
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

fn output_folder_subtitle(path: Option<&PathBuf>, kind: crate::export::OutputFolderKind) -> String {
    match path {
        Some(path) => path.to_string_lossy().into_owned(),
        None => crate::export::default_output_dir(kind)
            .to_string_lossy()
            .into_owned(),
    }
}

fn action_state_bool(window: &SharprWindow, action_name: &str) -> Option<bool> {
    window
        .lookup_action(action_name)
        .and_then(|action| action.state())
        .and_then(|state| state.get::<bool>())
}
