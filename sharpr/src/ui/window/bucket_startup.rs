//! Present the window before the backed-up bucket membership upgrade runs.
use super::*;

impl SharprWindow {
    pub(super) fn start_bucket_upgrade(&self) {
        let (index, tags) = {
            let state = self.app_state();
            let state = state.borrow();
            if !state.bucket_migration_pending {
                return;
            }
            (
                state
                    .library_index
                    .clone()
                    .expect("pending upgrade has an index"),
                state.tags.clone().expect("pending upgrade has tags"),
            )
        };
        let stack = self
            .imp()
            .content_stack
            .borrow()
            .clone()
            .expect("startup runs after the content stack is built");
        let status = libadwaita::StatusPage::builder()
            .title("Preparing library…")
            .description("Checking and upgrading buckets. Your images stay unchanged.")
            .build();
        let spinner = gtk4::Spinner::new();
        spinner.start();
        spinner.set_halign(gtk4::Align::Center);
        status.set_child(Some(&spinner));
        stack.add_named(&status, Some("initializing"));
        stack.set_visible_child_name("initializing");
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.set_sensitive(false);
            sidebar.imp().bucket_box.set_sensitive(false);
        }
        if let Some(strip) = self.imp().filmstrip.borrow().as_ref() {
            strip.set_sensitive(false);
        }
        let weak = self.downgrade();
        glib::idle_add_local_once(move || {
            let Some(window) = weak.upgrade() else { return };
            let operation = window.app_state().borrow().ops.add("Preparing buckets…");
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let backups = dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("sharpr/backups");
                let _ = tx.send_blocking(crate::buckets::migrate(&index, &tags, &backups));
            });
            let weak = window.downgrade();
            glib::MainContext::default().spawn_local(async move {
                let result = rx.recv().await.unwrap_or_else(|_| Err("Bucket upgrade worker stopped".into()));
                let Some(window) = weak.upgrade() else { return };
                let error = result.err();
                {
                    let state = window.app_state();
                    let mut state = state.borrow_mut();
                    state.bucket_migration_pending = false;
                    state.bucket_migration_error = error.clone();
                }
                if let Some(sidebar) = window.imp().sidebar.borrow().as_ref() {
                    sidebar.set_sensitive(true);
                    sidebar.imp().bucket_box.set_sensitive(error.is_none());
                }
                if let Some(strip) = window.imp().filmstrip.borrow().as_ref() { strip.set_sensitive(true); }
                if let Some(error) = error {
                    operation.fail(error.clone());
                    crate::bench_error!("buckets.migrate.fail", serde_json::json!({"error":error}));
                    let toast = libadwaita::Toast::new(&format!("Buckets could not be upgraded, so they can’t be changed yet. Nothing was lost: {error}"));
                    toast.set_timeout(0);
                    window.add_toast(toast);
                } else {
                    operation.complete();
                }
                if let Some(stack) = window.imp().content_stack.borrow().as_ref() {
                    stack.set_visible_child_name("welcome");
                    stack.remove(&status);
                }
                if let Some(refresh) = window.imp().refresh_sidebar_collections_cb.borrow().clone() { refresh(); }
                let folder = window.app_state().borrow().settings.last_folder.clone()
                    .or_else(|| window.imp().sidebar.borrow().as_ref().and_then(|s| s.first_folder_path()));
                if let (Some(folder), Some(open)) = (folder, window.imp().open_folder_cb.borrow().clone()) { open(folder); }
                window.library_scope_changed();
            });
        });
    }
}
