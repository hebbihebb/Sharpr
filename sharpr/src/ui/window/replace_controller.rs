//! Replace with upgrade (D49): from a selection, a folder or a bucket of
//! upgraded images, find each one's original on a worker, review each pair
//! in Compare's replace mode, then, after one confirmation, let each upgrade
//! take its original's place while the original goes to the trash, with Undo.
use super::*;
use crate::ui::compare_page::replace::{Decision, ReviewItem};
use crate::upgrade::{Match, NoMatch};

/// Where the upgraded images come from.
pub(crate) enum UpgradeSource {
    Paths(Vec<PathBuf>),
    Folder(PathBuf),
    Bucket(i64),
}

/// What matching found for one upgraded image.
struct Found {
    new: PathBuf,
    found: Match,
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

impl SharprWindow {
    pub(super) fn setup_replace_originals(&self) {
        let action =
            gio::SimpleAction::new("replace-originals", Some(glib::VariantTy::STRING_ARRAY));
        let weak = self.downgrade();
        action.connect_activate(move |_, parameter| {
            let (Some(window), Some(paths)) = (
                weak.upgrade(),
                parameter.and_then(|p| p.get::<Vec<String>>()),
            ) else {
                return;
            };
            let paths = paths.into_iter().map(PathBuf::from).collect();
            window.replace_originals(UpgradeSource::Paths(paths));
        });
        self.add_action(&action);
        let action =
            gio::SimpleAction::new("replace-originals-in-folder", Some(glib::VariantTy::STRING));
        let weak = self.downgrade();
        action.connect_activate(move |_, parameter| {
            let (Some(window), Some(folder)) =
                (weak.upgrade(), parameter.and_then(|p| p.get::<String>()))
            else {
                return;
            };
            window.replace_originals(UpgradeSource::Folder(PathBuf::from(folder)));
        });
        self.add_action(&action);
        let action =
            gio::SimpleAction::new("replace-originals-in-bucket", Some(glib::VariantTy::INT64));
        let weak = self.downgrade();
        action.connect_activate(move |_, parameter| {
            let (Some(window), Some(id)) = (weak.upgrade(), parameter.and_then(|p| p.get::<i64>()))
            else {
                return;
            };
            window.replace_originals(UpgradeSource::Bucket(id));
        });
        self.add_action(&action);
    }

    /// Finds the originals of the upgraded images in `source` on a worker,
    /// then shows what was found.
    pub(crate) fn replace_originals(&self, source: UpgradeSource) {
        if file_actions_blocked(self.upcast_ref()) {
            return;
        }
        let (index, roots, disabled) = {
            let st = self.imp().state.borrow();
            (
                st.library_index.clone(),
                st.settings.library_roots(),
                st.disabled_folders.clone(),
            )
        };
        let Some(index) = index else {
            self.add_toast(libadwaita::Toast::new("The library index is unavailable"));
            return;
        };
        let operation = self
            .imp()
            .state
            .borrow()
            .ops
            .add("Looking for originals…".to_string());
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = find_upgrades(&index, source, &roots, &disabled);
            let _ = tx.send_blocking(result);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let result = rx
                .recv()
                .await
                .unwrap_or_else(|_| Err("The search stopped unexpectedly".into()));
            let Some(window) = weak.upgrade() else { return };
            match result {
                Ok(found) => {
                    operation.complete();
                    window.present_upgrades(found);
                }
                Err(error) => {
                    operation.fail(&error);
                    window.add_toast(libadwaita::Toast::new(&format!(
                        "Couldn't look for originals: {error}"
                    )));
                }
            }
        });
    }

    /// Says what was found, lets the user ask to pick originals by hand
    /// for unmatched images, and starts the review.
    fn present_upgrades(&self, found: Vec<Found>) {
        if found.is_empty() {
            self.add_toast(libadwaita::Toast::new(
                "No images to replace originals with",
            ));
            return;
        }
        let paired = found
            .iter()
            .filter(|f| matches!(f.found, Match::Paired(_)))
            .count();
        let choose = found
            .iter()
            .filter(|f| matches!(f.found, Match::Choose(_)))
            .count();
        let not_indexed = found
            .iter()
            .filter(|f| {
                matches!(
                    f.found,
                    Match::Unmatched {
                        why: NoMatch::NotIndexed,
                        ..
                    }
                )
            })
            .count();
        let mut body = format!(
            "Found the original of {} of {}.",
            plural(paired + choose, "image", "images"),
            found.len()
        );
        if choose > 0 {
            body.push_str(&format!(
                " {} several possible originals; you'll choose.",
                if choose == 1 {
                    "1 has".to_string()
                } else {
                    format!("{choose} have")
                }
            ));
        }
        if not_indexed == found.len() {
            body = "These images aren't indexed for visual search yet; try again when Library Overview shows them indexed.".into();
        } else if not_indexed > 0 {
            body.push_str(&format!(
                " {} not indexed for visual search yet.",
                if not_indexed == 1 {
                    "1 is".to_string()
                } else {
                    format!("{not_indexed} are")
                }
            ));
        }
        body.push_str(
            "\n\nYou review each pair next. Nothing changes until you confirm at the end.",
        );
        let alert = libadwaita::AlertDialog::new(Some("Replace Originals"), Some(&body));
        // Unmatched images, each with its reason; those with weak
        // candidates can be reviewed against them on request (never by
        // default).
        let unmatched: Vec<&Found> = found
            .iter()
            .filter(|f| matches!(f.found, Match::Unmatched { .. }))
            .collect();
        let picks: std::rc::Rc<Vec<(PathBuf, gtk4::CheckButton)>> = {
            let list = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            let mut picks = Vec::new();
            for f in &unmatched {
                let Match::Unmatched { why, weak } = &f.found else {
                    continue;
                };
                let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                let label = gtk4::Label::new(Some(&format!("{}: {}", name(&f.new), why.words())));
                label.set_xalign(0.0);
                label.set_hexpand(true);
                label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
                row.append(&label);
                if !weak.is_empty() {
                    let pick = gtk4::CheckButton::with_label("Pick original…");
                    pick.set_tooltip_text(Some(&format!(
                        "Review it beside its {} closest images, labelled weak matches",
                        weak.len()
                    )));
                    row.append(&pick);
                    picks.push((f.new.clone(), pick));
                }
                list.append(&row);
            }
            if !unmatched.is_empty() {
                let scroller = gtk4::ScrolledWindow::new();
                scroller.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
                scroller.set_max_content_height(220);
                scroller.set_propagate_natural_height(true);
                scroller.set_child(Some(&list));
                alert.set_extra_child(Some(&scroller));
            }
            std::rc::Rc::new(picks)
        };
        alert.add_response("cancel", "Cancel");
        let reviewable = paired + choose > 0 || !picks.is_empty();
        if reviewable {
            alert.add_response("review", "Review Pairs");
            alert.set_response_appearance("review", libadwaita::ResponseAppearance::Suggested);
            alert.set_default_response(Some("review"));
        }
        alert.set_close_response("cancel");
        let weak = self.downgrade();
        let found = std::rc::Rc::new(found);
        alert.connect_response(None, move |_, response| {
            if response != "review" {
                return;
            }
            let Some(window) = weak.upgrade() else { return };
            let picked: Vec<PathBuf> = picks
                .iter()
                .filter(|(_, check)| check.is_active())
                .map(|(path, _)| path.clone())
                .collect();
            let items: Vec<ReviewItem> = found
                .iter()
                .filter_map(|f| match &f.found {
                    Match::Paired(c) => Some(ReviewItem {
                        new: f.new.clone(),
                        candidates: vec![c.path.clone()],
                        weak: false,
                    }),
                    Match::Choose(cs) => Some(ReviewItem {
                        new: f.new.clone(),
                        candidates: cs.iter().map(|c| c.path.clone()).collect(),
                        weak: false,
                    }),
                    Match::Unmatched { weak, .. } if picked.contains(&f.new) => Some(ReviewItem {
                        new: f.new.clone(),
                        candidates: weak.iter().map(|c| c.path.clone()).collect(),
                        weak: true,
                    }),
                    Match::Unmatched { .. } => None,
                })
                .collect();
            if items.is_empty() {
                return;
            }
            window.review_upgrades(items);
        });
        alert.present(Some(self));
    }

    /// Opens Compare's replace mode on `items`; the choices come back for
    /// confirmation.
    pub(crate) fn review_upgrades(&self, items: Vec<ReviewItem>) {
        // Compare's survivors don't change the browsing selection here.
        *self.imp().compare_done.borrow_mut() = Some(Box::new(|_| {}));
        let weak = self.downgrade();
        self.show_compare(move |page| {
            page.start_review(items, move |decisions| {
                if let Some(window) = weak.upgrade() {
                    window.confirm_replacements(decisions);
                }
            });
        });
    }

    /// One confirmation for every Replace chosen, then the job.
    fn confirm_replacements(&self, decisions: Vec<(ReviewItem, Decision)>) {
        let mut pairs = Vec::new();
        let mut lines = Vec::new();
        let (mut skipped, mut both) = (0, 0);
        for (item, decision) in decisions {
            match decision {
                Decision::Replace {
                    original,
                    also_trash,
                } => {
                    let stamp = |p: &Path| {
                        std::fs::metadata(p)
                            .ok()
                            .map(|m| crate::fingerprint::file_stamp(&m))
                    };
                    let (Some(new_stamp), Some(original_stamp)) =
                        (stamp(&item.new), stamp(&original))
                    else {
                        continue;
                    };
                    let mut line = format!("{} → replaces {}", name(&item.new), name(&original));
                    if !also_trash.is_empty() {
                        line.push_str(&format!(
                            " (also trashes {})",
                            also_trash
                                .iter()
                                .map(|p| name(p))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    lines.push(line);
                    pairs.push(crate::upgrade::Pair {
                        new: item.new,
                        original,
                        also_trash,
                        new_stamp,
                        original_stamp,
                    });
                }
                Decision::Skip => skipped += 1,
                Decision::KeepBoth => both += 1,
            }
        }
        if pairs.is_empty() {
            self.add_toast(libadwaita::Toast::new("No originals replaced"));
            return;
        }
        let mut body = format!(
            "Each upgrade takes its original's place and name (with its own extension), and keeps the original's buckets. The originals go to the trash; Undo puts everything back.\n\n{}",
            lines.join("\n")
        );
        if skipped + both > 0 {
            body.push_str(&format!("\n\n{} skipped, {} kept both.", skipped, both));
        }
        let title = format!("Replace {}?", plural(pairs.len(), "Original", "Originals"));
        let alert = libadwaita::AlertDialog::new(Some(&title), Some(&body));
        alert.add_response("cancel", "Cancel");
        alert.add_response("replace", "Replace");
        alert.set_response_appearance("replace", libadwaita::ResponseAppearance::Destructive);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");
        let weak = self.downgrade();
        alert.connect_response(None, move |_, response| {
            if response == "replace" {
                if let Some(window) = weak.upgrade() {
                    window.run_replacements(pairs.clone());
                }
            }
        });
        alert.present(Some(self));
    }

    pub(crate) fn run_replacements(&self, pairs: Vec<crate::upgrade::Pair>) {
        let total = pairs.len();
        let title = format!("Replacing {}…", plural(total, "original", "originals"));
        self.queue_file_job(
            crate::file_jobs::Job::Replace(pairs),
            title,
            move |win, report| {
                win.after_replace(&report.replaced, &[]);
                *win.imp().last_replaced.borrow_mut() = report.replaced.clone();
                let mut message = format!(
                    "Replaced {}",
                    plural(report.replaced.len(), "original", "originals")
                );
                if let Some((path, error)) = report.failed.first() {
                    message.push_str(&format!(
                        "; {} not replaced ({}: {error})",
                        report.failed.len(),
                        name(path)
                    ));
                }
                let toast = libadwaita::Toast::new(&message);
                if !report.replaced.is_empty() {
                    toast.set_button_label(Some("Undo"));
                    toast.set_timeout(10);
                    let weak = win.downgrade();
                    let replaced = report.replaced.clone();
                    toast.connect_button_clicked(move |_| {
                        if let Some(window) = weak.upgrade() {
                            window.undo_replacements(replaced.clone());
                        }
                    });
                }
                win.add_toast(toast);
            },
        );
    }

    pub(super) fn undo_replacements(&self, replaced: Vec<crate::upgrade::Replaced>) {
        let title = "Putting originals back…".to_string();
        self.queue_file_job(
            crate::file_jobs::Job::UndoReplace(replaced.clone()),
            title,
            move |win, report| {
                let undone: Vec<crate::upgrade::Replaced> = replaced
                    .iter()
                    .filter(|r| report.done.iter().any(|(p, _)| *p == r.placed))
                    .cloned()
                    .collect();
                win.after_replace(&[], &undone);
                let message = match report.failed.first() {
                    None => format!("Put back {}", plural(undone.len(), "original", "originals")),
                    Some((path, error)) => format!(
                        "Couldn't put back {}: {error}. Find it in the system trash.",
                        name(path)
                    ),
                };
                win.add_toast(libadwaita::Toast::new(&message));
            },
        );
    }

    /// Shows the result in the folder or bucket in view: files that left
    /// drop out, and a folder holding a new or restored file reloads.
    fn after_replace(
        &self,
        replaced: &[crate::upgrade::Replaced],
        undone: &[crate::upgrade::Replaced],
    ) {
        let mut gone: Vec<PathBuf> = Vec::new();
        let mut arrived: Vec<PathBuf> = Vec::new();
        for r in replaced {
            gone.push(r.new_from.clone());
            gone.push(r.original.clone());
            gone.extend(r.also_trashed.iter().cloned());
            arrived.push(r.placed.clone());
        }
        for r in undone {
            gone.push(r.placed.clone());
            arrived.push(r.original.clone());
            arrived.push(r.new_from.clone());
        }
        self.drop_paths_from_view(&gone);
        let folder = match &self.imp().state.borrow().scope {
            ViewScope::Folder(folder) => Some(folder.clone()),
            _ => None,
        };
        if let Some(path) = folder.and_then(|folder| {
            arrived
                .into_iter()
                .find(|p| p.parent() == Some(folder.as_path()))
        }) {
            self.show_created_copy(path);
        }
    }
}

/// The worker side: the upgraded images, the library's current images in
/// enabled folders, and each image's original.
fn find_upgrades(
    index: &crate::library_index::LibraryIndex,
    source: UpgradeSource,
    roots: &[PathBuf],
    disabled: &[PathBuf],
) -> Result<Vec<Found>, String> {
    use crate::config::FolderMode;
    use crate::model::library::collect_library_image_paths_cancellable;
    let library_paths = collect_library_image_paths_cancellable(
        roots,
        FolderMode::DrillDown,
        None,
        disabled,
        &|| false,
    );
    let paths: Vec<PathBuf> = match source {
        UpgradeSource::Paths(paths) => paths,
        UpgradeSource::Folder(folder) => library_paths
            .iter()
            .filter(|p| p.starts_with(&folder))
            .cloned()
            .collect(),
        UpgradeSource::Bucket(id) => crate::buckets::view(index, id, roots, disabled)?.paths,
    };
    let (new, library) = crate::upgrade::fingerprints_for(index, &paths, &library_paths)?;
    let vectors = crate::visual_search::files::locate()
        .ok()
        .and_then(|files| {
            let store = crate::visual_search::store::Store::open(
                &crate::visual_search::store::default_path(),
            )
            .ok()?;
            let key = store.model_key(&files).ok()?;
            store.vectors(&key).ok()
        })
        .unwrap_or_default();
    let read = crate::copies::Readers {
        size_of: &|_| None,
        tiny: &crate::copies::tiny_version,
        canceled: &|| false,
        progress: &|_, _| {},
    };
    Ok(
        crate::upgrade::find_originals(&new, &library, &vectors, &read)
            .into_iter()
            .map(|(new, found)| Found { new, found })
            .collect(),
    )
}
