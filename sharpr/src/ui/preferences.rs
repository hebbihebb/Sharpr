use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::config::AppSettings;
use crate::ui::window::SharprWindow;

pub fn build_preferences_dialog(
    settings: &AppSettings,
    parent: &SharprWindow,
) -> libadwaita::PreferencesDialog {
    let dialog = libadwaita::PreferencesDialog::new();
    dialog.set_title("Preferences");
    dialog.add_css_class("sharpr-sheet");

    let library_page = libadwaita::PreferencesPage::new();
    library_page.set_title("Library");
    library_page.set_icon_name(Some("folder-symbolic"));

    let library_group = libadwaita::PreferencesGroup::new();
    library_group.set_title("Library");
    let root_row = libadwaita::ActionRow::new();
    root_row.set_title("Library Root");
    root_row.set_subtitle(
        &settings
            .active_library()
            .map(|library| library.root.display().to_string())
            .unwrap_or_default(),
    );
    let choose = gtk4::Button::with_label("Choose…");
    choose.set_valign(gtk4::Align::Center);
    choose.set_action_name(Some("win.choose-library-root"));
    root_row.add_suffix(&choose);
    root_row.set_activatable_widget(Some(&choose));
    library_group.add(&root_row);
    library_group.set_description(Some(
        "Browse this folder and all its subfolders. Disabled folders stay excluded.",
    ));
    library_page.add(&library_group);
    let collections_group = libadwaita::PreferencesGroup::new();
    collections_group.set_title("Buckets");
    let weak_row = root_row.downgrade();
    let weak_parent = parent.downgrade();
    gtk4::glib::timeout_add_seconds_local(1, move || {
        let (Some(row), Some(parent)) = (weak_row.upgrade(), weak_parent.upgrade()) else {
            return gtk4::glib::ControlFlow::Break;
        };
        let root = parent
            .app_state()
            .borrow()
            .settings
            .active_library()
            .map(|library| library.root.display().to_string())
            .unwrap_or_default();
        row.set_subtitle(&root);
        gtk4::glib::ControlFlow::Continue
    });

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

    dialog.add(&library_page);

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
    dialog.add(&appearance_page);

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

fn action_state_bool(window: &SharprWindow, action_name: &str) -> Option<bool> {
    window
        .lookup_action(action_name)
        .and_then(|action| action.state())
        .and_then(|state| state.get::<bool>())
}
