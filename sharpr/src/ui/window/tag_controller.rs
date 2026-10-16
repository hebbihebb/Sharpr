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
            self.add_toast(libadwaita::Toast::new("Bucket database is unavailable"));
            return;
        };
        let count = paths.len();
        let settings = self.imp().state.borrow().settings.clone();
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
                    settings.remember_bucket(collection.id);
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
        let (index, scope) = {
            let state = self.imp().state.borrow();
            (state.library_index.clone(), state.scope.clone())
        };
        let Some(index) = index else {
            self.add_toast(libadwaita::Toast::new("Bucket database is unavailable"));
            return;
        };
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(crate::buckets::reparent_bucket(&index, id, parent));
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(win) = weak.upgrade() else { return };
            match result {
                Ok(name) => {
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
                        if let ViewScope::Collection(active) = scope {
                            win.refresh_current_bucket(active);
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
        let settings = self.imp().state.borrow().settings.clone();
        let library_id = settings.active_library_id.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                if !index
                    .list_collections_for_library(library_id.as_deref())
                    .map_err(|e| e.to_string())?
                    .iter()
                    .any(|bucket| bucket.id == id)
                {
                    return Err("Bucket is no longer in this library".into());
                }
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
                Ok((name, changed))
            })();
            let _ = tx.send_blocking(result);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(win) = weak.upgrade() else { return };
            match result {
                Ok((name, changed)) => {
                    if !remove {
                        settings.remember_bucket(id);
                    }
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
                        win.refresh_current_bucket(id);
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
        if self.app_state().borrow().bucket_migration_pending {
            return;
        }
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

    /// Looks for manual tags no bucket uses and offers them as buckets
    /// (`ROADMAP.md` Phase 5). With `once`, it only shows a toast pointing to
    /// the offer, and only the first time there is something to offer.
    pub(super) fn offer_tags_as_buckets(&self, once: bool) {
        if self.app_state().borrow().bucket_migration_pending {
            return;
        }
        const NOTICE: &str = "tag_bucket_offer_shown_at";
        let (index, tags, root, disabled) = {
            let st = self.imp().state.borrow();
            (
                st.library_index.clone(),
                st.tags.clone(),
                st.settings.active_library().map(|l| l.root.clone()),
                st.disabled_folders.clone(),
            )
        };
        let (Some(index), Some(tags), Some(root)) = (index, tags, root) else {
            if !once {
                self.add_toast(libadwaita::Toast::new(
                    "Tag database or library index is unavailable",
                ));
            }
            return;
        };
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                if once && index.notice_shown(NOTICE).map_err(|e| e.to_string())? {
                    return Ok(None);
                }
                let offers = crate::buckets::tag_offers(&index, &tags, &root, &disabled)?;
                if once && !offers.is_empty() {
                    index.mark_notice_shown(NOTICE).map_err(|e| e.to_string())?;
                }
                Ok(Some(offers))
            })();
            let _ = tx.send_blocking(result);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(win) = weak.upgrade() else { return };
            match result {
                Ok(None) => {}
                Ok(Some(offers)) if offers.is_empty() => {
                    if !once {
                        win.add_toast(libadwaita::Toast::new(
                            "No tags are left to turn into buckets",
                        ));
                    }
                }
                Ok(Some(offers)) if once => {
                    let toast = libadwaita::Toast::new(&format!(
                        "Tags are retired. {} of your tags can become buckets",
                        offers.len()
                    ));
                    toast.set_button_label(Some("Review…"));
                    toast.set_timeout(0);
                    let weak = win.downgrade();
                    toast.connect_button_clicked(move |_| {
                        if let Some(win) = weak.upgrade() {
                            win.present_tag_offers(offers.clone());
                        }
                    });
                    win.add_toast(toast);
                }
                Ok(Some(offers)) => win.present_tag_offers(offers),
                Err(error) => crate::bench_warn!(
                    "tags.bucket_offer.fail",
                    serde_json::json!({"error": error})
                ),
            }
        });
    }

    fn present_tag_offers(&self, offers: Vec<crate::buckets::TagOffer>) {
        let dialog = libadwaita::AlertDialog::new(
            Some("Turn Tags into Buckets"),
            Some(
                "Tags are retired. Pick the tags to keep as buckets: each becomes a bucket \
                 with the same images. The tags themselves are kept, unused.",
            ),
        );
        dialog.add_response("cancel", "Not Now");
        dialog.add_response("create", "Create Buckets");
        dialog.set_response_appearance("create", libadwaita::ResponseAppearance::Suggested);
        dialog.set_response_enabled("create", false);
        dialog.set_default_response(Some("create"));
        dialog.set_close_response("cancel");

        let list = gtk4::ListBox::new();
        list.set_selection_mode(gtk4::SelectionMode::None);
        list.add_css_class("boxed-list");
        let checks: Rc<Vec<(String, gtk4::CheckButton)>> = Rc::new(
            offers
                .iter()
                .map(|offer| {
                    let row = libadwaita::ActionRow::new();
                    row.set_title(&offer.tag);
                    row.set_subtitle(&format!(
                        "{} {}",
                        offer.images,
                        if offer.images == 1 { "image" } else { "images" }
                    ));
                    let check = gtk4::CheckButton::new();
                    check.set_valign(gtk4::Align::Center);
                    row.add_prefix(&check);
                    row.set_activatable_widget(Some(&check));
                    list.append(&row);
                    (offer.tag.clone(), check)
                })
                .collect(),
        );
        for (_, check) in checks.iter() {
            let dialog = dialog.downgrade();
            let all = Rc::downgrade(&checks);
            check.connect_toggled(move |_| {
                let (Some(dialog), Some(all)) = (dialog.upgrade(), all.upgrade()) else {
                    return;
                };
                dialog.set_response_enabled("create", all.iter().any(|(_, c)| c.is_active()));
            });
        }
        let scroller = gtk4::ScrolledWindow::new();
        scroller.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scroller.set_propagate_natural_height(true);
        scroller.set_max_content_height(360);
        scroller.set_child(Some(&list));
        dialog.set_extra_child(Some(&scroller));

        let weak = self.downgrade();
        dialog.connect_response(None, move |_, response| {
            if response != "create" {
                return;
            }
            let chosen: Vec<String> = checks
                .iter()
                .filter(|(_, check)| check.is_active())
                .map(|(tag, _)| tag.clone())
                .collect();
            if let Some(win) = weak.upgrade() {
                win.create_buckets_from_tags(chosen);
            }
        });
        dialog.present(Some(self));
    }

    fn create_buckets_from_tags(&self, chosen: Vec<String>) {
        let (index, tags, library, root, disabled) = {
            let st = self.imp().state.borrow();
            (
                st.library_index.clone(),
                st.tags.clone(),
                st.settings.active_library_id.clone(),
                st.settings.active_library().map(|l| l.root.clone()),
                st.disabled_folders.clone(),
            )
        };
        let (Some(index), Some(tags), Some(library), Some(root)) = (index, tags, library, root)
        else {
            self.add_toast(libadwaita::Toast::new(
                "Tag database or library index is unavailable",
            ));
            return;
        };
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let results: Vec<(String, Result<usize, String>)> = chosen
                .into_iter()
                .map(|tag| {
                    let result = crate::buckets::bucket_from_tag(
                        &index, &tags, &library, &root, &disabled, &tag,
                    )
                    .map(|(_, members)| members);
                    (tag, result)
                })
                .collect();
            let _ = tx.send_blocking(results);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(results) = rx.recv().await else { return };
            let Some(win) = weak.upgrade() else { return };
            let created = results.iter().filter(|(_, r)| r.is_ok()).count();
            for (tag, result) in &results {
                match result {
                    Ok(members) => crate::bench_event!(
                        "tags.bucket_created",
                        serde_json::json!({"tag": tag, "members": members})
                    ),
                    Err(error) => crate::bench_warn!(
                        "tags.bucket_create.fail",
                        serde_json::json!({"tag": tag, "error": error})
                    ),
                }
            }
            if let Some(refresh) = win.imp().refresh_sidebar_collections_cb.borrow().as_ref() {
                refresh();
            }
            if let Some(filmstrip) = win.imp().filmstrip.borrow().as_ref() {
                filmstrip.rebuild_path_colors();
            }
            let failed: Vec<String> = results
                .into_iter()
                .filter_map(|(tag, r)| r.err().map(|e| format!("{tag}: {e}")))
                .collect();
            let message = if failed.is_empty() {
                format!(
                    "Created {created} {}",
                    if created == 1 { "bucket" } else { "buckets" }
                )
            } else {
                format!("Created {created}; could not create {}", failed.join("; "))
            };
            win.add_toast(libadwaita::Toast::new(&message));
        });
    }
}
