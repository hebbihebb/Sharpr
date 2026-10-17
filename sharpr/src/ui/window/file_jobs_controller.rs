//! GTK side of the file-operations worker (`crate::file_jobs`): queues each
//! file action, shows its progress, and applies its result to the view.
//! Files in a queued or running job are pending, so repeating an action on
//! them does nothing; actions on other files wait their turn.
use super::*;
use crate::file_jobs::{Event, Job, Report};
use crate::ui::copies_dialog::images_text;

fn files_text(count: usize) -> String {
    format!("{count} {}", if count == 1 { "file" } else { "files" })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// "Moved 3 files", or "Moved 12 of 40 files, stopped", plus the first
/// failure.
fn summary(verb: &str, report: &Report, total: usize) -> String {
    let done = report.done.len();
    let mut message = if report.cancelled {
        format!("{verb} {done} of {}, stopped", files_text(total))
    } else {
        format!("{verb} {}", files_text(done))
    };
    if let Some((path, error)) = report.failed.first() {
        message.push_str(&format!(
            "; {} failed ({}: {error})",
            report.failed.len(),
            file_name(path)
        ));
    }
    message
}

impl SharprWindow {
    /// Leaves out files already in a queued or running job.
    fn without_pending(&self, paths: Vec<PathBuf>) -> Vec<PathBuf> {
        let pending = self.imp().file_jobs_pending.borrow();
        paths.into_iter().filter(|p| !pending.contains(p)).collect()
    }

    /// Queues `job` behind earlier file jobs. It shows in the activity
    /// indicator; a job with more than one file also gets a toast that counts
    /// and can cancel. `on_done` runs on the GTK thread with the result.
    fn queue_file_job(
        &self,
        job: Job,
        title: String,
        on_done: impl FnOnce(&SharprWindow, Report) + 'static,
    ) {
        let paths = job.paths();
        let total = paths.len();
        self.imp()
            .file_jobs_pending
            .borrow_mut()
            .extend(paths.iter().cloned());
        let ctx = {
            let st = self.imp().state.borrow();
            crate::file_jobs::Context::new(st.library_index.clone(), st.tags.clone())
        };
        let (cancel, events) = self
            .imp()
            .file_jobs
            .borrow_mut()
            .get_or_insert_with(crate::file_jobs::Worker::new)
            .submit(job, ctx);
        let operation = self.imp().state.borrow().ops.add(title.clone());
        let toast = (total > 1).then(|| {
            let toast = libadwaita::Toast::new(&title);
            toast.set_timeout(0);
            toast.set_button_label(Some("Cancel"));
            toast.connect_button_clicked(move |_| {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed)
            });
            self.add_toast(toast.clone());
            toast
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let report = loop {
                match events.recv().await {
                    Ok(Event::Progress { done, total }) => {
                        operation.progress(Some(done as f32 / total.max(1) as f32));
                        if let Some(toast) = &toast {
                            toast.set_title(&format!("{title} ({done} of {total})"));
                        }
                    }
                    Ok(Event::Finished(report)) => break Some(report),
                    Err(_) => break None,
                }
            };
            if let Some(toast) = &toast {
                toast.dismiss();
            }
            let Some(win) = weak.upgrade() else { return };
            {
                let mut pending = win.imp().file_jobs_pending.borrow_mut();
                for path in &paths {
                    pending.remove(path);
                }
            }
            match report {
                Some(report) => {
                    operation.complete();
                    on_done(&win, report);
                }
                None => operation.fail("File worker stopped"),
            }
        });
    }

    /// Lets the running job finish its current file, skips the rest, and
    /// waits for the worker. Called when the window closes.
    pub(super) fn stop_file_jobs(&self) {
        if let Some(mut worker) = self.imp().file_jobs.take() {
            worker.shutdown();
        }
    }

    /// Drops `removed` from the view. The selection moves on only if the
    /// image in view was one of them.
    fn drop_paths_from_view(&self, removed: &[PathBuf]) {
        if removed.is_empty() {
            return;
        }
        let (Some(filmstrip), Some(viewer)) = (
            self.imp().filmstrip.borrow().clone(),
            self.imp().viewer.borrow().clone(),
        ) else {
            return;
        };
        let state = self.imp().state.clone();
        let selected = state.borrow().library.selected_entry().map(|e| e.path());
        {
            let mut st = state.borrow_mut();
            for path in removed {
                st.library.remove_path(path);
                remove_path_from_action_selection(&mut st, path);
            }
        }
        if selected.is_some_and(|path| !removed.contains(&path)) {
            filmstrip.refresh_multi_selection_visuals();
            return;
        }
        let new_count = state.borrow().library.image_count();
        if new_count == 0 {
            viewer.clear();
            filmstrip.refresh_multi_selection_visuals();
        } else {
            let index = state.borrow().library.selected_index.unwrap_or(0);
            let new_index = index.min(new_count - 1);
            filmstrip.navigate_to(new_index);
            let next_path = state
                .borrow()
                .library
                .entry_at(new_index)
                .map(|e: ImageEntry| e.path());
            if let Some(next_path) = next_path {
                filmstrip.set_action_selection_to_path(&next_path);
            }
        }
    }

    /// Moves `paths` to the system trash and drops them from the view, then
    /// offers Undo (D25). Tags are kept, so restoring a file from the trash
    /// restores its buckets. Files that cannot be trashed are reported instead
    /// of being skipped silently.
    pub(crate) fn trash_paths(&self, paths: Vec<PathBuf>) {
        let paths = self.without_pending(paths);
        if paths.is_empty() {
            return;
        }
        let total = paths.len();
        let title = format!("Moving {} to the trash…", images_text(total));
        self.queue_file_job(Job::Trash(paths), title, move |win, report| {
            let trashed: Vec<PathBuf> = report.done.iter().map(|(p, _)| p.clone()).collect();
            win.drop_paths_from_view(&trashed);
            if !trashed.is_empty() {
                let mut message = format!("Moved {} to the trash", images_text(trashed.len()));
                if report.cancelled {
                    message.push_str(&format!(" ({} of {total}, stopped)", trashed.len()));
                }
                let toast = libadwaita::Toast::new(&message);
                toast.set_button_label(Some("Undo"));
                toast.set_timeout(10);
                let weak = win.downgrade();
                toast.connect_button_clicked(move |_| {
                    if let Some(window) = weak.upgrade() {
                        window.restore_trashed(trashed.clone());
                    }
                });
                win.add_toast(toast);
            }
            if let Some((path, error)) = report.failed.first() {
                crate::bench_warn!(
                    "file.trash.fail",
                    serde_json::json!({
                        "count": report.failed.len(),
                        "path": path.display().to_string(),
                        "error": error,
                    })
                );
                let message = if report.failed.len() == 1 {
                    format!("Could not move “{}” to the trash: {error}", file_name(path))
                } else {
                    format!(
                        "Could not move {} files to the trash: {error}",
                        report.failed.len()
                    )
                };
                win.add_toast(libadwaita::Toast::new(&message));
            }
        });
    }

    /// Undo for the filmstrip's trash (D25): puts `paths` back, never over a
    /// file now at the same name, and reopens their folder if it is the one
    /// in view. Those that can't return stay in the trash.
    pub(crate) fn restore_trashed(&self, paths: Vec<PathBuf>) {
        let title = format!("Restoring {}…", images_text(paths.len()));
        self.queue_file_job(Job::Restore(paths), title, |win, report| {
            let restored: Vec<PathBuf> = report.done.iter().map(|(p, _)| p.clone()).collect();
            let failed = report.failed;
            crate::bench_event!(
                "file.trash.undo",
                serde_json::json!({"restored": restored.len(), "failed": failed.len()})
            );
            let in_view = {
                let st = win.imp().state.borrow();
                match &st.scope {
                    ViewScope::Folder(folder) => restored
                        .iter()
                        .find(|p| p.parent() == Some(folder.as_path()))
                        .cloned(),
                    _ => None,
                }
            };
            if let Some(path) = in_view {
                // Reopens the folder with the restored image selected.
                win.show_created_copy(path);
            }
            let message = match failed.first() {
                None => format!("Restored {}", images_text(restored.len())),
                Some((path, error)) => {
                    let mut message = format!("Couldn’t restore “{}”: {error}", file_name(path));
                    if failed.len() > 1 {
                        message.push_str(&format!(" ({} not restored)", failed.len()));
                    }
                    message.push_str(". Find it in the system trash.");
                    message
                }
            };
            win.add_toast(libadwaita::Toast::new(&message));
        });
    }

    /// Trashes the copies confirmed in "Find Copies…" (Phase 7), after
    /// checking each against what was reviewed (`copies::verify`), and drops
    /// them from the view.
    pub(crate) fn trash_copies(&self, groups: Vec<crate::copies::TrashGroup>) {
        let pending = self.imp().file_jobs_pending.borrow().clone();
        let groups: Vec<_> = groups
            .into_iter()
            .map(|mut group| {
                group.extras.retain(|(path, _)| !pending.contains(path));
                group
            })
            .filter(|group| !group.extras.is_empty())
            .collect();
        let job = Job::TrashCopies(groups);
        let total = job.paths().len();
        if total == 0 {
            return;
        }
        let title = "Moving copies to the trash…".to_string();
        self.queue_file_job(job, title, move |win, report| {
            let trashed: Vec<PathBuf> = report.done.iter().map(|(p, _)| p.clone()).collect();
            win.drop_paths_from_view(&trashed);
            if let Some(filmstrip) = win.imp().filmstrip.borrow().as_ref() {
                filmstrip.refresh_virtual();
                filmstrip.refresh_multi_selection_visuals();
            }
            crate::bench_event!(
                "copies.trash",
                serde_json::json!({"trashed": trashed.len(), "failed": report.failed.len()})
            );
            let mut message = format!("Moved {} to the trash", files_text(trashed.len()));
            if report.cancelled {
                message.push_str(&format!(" ({} of {total}, stopped)", trashed.len()));
            }
            if let Some((path, error)) = report.failed.first() {
                message.push_str(&format!(
                    "; {} couldn't be moved ({}: {error})",
                    report.failed.len(),
                    file_name(path)
                ));
            }
            win.add_toast(libadwaita::Toast::new(&message));
        });
    }

    /// Move to Folder…: moves `paths` into `dest_dir` under free names,
    /// carrying their buckets and tags, and drops them from the view.
    pub(super) fn move_to_folder(&self, paths: Vec<PathBuf>, dest_dir: PathBuf) {
        let paths = self.without_pending(paths);
        if paths.is_empty() {
            return;
        }
        let total = paths.len();
        let title = format!("Moving {}…", files_text(total));
        self.queue_file_job(Job::Move { paths, dest_dir }, title, move |win, report| {
            let moved: Vec<PathBuf> = report.done.iter().map(|(p, _)| p.clone()).collect();
            win.drop_paths_from_view(&moved);
            win.add_toast(libadwaita::Toast::new(&summary("Moved", &report, total)));
        });
    }

    /// Copy to Folder…: copies `paths` into `dest_dir`; sources are only read.
    pub(super) fn copy_to_folder(&self, paths: Vec<PathBuf>, dest_dir: PathBuf) {
        let paths = self.without_pending(paths);
        if paths.is_empty() {
            return;
        }
        let total = paths.len();
        let title = format!("Copying {}…", files_text(total));
        self.queue_file_job(Job::Copy { paths, dest_dir }, title, move |win, report| {
            win.add_toast(libadwaita::Toast::new(&summary("Copied", &report, total)));
        });
    }

    /// Rename…: renames `path` in place, carrying its buckets and tags.
    pub(super) fn rename_file(&self, path: PathBuf, new_name: String) {
        if self.without_pending(vec![path.clone()]).is_empty() {
            return;
        }
        let title = format!("Renaming “{}”…", file_name(&path));
        self.queue_file_job(Job::Rename { path, new_name }, title, |win, report| {
            let renamed: Vec<PathBuf> = report.done.iter().map(|(p, _)| p.clone()).collect();
            win.drop_paths_from_view(&renamed);
            let message = match report.failed.first() {
                Some((_, error)) => error.clone(),
                None if report.cancelled => "Rename stopped".to_string(),
                None => "File renamed".to_string(),
            };
            win.add_toast(libadwaita::Toast::new(&message));
        });
    }

    /// Duplicate: copies `path` beside itself under a free name, and adds the
    /// pair to the Edits buckets.
    pub(super) fn duplicate_file(&self, path: PathBuf) {
        if self.without_pending(vec![path.clone()]).is_empty() {
            return;
        }
        let title = format!("Duplicating “{}”…", file_name(&path));
        self.queue_file_job(Job::Duplicate(path), title, |win, report| {
            let message = match (report.done.first(), report.failed.first()) {
                (Some((source, Some(dest))), _) => {
                    win.add_edit_copies(
                        crate::buckets::EditKind::Duplicated,
                        vec![(source.clone(), dest.clone())],
                    );
                    format!("Duplicated as \"{}\"", file_name(dest))
                }
                (_, Some((_, error))) => format!("Duplicate failed: {error}"),
                _ => "Duplicate stopped".to_string(),
            };
            win.add_toast(libadwaita::Toast::new(&message));
        });
    }
}
