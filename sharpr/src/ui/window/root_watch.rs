//! Noticing changes under the library roots while Sharpr runs: a folder or an
//! image made outside Sharpr, or a destination folder made in the Move to
//! Folder chooser. File-system events from the roots and the folders the
//! sidebar lists, plus a look when the window gets the focus back, ask for
//! one debounced pass: the sidebar lists the folders again, the fingerprint
//! and visual-search passes are requested, and images that appeared in the
//! open folder are added to it. Roots on a network are left to the focus
//! look, since polling them would cost more than it gives. Nothing here
//! touches a file.
use std::collections::HashMap;
use std::time::Duration;

use super::*;

/// Quiet time after the last event before the pass runs, so a copy of many
/// files or a download that keeps growing is one pass.
const QUIET: Duration = Duration::from_millis(1500);
/// A steady stream of events still gets a pass this often.
const LONGEST_WAIT: Duration = Duration::from_secs(10);
/// A file job knows exactly what it changed, so it need not wait long.
const AFTER_FILE_JOB: Duration = Duration::from_millis(300);
/// Getting the focus back looks again, but not more often than this.
const FOCUS_LOOK_EVERY: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(super) struct RootWatch {
    pub(super) monitors: HashMap<PathBuf, gio::FileMonitor>,
    /// Whether each root is on a local drive, asked of GIO once per root.
    local_roots: HashMap<PathBuf, bool>,
    timer: Option<glib::SourceId>,
    /// When the pass now waiting was first asked for.
    first_asked: Option<Instant>,
    last_pass: Option<Instant>,
    /// The window lost the focus since it last had it.
    away: bool,
}

/// Whether an event on `path` (and `other`, where a file was renamed or moved
/// into view) can add or remove a folder or an image. Hidden names are
/// ignored, as the folder scan skips them; a file that is not an image only
/// counts if it is a folder that just appeared. A removed folder with a dot
/// in its name is not told from a removed file, so it waits for the focus look.
fn is_relevant(
    event: gio::FileMonitorEvent,
    path: &Path,
    other: Option<&Path>,
    is_dir: impl Fn(&Path) -> bool,
) -> bool {
    use gio::FileMonitorEvent as Event;
    let appears = match event {
        Event::Created | Event::MovedIn | Event::Renamed => true,
        Event::Deleted | Event::MovedOut | Event::ChangesDoneHint => false,
        _ => return false,
    };
    let counts = |path: &Path| {
        let hidden = path.file_name().is_none_or(crate::model::walk::is_hidden);
        !hidden && (LibraryManager::is_image(path) || (appears && is_dir(path)))
    };
    counts(path) || other.is_some_and(counts)
}

impl SharprWindow {
    pub(super) fn setup_root_watch(&self) {
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            let weak = self.downgrade();
            sidebar.connect_folders_scanned(move || {
                if let Some(window) = weak.upgrade() {
                    window.sync_root_monitors();
                }
            });
        }
        self.connect_is_active_notify(|window| {
            let mut watch = window.imp().root_watch.borrow_mut();
            if !window.is_active() {
                watch.away = true;
                return;
            }
            let recent = watch
                .last_pass
                .is_some_and(|at| at.elapsed() < FOCUS_LOOK_EVERY);
            if !std::mem::take(&mut watch.away) || recent {
                return;
            }
            drop(watch);
            window.schedule_roots_changed(Duration::from_millis(200));
        });
    }

    /// Asks for the pass after `delay` of quiet, restarting the wait if one
    /// was already asked for, up to `LONGEST_WAIT` since the first ask.
    pub(super) fn schedule_roots_changed(&self, delay: Duration) {
        let mut watch = self.imp().root_watch.borrow_mut();
        let first = *watch.first_asked.get_or_insert_with(Instant::now);
        if let Some(timer) = watch.timer.take() {
            timer.remove();
        }
        let delay = if first.elapsed() >= LONGEST_WAIT {
            Duration::ZERO
        } else {
            delay
        };
        let weak = self.downgrade();
        watch.timer = Some(glib::timeout_add_local_once(delay, move || {
            let Some(window) = weak.upgrade() else { return };
            {
                let mut watch = window.imp().root_watch.borrow_mut();
                watch.timer = None;
                watch.first_asked = None;
                watch.last_pass = Some(Instant::now());
            }
            window.roots_changed();
        }));
    }

    /// The pass itself. The sidebar walks on a worker; the index passes are
    /// the ones the services already coalesce and space out.
    pub(super) fn roots_changed(&self) {
        if let Some(sidebar) = self.imp().sidebar.borrow().clone() {
            sidebar.rescan_folders(self.app_state());
        }
        self.library_scope_changed();
        self.add_images_found_in_open_folder();
    }

    /// Lists the open folder on a worker and adds the images the view does not
    /// have, the way a file job's results are added: no reload, selection and
    /// scheduler untouched.
    fn add_images_found_in_open_folder(&self) {
        let folder = {
            let st = self.imp().state.borrow();
            match &st.scope {
                ViewScope::Folder(folder)
                    if st.library.current_folder.as_deref() == Some(folder.as_path())
                        && !st.library.is_loading_folder() =>
                {
                    folder.clone()
                }
                _ => return,
            }
        };
        let (tx, rx) = async_channel::bounded(1);
        let listing = folder.clone();
        let spawned = std::thread::Builder::new()
            .name("open-folder-listing".into())
            .spawn(move || {
                let images: Vec<PathBuf> = std::fs::read_dir(&listing)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                    .map(|entry| entry.path())
                    .filter(|path| {
                        LibraryManager::is_image(path)
                            && path
                                .file_name()
                                .is_some_and(|name| !crate::model::walk::is_hidden(name))
                    })
                    .collect();
                let _ = tx.send_blocking(images);
            });
        if spawned.is_err() {
            return;
        }
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(listed) = rx.recv().await else { return };
            let Some(window) = weak.upgrade() else { return };
            let new: Vec<PathBuf> = {
                let st = window.imp().state.borrow();
                if st.scope != ViewScope::Folder(folder.clone()) {
                    return;
                }
                listed
                    .into_iter()
                    .filter(|path| st.library.index_of_path(path).is_none())
                    .filter(|path| !window.imp().file_jobs_pending.borrow().contains(path))
                    .collect()
            };
            if !new.is_empty() {
                window.show_new_files(new, |_| {});
            }
        });
    }

    /// Watches the roots and the folders the sidebar lists, and stops
    /// watching the ones it no longer lists. Which roots are on a local drive
    /// is asked of GIO on a worker the first time, since a network share can
    /// be slow to answer.
    fn sync_root_monitors(&self) {
        let Some(sidebar) = self.imp().sidebar.borrow().clone() else {
            return;
        };
        let wanted = sidebar.watchable_folders();
        let roots = self.app_state().borrow().settings.library_roots();
        let unknown: Vec<PathBuf> = {
            let watch = self.imp().root_watch.borrow();
            roots
                .iter()
                .filter(|root| !watch.local_roots.contains_key(*root))
                .cloned()
                .collect()
        };
        if unknown.is_empty() {
            self.apply_monitors(&roots, wanted);
            return;
        }
        let (tx, rx) = async_channel::bounded(1);
        let spawned = std::thread::Builder::new()
            .name("root-locality".into())
            .spawn(move || {
                let answers: Vec<(PathBuf, bool)> = unknown
                    .into_iter()
                    .map(|root| {
                        let local = gio::File::for_path(&root)
                            .query_filesystem_info("filesystem::remote", gio::Cancellable::NONE)
                            .map(|info| !info.boolean("filesystem::remote"))
                            .unwrap_or(false);
                        (root, local)
                    })
                    .collect();
                let _ = tx.send_blocking(answers);
            });
        if spawned.is_err() {
            return;
        }
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(answers) = rx.recv().await else { return };
            let Some(window) = weak.upgrade() else { return };
            window
                .imp()
                .root_watch
                .borrow_mut()
                .local_roots
                .extend(answers);
            // The list may have changed while the worker asked.
            let sidebar = window.imp().sidebar.borrow().clone();
            if let Some(sidebar) = sidebar {
                let roots = window.app_state().borrow().settings.library_roots();
                window.apply_monitors(&roots, sidebar.watchable_folders());
            }
        });
    }

    fn apply_monitors(&self, roots: &[PathBuf], wanted: Vec<PathBuf>) {
        let local = |folder: &PathBuf| {
            let watch = self.imp().root_watch.borrow();
            roots
                .iter()
                .find(|root| folder.starts_with(root))
                .is_some_and(|root| watch.local_roots.get(root).copied().unwrap_or(false))
        };
        let wanted: Vec<PathBuf> = wanted.into_iter().filter(local).collect();
        let mut watch = self.imp().root_watch.borrow_mut();
        watch.monitors.retain(|folder, monitor| {
            let keep = wanted.contains(folder);
            if !keep {
                monitor.cancel();
            }
            keep
        });
        for folder in wanted {
            if watch.monitors.contains_key(&folder) {
                continue;
            }
            let monitor = gio::File::for_path(&folder)
                .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE);
            let Ok(monitor) = monitor else { continue };
            let weak = self.downgrade();
            monitor.connect_changed(move |_, file, other, event| {
                let Some(path) = file.path() else { return };
                let other = other.and_then(|other| other.path());
                if is_relevant(event, &path, other.as_deref(), Path::is_dir) {
                    if let Some(window) = weak.upgrade() {
                        if let Some(service) = window.imp().fingerprints.borrow().as_ref() {
                            service.invalidate();
                        }
                        window.schedule_roots_changed(QUIET);
                    }
                }
            });
            watch.monitors.insert(folder, monitor);
        }
    }

    /// After a Move or Copy to Folder: the destination may be a folder the
    /// sidebar has not listed, which no monitor was watching.
    pub(super) fn destination_changed(&self) {
        if let Some(service) = self.imp().fingerprints.borrow().as_ref() {
            service.invalidate();
        }
        self.schedule_roots_changed(AFTER_FILE_JOB);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gio::FileMonitorEvent as Event;

    fn never(_: &Path) -> bool {
        false
    }

    #[test]
    fn images_and_new_folders_count_and_everything_else_does_not() {
        let is_dir = |path: &Path| path.ends_with("Trip 2024.05");
        let a = |event, path: &str| is_relevant(event, Path::new(path), None, is_dir);
        assert!(a(Event::Created, "/r/new.png"));
        assert!(a(Event::MovedIn, "/r/new.JPG"));
        assert!(a(Event::Deleted, "/r/old.webp"));
        assert!(a(Event::ChangesDoneHint, "/r/download.jpg"));
        assert!(a(Event::Created, "/r/Trip 2024.05"), "a folder appeared");
        assert!(
            !a(Event::Deleted, "/r/Trip 2024.05"),
            "a gone folder looks like a gone file"
        );
        assert!(!a(Event::Created, "/r/notes.txt"));
        assert!(!a(Event::Created, "/r/.hidden.png"), "hidden");
        assert!(!a(Event::Created, "/r/.cache"), "hidden folder");
        assert!(!a(Event::Changed, "/r/new.png"), "still being written");
        assert!(!a(Event::AttributeChanged, "/r/new.png"));
    }

    #[test]
    fn a_rename_counts_when_either_name_does() {
        assert!(is_relevant(
            Event::Renamed,
            Path::new("/r/a.tmp"),
            Some(Path::new("/r/a.png")),
            never
        ));
        assert!(!is_relevant(
            Event::Renamed,
            Path::new("/r/a.tmp"),
            Some(Path::new("/r/b.tmp")),
            never
        ));
    }
}
