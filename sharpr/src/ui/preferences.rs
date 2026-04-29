use std::path::PathBuf;

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::config::AppSettings;
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
    library_group.set_title("Source folders");

    let library_root_row = libadwaita::ActionRow::new();
    library_root_row.set_title("Library root");
    library_root_row.set_subtitle(&library_root_subtitle(settings.library_root.as_ref()));

    let choose_button = gtk4::Button::with_label("Choose…");
    library_root_row.add_suffix(&choose_button);
    library_root_row.set_activatable_widget(Some(&choose_button));

    let restart_note_row = libadwaita::ActionRow::new();
    restart_note_row.set_title("Applying changes");
    restart_note_row.set_subtitle("Restart Sharpr to apply folder changes.");
    restart_note_row.set_sensitive(false);

    {
        let parent_c = parent.clone();
        let row_c = library_root_row.clone();
        let parent_window = parent.clone().upcast::<gtk4::Window>();
        choose_button.connect_clicked(move |_| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Choose Library Root");
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
                                .set_library_root(Some(path.clone()));
                            row_inner.set_subtitle(&library_root_subtitle(Some(&path)));
                        }
                    }
                },
            );
        });
    }

    library_group.add(&library_root_row);
    library_group.add(&restart_note_row);
    library_page.add(&library_group);

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

fn library_root_subtitle(path: Option<&PathBuf>) -> String {
    match path {
        Some(path) => path.to_string_lossy().into_owned(),
        None => "Sharpr scans this folder for images. Default: ~/Pictures".to_string(),
    }
}

fn action_state_bool(window: &SharprWindow, action_name: &str) -> Option<bool> {
    window
        .lookup_action(action_name)
        .and_then(|action| action.state())
        .and_then(|state| state.get::<bool>())
}
