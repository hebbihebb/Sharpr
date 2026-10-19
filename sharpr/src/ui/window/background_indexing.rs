//! Background indexing started with the window: the fingerprint service, the
//! visual-search index and searcher, and what follows each fingerprint pass
//! (re-linking bucket members, dropping missing ones, refreshing views).
//! Fingerprints start once startup has settled, at the lowest priority.
use super::*;

impl SharprWindow {
    pub(super) fn start_background_indexing(&self) {
        let Some(index) = self.imp().state.borrow().library_index.clone() else {
            return;
        };
        // After each pass, members follow moved and edited files; views
        // refresh when fingerprints or members changed.
        let (changed_tx, changed_rx) = async_channel::unbounded::<()>();
        let (dropped_tx, dropped_rx) = async_channel::unbounded::<String>();
        let relink_index = index.clone();
        let backups = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sharpr/backups");
        // Visual search indexes what the fingerprint pass just covered.
        let use_gpu = self.imp().state.borrow().settings.visual_search_gpu;
        let visual_index = Arc::new(crate::visual_search::index::Service::start(
            index.clone(),
            crate::visual_search::store::default_path(),
            use_gpu,
        ));
        *self.imp().visual_index.borrow_mut() = Some(visual_index.clone());
        // A deleted or replaced vector file starts indexing again, through
        // a fingerprint pass on the GTK side, which knows the library.
        let (reindex_tx, reindex_rx) = async_channel::unbounded::<()>();
        *self.imp().visual_searcher.borrow_mut() =
            Some(crate::visual_search::search::Searcher::start(
                crate::visual_search::store::default_path(),
                visual_index.vectors_version(),
                Box::new(move || {
                    let _ = reindex_tx.send_blocking(());
                }),
            ));
        {
            let weak = self.downgrade();
            glib::MainContext::default().spawn_local(async move {
                while reindex_rx.recv().await.is_ok() {
                    let Some(win) = weak.upgrade() else { return };
                    win.request_fingerprints();
                }
            });
        }
        let mut cleanup = crate::buckets::CleanupState::default();
        let mut last_trash_error: Option<String> = None;
        let service = crate::fingerprint::Service::start(
            index,
            Box::new(move |job, pass, guard| {
                {
                    let _scope = guard.scope_gate.lock().unwrap_or_else(|e| e.into_inner());
                    if !guard.is_current() {
                        return;
                    }
                    visual_index.request(crate::visual_search::index::Job {
                        roots: job.roots.clone(),
                        disabled: job.disabled.clone(),
                    });
                }
                match crate::buckets::relink_guarded(
                    &relink_index,
                    &job.roots,
                    &job.disabled,
                    &backups,
                    Some(guard),
                ) {
                    Ok(relinked) => {
                        crate::bench_event!(
                            "buckets.relink",
                            serde_json::json!({
                                "fingerprinted": relinked.fingerprinted,
                                "relinked": relinked.relinked,
                            }),
                        );
                        if pass.hashed + relinked.fingerprinted + relinked.relinked > 0 {
                            let _ = changed_tx.send_blocking(());
                        }
                    }
                    Err(error) => crate::bench_warn!(
                        "buckets.relink.fail",
                        serde_json::json!({"error": error}),
                    ),
                }
                // A file the pass could not read may be a moved member,
                // so only a pass that read everything may drop members.
                if pass.failed > 0 {
                    return;
                }
                let trashed = match crate::file_ops::trashed_originals(&job.roots) {
                    Ok(snapshot) => {
                        last_trash_error = None;
                        snapshot
                    }
                    Err(error) => {
                        crate::bench_warn!(
                            "buckets.drop_missing.fail",
                            serde_json::json!({"error":error})
                        );
                        if last_trash_error.as_ref() != Some(&error) {
                            let _ = dropped_tx.send_blocking(format!(
                                "Bucket cleanup paused: {error}. Memberships were kept."
                            ));
                            last_trash_error = Some(error);
                        }
                        return;
                    }
                };
                match crate::buckets::drop_missing(
                    &relink_index,
                    job,
                    pass,
                    &trashed,
                    &backups,
                    &mut cleanup,
                    guard,
                ) {
                    Ok(dropped) if !dropped.is_empty() => {
                        crate::bench_event!(
                            "buckets.drop_missing",
                            serde_json::json!({"members": dropped.values().sum::<usize>()}),
                        );
                        let collections = relink_index.list_collections().unwrap_or_default();
                        let _ = dropped_tx
                            .send_blocking(crate::buckets::dropped_message(&collections, &dropped));
                        let _ = changed_tx.send_blocking(());
                    }
                    Ok(_) => {}
                    Err(error) => crate::bench_warn!(
                        "buckets.drop_missing.fail",
                        serde_json::json!({"error": error}),
                    ),
                }
            }),
        );
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(message) = dropped_rx.recv().await {
                let Some(win) = weak.upgrade() else { return };
                win.add_toast(libadwaita::Toast::new(&message));
            }
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            while changed_rx.recv().await.is_ok() {
                let Some(win) = weak.upgrade() else { return };
                let refresh = win.imp().refresh_sidebar_collections_cb.borrow().clone();
                if let Some(refresh) = refresh {
                    refresh();
                }
                let filmstrip = win.imp().filmstrip.borrow().clone();
                if let Some(filmstrip) = filmstrip {
                    filmstrip.rebuild_path_colors();
                }
                // A member that moved may now be found: the open bucket
                // shows it without being opened again.
                let scope = win.app_state().borrow().scope.clone();
                if let ViewScope::Collection(id) = scope {
                    win.refresh_bucket_if_members_changed(id);
                }
            }
        });
        *self.imp().fingerprints.borrow_mut() = Some(Rc::new(service));
        let weak = self.downgrade();
        glib::timeout_add_seconds_local_once(10, move || {
            if let Some(win) = weak.upgrade() {
                win.request_fingerprints();
            }
        });
        // Tags are retired (Phase 5): point once to turning them into buckets.
        let weak = self.downgrade();
        glib::timeout_add_seconds_local_once(3, move || {
            if let Some(win) = weak.upgrade() {
                win.offer_tags_as_buckets(true);
            }
        });
    }
}
