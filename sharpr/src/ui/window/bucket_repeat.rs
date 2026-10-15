//! Repeat the last successful bucket destination through the existing writer.
use super::*;

impl SharprWindow {
    pub(crate) fn add_to_last_bucket(&self, paths: Vec<PathBuf>) {
        if file_actions_blocked(self.upcast_ref())
            || self.visible_dialog().is_some()
            || paths.is_empty()
        {
            return;
        }
        let id = {
            let state = self.imp().state.borrow();
            state
                .settings
                .last_bucket()
                .filter(|id| state.collections.iter().any(|c| c.id == *id))
        };
        if let Some(id) = id {
            self.change_collection_tags(id, paths, false);
        } else {
            self.add_toast(libadwaita::Toast::new("Choose a bucket with Ctrl+B first"));
        }
    }

    pub(super) fn setup_bucket_repeat(&self) {
        let action = gio::SimpleAction::new("add-last-bucket", None);
        let weak = self.downgrade();
        action.connect_activate(move |_, _| {
            let Some(window) = weak.upgrade() else { return };
            let paths = window
                .imp()
                .filmstrip
                .borrow()
                .as_ref()
                .map(|filmstrip| filmstrip.selected_paths_for_action())
                .unwrap_or_default();
            window.add_to_last_bucket(paths);
        });
        self.add_action(&action);
        let shortcuts = gtk4::ShortcutController::new();
        shortcuts.set_scope(gtk4::ShortcutScope::Managed);
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Control><Shift>b").unwrap()),
            Some(gtk4::CallbackAction::new(|widget, _| {
                let Some(window) = widget.downcast_ref::<SharprWindow>() else {
                    return glib::Propagation::Proceed;
                };
                if text_input_has_focus(widget)
                    || window.visible_dialog().is_some()
                    || file_actions_blocked(widget)
                {
                    return glib::Propagation::Proceed;
                }
                let mut focus = gtk4::prelude::GtkWindowExt::focus(window);
                while let Some(widget) = focus {
                    if widget.is::<gtk4::Popover>() {
                        return glib::Propagation::Proceed;
                    }
                    focus = widget.parent();
                }
                gio::prelude::ActionGroupExt::activate_action(window, "add-last-bucket", None);
                glib::Propagation::Stop
            })),
        ));
        self.add_controller(shortcuts);
    }
}
