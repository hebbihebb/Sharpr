//! The bucket list in the sidebar: creating, opening, renaming, recolouring,
//! deleting and reparenting buckets. Wired once from `SharprWindow::setup`.
use super::*;

impl SharprWindow {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn setup_bucket_sidebar(
        &self,
        state: &Rc<RefCell<AppState>>,
        sidebar: &SidebarPane,
        filmstrip: &FilmstripPane,
        viewer: &ViewerPane,
        content_stack: &gtk4::Stack,
        toast_overlay: &libadwaita::ToastOverlay,
        refresh_sidebar_collections: impl Fn() + Clone + 'static,
    ) {
        // "New Bucket" + button → create root collection metadata.
        {
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            let window_weak = self.downgrade();
            sidebar.connect_collection_add_requested(move || {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                show_new_collection_dialog(
                    win.upcast(),
                    String::new(),
                    state_c.clone(),
                    toast_overlay_c.clone(),
                    refresh_c.clone(),
                );
            });
        }

        // Collection row selected → load as virtual view.
        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let content_stack = content_stack.clone();
            let toast_overlay_c = toast_overlay.clone();
            let window_weak = self.downgrade();
            sidebar.connect_collection_selected(move |id| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                let _ = win.bump_thumbnail_generation("collection.load");
                win.complete_thumbnail_ops();
                win.clear_inline_search(true);
                content_stack.set_visible_child_name("viewer");
                let generation = win.imp().bucket_open_generation.get().wrapping_add(1);
                win.imp().bucket_open_generation.set(generation);
                win.imp().bucket_open_pending.set(Some(generation));
                state_c.borrow_mut().scope = ViewScope::Collection(id);
                apply_scope_to_sidebar(&ViewScope::Collection(id), &sidebar_c);
                let (index, active_roots, disabled) = {
                    let state = state_c.borrow();
                    (
                        state.library_index.clone(),
                        state.settings.library_roots(),
                        state.disabled_folders.clone(),
                    )
                };
                let started = std::time::Instant::now();
                // Membership needs a database query and a file check per member,
                // so it runs on a worker instead of the GTK thread.
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let result = (|| -> Result<crate::buckets::BucketView, String> {
                        let Some(index) = index else {
                            return Err("the bucket database is unavailable".into());
                        };
                        crate::buckets::view(&index, id, &active_roots, &disabled)
                    })();
                    let _ = tx.send_blocking(result);
                });
                let filmstrip_c = filmstrip_c.clone();
                let viewer_c = viewer_c.clone();
                let state_c = state_c.clone();
                let toast_overlay_c = toast_overlay_c.clone();
                let window_weak = window_weak.clone();
                glib::MainContext::default().spawn_local(async move {
                    let Ok(result) = rx.recv().await else { return };
                    let Some(win) = window_weak.upgrade() else {
                        return;
                    };
                    if win.imp().bucket_open_pending.get() == Some(generation) {
                        win.imp().bucket_open_pending.set(None);
                    }
                    if win.imp().bucket_open_generation.get() != generation
                        || state_c.borrow().scope != ViewScope::Collection(id)
                    {
                        return;
                    }
                    let paths = match result {
                        // Missing members are told about once, when a
                        // fingerprint pass drops them (D42), not on every open.
                        Ok(view) => view.paths,
                        Err(error) => {
                            toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                                "Could not open bucket: {error}"
                            )));
                            Vec::new()
                        }
                    };
                    win.update_empty_title();
                    state_c.borrow_mut().selected_paths.clear();
                    load_virtual_async(&state_c, &paths);
                    crate::bench_event!(
                        "collection.load",
                        serde_json::json!({
                            "collection_id": id,
                            "path_count": paths.len(),
                            "duration_ms": crate::bench::duration_ms(started),
                        }),
                    );
                    filmstrip_c.refresh_virtual();
                    let has_first = state_c.borrow().library.entry_at(0).is_some();
                    if has_first {
                        state_c.borrow_mut().library.selected_index = Some(0);
                        filmstrip_c.navigate_to(0);
                    } else {
                        viewer_c.clear();
                        toast_overlay_c.add_toast(libadwaita::Toast::new("No images in bucket"));
                    }
                    // Run what was typed while the bucket was loading.
                    let query = win
                        .imp()
                        .inline_search_entry
                        .borrow()
                        .as_ref()
                        .map(|e| e.text().to_string())
                        .unwrap_or_default();
                    if !query.trim().is_empty() || win.imp().browse_options.borrow().has_filters() {
                        win.run_scoped_search(query);
                    }
                });
            });
        }

        // Collection row context actions.
        {
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            let window_weak = self.downgrade();
            sidebar.connect_collection_rename_requested(move |id| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                show_rename_collection_dialog(
                    win.upcast(),
                    id,
                    state_c.clone(),
                    toast_overlay_c.clone(),
                    refresh_c.clone(),
                );
            });
        }

        {
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            let window_weak = self.downgrade();
            sidebar.connect_collection_color_requested(move |id| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                show_change_color_dialog(
                    win.upcast(),
                    id,
                    state_c.clone(),
                    toast_overlay_c.clone(),
                    refresh_c.clone(),
                );
            });
        }

        // Delete collection → update DB → clear if active → refresh sidebar.
        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            sidebar.connect_collection_delete_requested(move |id| {
                let idx = { state_c.borrow().library_index.clone() };
                if let Some(idx) = idx {
                    let scope = state_c.borrow().scope.clone();
                    let worker_scope = scope.clone();
                    let state = state_c.clone();
                    let refresh = refresh_c.clone();
                    let toast = toast_overlay_c.clone();
                    let viewer = viewer_c.clone();
                    let filmstrip = filmstrip_c.clone();
                    crate::ui::collection_dialogs::run_collection_write(
                        move || {
                            idx.delete_collection(id).map_err(|e| e.to_string())?;
                            if let ViewScope::Collection(active) = worker_scope {
                                return Ok(idx
                                    .collection(active)
                                    .map_err(|e| e.to_string())?
                                    .is_none());
                            }
                            Ok(false)
                        },
                        move |result| match result {
                            Ok(deleted_active) => {
                                crate::bench_event!(
                                    "collection.delete",
                                    serde_json::json!({"id":id})
                                );
                                if deleted_active && state.borrow().scope == scope {
                                    state.borrow_mut().scope = ViewScope::Search;
                                    state.borrow_mut().selected_paths.clear();
                                    load_virtual_async(&state, &[]);
                                    viewer.clear();
                                    filmstrip.refresh_virtual();
                                }
                                refresh();
                            }
                            Err(error) => toast.add_toast(libadwaita::Toast::new(&format!(
                                "Could not delete bucket: {error}"
                            ))),
                        },
                    );
                }
            });
        }

        // Reparent a leaf collection under another collection.
        {
            let window_weak = self.downgrade();
            let reparent_collection = Rc::new(move |source_id: i64, parent: Option<i64>| {
                if let Some(win) = window_weak.upgrade() {
                    win.reparent_collection(source_id, parent);
                }
            });

            let state_move = state.clone();
            let toast_move = toast_overlay.clone();
            let window_weak = self.downgrade();
            let reparent_from_dialog = reparent_collection.clone();
            sidebar.connect_collection_move_requested(move |source_id| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                let collections = {
                    let state = state_move.borrow();
                    collections_for_sidebar(&state)
                };
                let mut excluded_ids = vec![source_id];
                let mut cursor = 0;
                while cursor < excluded_ids.len() {
                    let parent_id = excluded_ids[cursor];
                    for child in collections
                        .iter()
                        .filter(|collection| collection.parent_id == Some(parent_id))
                    {
                        excluded_ids.push(child.id);
                    }
                    cursor += 1;
                }
                let options: Vec<(i64, String)> = flatten_collection_paths(&collections)
                    .into_iter()
                    .filter(|(id, _)| !excluded_ids.contains(id))
                    .collect();
                if options.is_empty() {
                    toast_move.add_toast(libadwaita::Toast::new(
                        "No target bucket is available for reparenting",
                    ));
                    return;
                }

                let dialog = libadwaita::AlertDialog::new(Some("Move into Bucket"), None);
                dialog.add_css_class("sharpr-sheet");
                dialog.add_response("cancel", "Cancel");
                dialog.set_close_response("cancel");
                let list_box = gtk4::ListBox::new();
                list_box.add_css_class("boxed-list");
                list_box.set_selection_mode(gtk4::SelectionMode::Single);
                for (id, label) in options {
                    let row = gtk4::ListBoxRow::new();
                    unsafe {
                        row.set_data("collection-id", id);
                    }
                    let lbl = gtk4::Label::new(Some(&label));
                    lbl.set_halign(gtk4::Align::Start);
                    lbl.set_margin_top(8);
                    lbl.set_margin_bottom(8);
                    lbl.set_margin_start(12);
                    lbl.set_margin_end(12);
                    row.set_child(Some(&lbl));
                    list_box.append(&row);
                }
                let scroll = gtk4::ScrolledWindow::new();
                scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
                scroll.set_max_content_height(300);
                scroll.set_propagate_natural_height(true);
                scroll.set_child(Some(&list_box));
                let extra = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                extra.set_margin_top(6);
                extra.append(&scroll);
                dialog.set_extra_child(Some(&extra));
                let reparent = reparent_from_dialog.clone();
                list_box.connect_row_activated(move |_, row| {
                    let Some(target_id) =
                        (unsafe { row.data("collection-id").map(|p| *p.as_ref()) })
                    else {
                        return;
                    };
                    reparent(source_id, Some(target_id));
                });
                dialog.present(Some(&win));
            });

            let reparent_from_drop = reparent_collection.clone();
            sidebar.connect_collection_reparent_requested(move |source_id, target_parent_id| {
                reparent_from_drop(source_id, Some(target_parent_id));
            });

            let promote_to_root = reparent_collection.clone();
            sidebar.connect_collection_promote_requested(move |source_id| {
                promote_to_root(source_id, None);
            });
        }
    }
}
