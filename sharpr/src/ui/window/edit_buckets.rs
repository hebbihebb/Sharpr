//! Copies and their originals join a bucket under Edits (D35), through the
//! same off-thread bucket writer as Add to Bucket.
use super::*;
use crate::buckets::EditKind;

impl SharprWindow {
    /// Adds each `(original, copy)` pair to `kind`'s bucket, unless the user
    /// turned edit buckets off. A copy saved outside the library would be
    /// hidden there, so its pair is left out and the user is told.
    pub(crate) fn add_edit_copies(&self, kind: EditKind, pairs: Vec<(PathBuf, PathBuf)>) {
        let (enabled, index, library_id, root, disabled, scope) = {
            let st = self.imp().state.borrow();
            (
                st.settings.edit_buckets,
                st.library_index.clone(),
                st.settings.active_library_id.clone(),
                st.settings.active_library().map(|l| l.root.clone()),
                st.disabled_folders.clone(),
                st.scope.clone(),
            )
        };
        let (Some(index), Some(library_id)) = (index, library_id) else {
            return;
        };
        if !enabled || pairs.is_empty() {
            return;
        }
        let inside = |path: &PathBuf| {
            !filter_paths_for_library(vec![path.clone()], root.as_deref(), &disabled).is_empty()
        };
        let outside = pairs.iter().filter(|(_, copy)| !inside(copy)).count();
        let paths: Vec<PathBuf> = pairs
            .into_iter()
            .filter(|(_, copy)| inside(copy))
            .flat_map(|(original, copy)| [original, copy])
            .filter(|path| inside(path))
            .collect();
        let name = kind.bucket_name();
        if outside > 0 {
            self.add_toast(libadwaita::Toast::new(&format!(
                "Saved outside the library, so {name} can't show {}",
                if outside == 1 {
                    "the copy"
                } else {
                    "those copies"
                }
            )));
        }
        if paths.is_empty() {
            return;
        }
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = crate::buckets::add_edit_copies(&index, &library_id, kind, &paths);
            let _ = tx.send_blocking(result);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(win) = weak.upgrade() else { return };
            match result {
                Ok((id, added)) => {
                    crate::bench_event!(
                        "collection.edit_copies",
                        serde_json::json!({"bucket": name, "added": added}),
                    );
                    if let Some(refresh) =
                        win.imp().refresh_sidebar_collections_cb.borrow().as_ref()
                    {
                        refresh();
                    }
                    if win.imp().state.borrow().scope == scope && scope == ViewScope::Collection(id)
                    {
                        win.refresh_current_bucket(id);
                    }
                }
                Err(error) => win.add_toast(libadwaita::Toast::new(&format!(
                    "Could not add to {name}: {error}"
                ))),
            }
        });
    }
}
