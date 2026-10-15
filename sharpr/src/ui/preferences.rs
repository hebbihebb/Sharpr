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

    library_page.add(&build_visual_search_group(parent));

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
