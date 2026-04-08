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
        let settings_c = settings.clone();
        let row_c = library_root_row.clone();
        let parent_window = parent.clone().upcast::<gtk4::Window>();
        choose_button.connect_clicked(move |_| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Choose Library Root");
            let settings_inner = settings_c.clone();
            let row_inner = row_c.clone();
            dialog.select_folder(
                Some(&parent_window),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            let mut settings = settings_inner.clone();
                            settings.set_library_root(Some(path.clone()));
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
    window.add(&library_page);

    let upscaler_page = libadwaita::PreferencesPage::new();
    upscaler_page.set_title("Upscaler");
    upscaler_page.set_icon_name(Some("image-x-generic-symbolic"));

    let upscaler_group = libadwaita::PreferencesGroup::new();
    upscaler_group.set_title("AI Upscale (realesrgan-ncnn-vulkan)");

    let binary_row = libadwaita::ActionRow::new();
    binary_row.set_title("Binary path");
    binary_row.set_subtitle("Leave blank to auto-detect from ~/.local/bin/");

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
        let settings_c = settings.clone();
        binary_entry.connect_changed(move |entry| {
            let text = entry.text().trim().to_string();
            let mut settings = settings_c.clone();
            settings.set_upscaler_binary_path(if text.is_empty() {
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
        let settings_c = settings.clone();
        model_row.connect_selected_notify(move |row| {
            let mut settings = settings_c.clone();
            settings.set_upscaler_default_model(if row.selected() == 1 {
                "anime"
            } else {
                "standard"
            });
        });
    }

    upscaler_group.add(&binary_row);
    upscaler_group.add(&model_row);
    upscaler_page.add(&upscaler_group);
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
        action_state_bool(parent.as_ref(), "show-metadata").unwrap_or(settings.metadata_visible),
    );

    {
        let settings_c = settings.clone();
        let parent_c = parent.clone();
        metadata_row.connect_active_notify(move |row| {
            let desired = row.is_active();
            if action_state_bool(&parent_c, "show-metadata") != Some(desired) {
                let _ = gtk4::prelude::ActionGroupExt::activate_action(
                    &parent_c,
                    "show-metadata",
                    None,
                );
            }
            let mut settings = settings_c.clone();
            settings.set_metadata_visible(desired);
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
        let settings_c = settings.clone();
        cache_spin.connect_value_changed(move |spin| {
            let mut settings = settings_c.clone();
            settings.set_thumbnail_cache_max(spin.value_as_int());
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
