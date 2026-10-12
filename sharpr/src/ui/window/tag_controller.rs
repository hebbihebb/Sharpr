//! Bucket membership writes finish before success feedback, outside GTK.
use super::*;

impl SharprWindow {
    pub(super) fn create_collection_with_paths(
        &self,
        name: String,
        extra: Vec<String>,
        paths: Vec<PathBuf>,
    ) {
        let (index, tags, library, root, disabled) = {
            let st = self.imp().state.borrow();
            (
                st.library_index.clone(),
                st.tags.clone(),
                st.settings.active_library_id.clone().unwrap_or_default(),
                st.settings.active_library().map(|l| l.root.clone()),
                st.disabled_folders.clone(),
            )
        };
        let (Some(index), Some(tags)) = (index, tags) else {
            self.add_toast(libadwaita::Toast::new("Collection database is unavailable"));
            return;
        };
        let count = paths.len();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                let filtered = filter_paths_for_library(paths, root.as_deref(), &disabled);
                if filtered.len() != count {
                    return Err(
                        "Some images are outside the active library or in disabled folders".into(),
                    );
                }
                let collection = index
                    .create_collection(&library, None, &name, &extra, None, None)
                    .map_err(|e| e.to_string())?;
                let write = (|| -> Result<(), String> {
                    let effective = index
                        .collection_effective_tags(collection.id)
                        .map_err(|e| e.to_string())?;
                    tags.change_tags_checked(&filtered, &effective, false)?;
                    Ok(())
                })();
                if let Err(error) = write {
                    if let Err(rollback) = index.delete_collection(collection.id) {
                        crate::bench_error!(
                            "collection.create.rollback.fail",
                            serde_json::json!({"error":rollback.to_string()})
                        );
                        return Err(format!("{error}; collection rollback failed: {rollback}"));
                    }
                    return Err(error);
                }
                Ok(collection)
            })();
            let _ = tx.send_blocking(result);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(win) = weak.upgrade() else { return };
            match result {
                Ok(collection) => {
                    crate::bench_event!(
                        "collection.create",
                        serde_json::json!({"id":collection.id,"path_count":count})
                    );
                    if let Some(refresh) =
                        win.imp().refresh_sidebar_collections_cb.borrow().as_ref()
                    {
                        refresh();
                    }
                    win.add_toast(libadwaita::Toast::new(&format!(
                        "Tagged {count} images for {}",
                        collection.name
                    )));
                }
                Err(error) => win.add_toast(libadwaita::Toast::new(&format!(
                    "Could not create collection: {error}"
                ))),
            }
        });
    }

    pub(super) fn reparent_collection(&self, id: i64, parent: Option<i64>) {
        let (index, tags, root, disabled, scope) = {
            let state = self.imp().state.borrow();
            (
                state.library_index.clone(),
                state.tags.clone(),
                state.settings.active_library().map(|l| l.root.clone()),
                state.disabled_folders.clone(),
                state.scope.clone(),
            )
        };
        let (Some(index), Some(tags)) = (index, tags) else {
            self.add_toast(libadwaita::Toast::new("Collection database is unavailable"));
            return;
        };
        let (tx, rx) = async_channel::bounded(1);
        let worker_scope = scope.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                let name =
                    crate::buckets::reparent_bucket(&index, &tags, id, parent, root.as_deref())?;
                let remaining = if let ViewScope::Collection(active) = worker_scope {
                    let effective = index
                        .collection_effective_tags(active)
                        .map_err(|e| e.to_string())?;
                    Some(filter_paths_for_library(
                        tags.paths_for_all_tags_checked(&effective)?,
                        root.as_deref(),
                        &disabled,
                    ))
                } else {
                    None
                };
                Ok((name, remaining))
            })();
            let _ = tx.send_blocking(result);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(win) = weak.upgrade() else { return };
            match result {
                Ok((name, remaining)) => {
                    crate::bench_event!(
                        "collection.reparent",
                        serde_json::json!({"id":id,"parent":parent})
                    );
                    if let Some(refresh) =
                        win.imp().refresh_sidebar_collections_cb.borrow().as_ref()
                    {
                        refresh();
                    }
                    if win.imp().state.borrow().scope == scope {
                        if let Some(paths) = remaining {
                            win.clear_inline_search(true);
                            load_virtual_async(&win.imp().state, &paths);
                            win.present_browse_paths(None);
                        }
                    }
                    win.add_toast(libadwaita::Toast::new(&format!("Moved collection {name}")));
                }
                Err(error) => win.add_toast(libadwaita::Toast::new(&format!(
                    "Could not move collection: {error}"
                ))),
            }
        });
    }

    pub(super) fn change_collection_tags(&self, id: i64, paths: Vec<PathBuf>, remove: bool) {
        let (index, tags, root, disabled, scope) = {
            let st = self.imp().state.borrow();
            (
                st.library_index.clone(),
                st.tags.clone(),
                st.settings.active_library().map(|l| l.root.clone()),
                st.disabled_folders.clone(),
                st.scope.clone(),
            )
        };
        let (Some(index), Some(tags)) = (index, tags) else {
            self.add_toast(libadwaita::Toast::new(
                "Tag database or library index is unavailable",
            ));
            return;
        };
        let count = paths.len();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                let effective = index
                    .collection_effective_tags(id)
                    .map_err(|e| e.to_string())?;
                let paths = filter_paths_for_library(paths, root.as_deref(), &disabled);
                if paths.len() != count {
                    return Err(
                        "Some images are outside the active library or in disabled folders".into(),
                    );
                }
                let changed = if remove {
                    crate::buckets::remove_from_bucket(&index, &tags, id, &paths)?
                } else {
                    crate::buckets::add_to_bucket(&index, &tags, id, &paths)?
                };
                if let Err(error) = index.touch_collection(id) {
                    crate::bench_warn!(
                        "collection.touch.fail",
                        serde_json::json!({"error":error.to_string()})
                    );
                }
                let name = index
                    .collection(id)
                    .map_err(|e| e.to_string())?
                    .map(|c| c.name)
                    .unwrap_or_else(|| "collection".into());
                let remaining = filter_paths_for_library(
                    tags.paths_for_all_tags_checked(&effective)?,
                    root.as_deref(),
                    &disabled,
                );
                Ok((name, changed, remaining))
            })();
            let _ = tx.send_blocking(result);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(win) = weak.upgrade() else { return };
            match result {
                Ok((name, changed, remaining)) => {
                    crate::bench_event!(
                        "collection.tags.changed",
                        serde_json::json!({"id":id,"changed":changed,"remove":remove})
                    );
                    if let Some(refresh) =
                        win.imp().refresh_sidebar_collections_cb.borrow().as_ref()
                    {
                        refresh();
                    }
                    if win.imp().state.borrow().scope == scope && scope == ViewScope::Collection(id)
                    {
                        win.clear_inline_search(true);
                        load_virtual_async(&win.imp().state, &remaining);
                        win.present_browse_paths(None);
                    }
                    win.add_toast(libadwaita::Toast::new(&format!(
                        "{} {count} images {} {name}",
                        if remove { "Removed" } else { "Tagged" },
                        if remove { "from" } else { "for" }
                    )));
                }
                Err(error) => win.add_toast(libadwaita::Toast::new(&format!(
                    "Could not update collection: {error}"
                ))),
            }
        });
    }
}
