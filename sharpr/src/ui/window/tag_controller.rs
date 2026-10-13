//! Bucket membership writes finish before success feedback, outside GTK.
use super::*;

impl SharprWindow {
    pub(super) fn create_collection_with_paths(
        &self,
        name: String,
        extra: Vec<String>,
        paths: Vec<PathBuf>,
    ) {
        let (index, library, root, disabled) = {
            let st = self.imp().state.borrow();
            (
                st.library_index.clone(),
                st.settings.active_library_id.clone().unwrap_or_default(),
                st.settings.active_library().map(|l| l.root.clone()),
                st.disabled_folders.clone(),
            )
        };
        let Some(index) = index else {
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
                if let Err(error) = crate::buckets::add_to_bucket(&index, collection.id, &filtered)
                {
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
                        "Added {count} images to {}",
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
        let (index, root, disabled, scope) = {
            let state = self.imp().state.borrow();
            (
                state.library_index.clone(),
                state.settings.active_library().map(|l| l.root.clone()),
                state.disabled_folders.clone(),
                state.scope.clone(),
            )
        };
        let Some(index) = index else {
            self.add_toast(libadwaita::Toast::new("Collection database is unavailable"));
            return;
        };
        let (tx, rx) = async_channel::bounded(1);
        let worker_scope = scope.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                let name = crate::buckets::reparent_bucket(&index, id, parent)?;
                let remaining = if let ViewScope::Collection(active) = worker_scope {
                    Some(crate::buckets::view(&index, active, root.as_deref(), &disabled)?.paths)
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
        let (index, root, disabled, scope) = {
            let st = self.imp().state.borrow();
            (
                st.library_index.clone(),
                st.settings.active_library().map(|l| l.root.clone()),
                st.disabled_folders.clone(),
                st.scope.clone(),
            )
        };
        let Some(index) = index else {
            self.add_toast(libadwaita::Toast::new("Library index is unavailable"));
            return;
        };
        let count = paths.len();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                let paths = filter_paths_for_library(paths, root.as_deref(), &disabled);
                if paths.len() != count {
                    return Err(
                        "Some images are outside the active library or in disabled folders".into(),
                    );
                }
                let changed = if remove {
                    crate::buckets::remove_from_bucket(&index, id, &paths)?
                } else {
                    crate::buckets::add_to_bucket(&index, id, &paths)?
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
                let remaining = crate::buckets::view(&index, id, root.as_deref(), &disabled)?.paths;
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
                        if remove { "Removed" } else { "Added" },
                        if remove { "from" } else { "to" }
                    )));
                }
                Err(error) => win.add_toast(libadwaita::Toast::new(&format!(
                    "Could not update collection: {error}"
                ))),
            }
        });
    }

    /// Finds the tags the old automatic tagger added (decision D17) and, after
    /// confirmation and a backup, removes them. Manual tags and every tag a
    /// bucket uses are kept.
    pub(super) fn start_automatic_tag_cleanup(&self) {
        let (index, tags) = {
            let st = self.imp().state.borrow();
            (st.library_index.clone(), st.tags.clone())
        };
        let (Some(index), Some(tags)) = (index, tags) else {
            self.add_toast(libadwaita::Toast::new(
                "Tag database or library index is unavailable",
            ));
            return;
        };
        let (tx, rx) = async_channel::bounded(1);
        let tags_worker = tags.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                let protected: std::collections::HashSet<String> = index
                    .list_collections()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .flat_map(|c| std::iter::once(c.primary_tag).chain(c.extra_tags))
                    .collect();
                tags_worker.automatic_tag_rows(&protected)
            })();
            let _ = tx.send_blocking(result);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(win) = weak.upgrade() else { return };
            match result {
                Ok(rows) if rows.is_empty() => {
                    win.add_toast(libadwaita::Toast::new("No old automatic tags found"));
                }
                Ok(rows) => win.confirm_automatic_tag_cleanup(rows, tags),
                Err(error) => win.add_toast(libadwaita::Toast::new(&format!(
                    "Could not look for automatic tags: {error}"
                ))),
            }
        });
    }

    fn confirm_automatic_tag_cleanup(
        &self,
        rows: Vec<(String, String)>,
        tags: Arc<crate::tags::TagDatabase>,
    ) {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for (_, tag) in &rows {
            match counts.iter_mut().find(|(t, _)| t == tag) {
                Some((_, n)) => *n += 1,
                None => counts.push((tag.clone(), 1)),
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let summary = counts
            .iter()
            .map(|(tag, n)| format!("{tag} ({n})"))
            .collect::<Vec<_>>()
            .join(", ");
        let backups = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sharpr")
            .join("backups");
        let body = format!(
            "{} tags with the names Sharpr once added automatically will be removed: \
             {summary}.\n\nThey are matched by name: a resolution tag (720p, 1080p, 4k), or a \
             format tag equal to the file’s own type, such as png on a PNG file. If you \
             added one of these yourself, it is removed too. Tags used by buckets and all \
             other tags are kept. The tag database is backed up first, to {}.",
            rows.len(),
            backups.display()
        );
        let dialog = libadwaita::AlertDialog::new(Some("Remove Old Automatic Tags?"), Some(&body));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("remove", "Remove");
        dialog.set_response_appearance("remove", libadwaita::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let weak = self.downgrade();
        dialog.connect_response(None, move |_, response| {
            if response != "remove" {
                return;
            }
            let (tx, rx) = async_channel::bounded(1);
            let rows = rows.clone();
            let tags = tags.clone();
            let backups = backups.clone();
            std::thread::spawn(move || {
                let result = (|| -> Result<(usize, PathBuf), String> {
                    std::fs::create_dir_all(&backups).map_err(|e| e.to_string())?;
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or_default();
                    let backup =
                        backups.join(format!("tags-before-automatic-cleanup-{stamp}.sqlite3"));
                    tags.backup_to(&backup)?;
                    let removed = tags.delete_rows(&rows)?;
                    Ok((removed, backup))
                })();
                let _ = tx.send_blocking(result);
            });
            let weak = weak.clone();
            glib::MainContext::default().spawn_local(async move {
                let Ok(result) = rx.recv().await else { return };
                let Some(win) = weak.upgrade() else { return };
                match result {
                    Ok((removed, backup)) => {
                        crate::bench_event!(
                            "tags.automatic_cleanup",
                            serde_json::json!({
                                "removed": removed,
                                "backup": backup.display().to_string(),
                            })
                        );
                        win.add_toast(libadwaita::Toast::new(&format!(
                            "Removed {removed} automatic tags; a backup was saved first"
                        )));
                        win.tags_changed();
                    }
                    Err(error) => {
                        crate::bench_error!(
                            "tags.automatic_cleanup.fail",
                            serde_json::json!({"error": error})
                        );
                        win.add_toast(libadwaita::Toast::new(&format!(
                            "Nothing was removed: {error}"
                        )));
                    }
                }
            });
        });
        dialog.present(Some(self));
    }
}
