//! One editable root, keeping the active library's identity and stored data.
use super::*;

impl SharprWindow {
    pub(super) fn setup_library_root(&self) {
        let action = gio::SimpleAction::new("choose-library-root", None);
        let weak = self.downgrade();
        action.connect_activate(move |_, _| {
            let Some(window) = weak.upgrade() else { return };
            if file_actions_blocked(window.upcast_ref()) {
                return;
            }
            let chooser = gtk4::FileDialog::new();
            chooser.set_title("Open Library Root");
            let previous = window
                .app_state()
                .borrow()
                .settings
                .active_library()
                .cloned();
            if let Some(library) = previous.as_ref() {
                chooser.set_initial_folder(Some(&gio::File::for_path(&library.root)));
            }
            let weak = window.downgrade();
            chooser.select_folder(Some(&window), None::<&gio::Cancellable>, move |result| {
                let Ok(file) = result else { return };
                let Some(root) = file.path() else { return };
                crate::ui::collection_dialogs::run_collection_write(
                    move || {
                        if root.is_dir() {
                            Ok(root)
                        } else {
                            Err("Choose an available local folder".into())
                        }
                    },
                    move |result| {
                        let Some(window) = weak.upgrade() else { return };
                        // A second choice or a mode entered during the native
                        // chooser must not be replaced by its delayed response.
                        if window.app_state().borrow().settings.active_library()
                            != previous.as_ref()
                            || file_actions_blocked(window.upcast_ref())
                        {
                            return;
                        }
                        match result.and_then(|root| window.change_library_root(root)) {
                            Ok(()) => {
                                window.add_toast(libadwaita::Toast::new("Library root changed"))
                            }
                            Err(error) => window.add_toast(libadwaita::Toast::new(&error)),
                        }
                    },
                );
            });
        });
        self.add_action(&action);
    }

    /// The chooser validates directory availability on its worker first.
    pub(crate) fn change_library_root(&self, root: PathBuf) -> Result<(), String> {
        if file_actions_blocked(self.upcast_ref()) {
            return Err("Finish the current review or crop first".into());
        }
        {
            let state = self.app_state();
            let mut state = state.borrow_mut();
            if state
                .settings
                .active_library()
                .is_some_and(|library| library.root == root)
            {
                return Ok(());
            }
            state.settings.set_single_library_root(root)?;
            state.scope = ViewScope::Search;
            state.selected_paths.clear();
            state.selection_explicit = false;
            let _ = state.library.load_virtual(&[]);
        }
        self.clear_inline_search(true);
        self.bump_thumbnail_generation("library.root");
        self.complete_thumbnail_ops();
        if let Some(viewer) = self.imp().viewer.borrow().as_ref() {
            viewer.clear();
        }
        if let Some(strip) = self.imp().filmstrip.borrow().as_ref() {
            strip.refresh();
            strip.refresh_multi_selection_visuals();
        }
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.refresh_active_library(self.app_state());
        }
        if let Some(refresh) = self.imp().refresh_sidebar_collections_cb.borrow().as_ref() {
            refresh();
        }
        let folder = self.app_state().borrow().settings.last_folder.clone();
        if let (Some(folder), Some(open)) = (folder, self.imp().open_folder_cb.borrow().as_ref()) {
            open(folder);
        }
        self.library_scope_changed();
        self.update_empty_title();
        Ok(())
    }
}
