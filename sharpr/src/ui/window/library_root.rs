//! The library's roots (D32, D41): change the first, add more, forget one.
//! All of them keep the active library's identity and stored data; nothing
//! here touches a file.
use super::*;

type RootChange = fn(&SharprWindow, PathBuf) -> Result<(), String>;

impl SharprWindow {
    pub(super) fn setup_library_root(&self) {
        self.add_root_chooser_action(
            "choose-library-root",
            "Open Library Root",
            SharprWindow::change_library_root,
            "Library root changed",
        );
        self.add_root_chooser_action(
            "add-library-root",
            "Add Library Root",
            SharprWindow::add_library_root,
            "Library root added",
        );

        let remove = gio::SimpleAction::new("remove-library-root", Some(glib::VariantTy::STRING));
        let weak = self.downgrade();
        remove.connect_activate(move |_, target| {
            let Some(window) = weak.upgrade() else { return };
            let Some(root) = target.and_then(|t| t.get::<String>()).map(PathBuf::from) else {
                return;
            };
            let message = match window.remove_library_root(&root) {
                Ok(()) => format!(
                    "Removed “{}” from the library; its files are untouched",
                    root.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.display().to_string())
                ),
                Err(error) => error,
            };
            window.add_toast(libadwaita::Toast::new(&message));
        });
        self.add_action(&remove);
    }

    /// A folder chooser action whose choice is checked on a worker and then
    /// handed to `apply`.
    fn add_root_chooser_action(
        &self,
        name: &str,
        title: &'static str,
        apply: RootChange,
        done: &'static str,
    ) {
        let action = gio::SimpleAction::new(name, None);
        let weak = self.downgrade();
        action.connect_activate(move |_, _| {
            let Some(window) = weak.upgrade() else { return };
            if file_actions_blocked(window.upcast_ref()) {
                return;
            }
            let chooser = gtk4::FileDialog::new();
            chooser.set_title(title);
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
                        match result.and_then(|root| apply(&window, root)) {
                            Ok(()) => window.add_toast(libadwaita::Toast::new(done)),
                            Err(error) => window.add_toast(libadwaita::Toast::new(&error)),
                        }
                    },
                );
            });
        });
        self.add_action(&action);
    }

    /// Changes the first root. The chooser validates directory availability
    /// on its worker first.
    pub(crate) fn change_library_root(&self, root: PathBuf) -> Result<(), String> {
        if self
            .app_state()
            .borrow()
            .settings
            .active_library()
            .is_some_and(|library| library.root == root)
        {
            return Ok(());
        }
        self.edit_library_roots(|settings| settings.set_single_library_root(root))
    }

    /// Adds another root, listed below the others (D41).
    pub(crate) fn add_library_root(&self, root: PathBuf) -> Result<(), String> {
        self.edit_library_roots(|settings| settings.add_library_root(root))
    }

    /// Forgets a root. Its files, fingerprints, vectors and bucket rows stay;
    /// its bucket members are hidden until the root is added back.
    pub(crate) fn remove_library_root(&self, root: &Path) -> Result<(), String> {
        self.edit_library_roots(|settings| settings.remove_library_root(root))
    }

    /// Applies a change to the roots, then shows the library afresh: browse
    /// results are retired, thumbnail requests invalidated, the sidebar
    /// rescanned and new fingerprint and visual-index scopes requested.
    fn edit_library_roots(
        &self,
        edit: impl FnOnce(&mut crate::config::AppSettings) -> Result<(), String>,
    ) -> Result<(), String> {
        if file_actions_blocked(self.upcast_ref()) {
            return Err("Finish the current review or crop first".into());
        }
        {
            let state = self.app_state();
            let mut state = state.borrow_mut();
            edit(&mut state.settings)?;
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
