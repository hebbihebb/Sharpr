//! The file-operations worker (ROADMAP Work Order item 6). Trash, Undo,
//! Move to Folder, Rename, Copy to Folder, Duplicate and Find Copies' trash
//! run on one background thread, one job at a time in the order asked, so
//! the GTK thread never waits on the disk or SQLite, and an Undo can't
//! overtake the trash it undoes.
//!
//! Each file finishes or fails on its own: a failure moves on to the next
//! file, and `LibraryIndex::relocate_file` rolls back its own file and
//! database changes. Cancelling (or closing the window) stops between files,
//! never inside one.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gtk4::prelude::FileExt;

use crate::library_index::LibraryIndex;
use crate::tags::TagDatabase;

pub enum Job {
    Trash(Vec<PathBuf>),
    /// Undo of a trash (D25).
    Restore(Vec<PathBuf>),
    Move {
        paths: Vec<PathBuf>,
        dest_dir: PathBuf,
    },
    Rename {
        path: PathBuf,
        new_name: String,
    },
    Copy {
        paths: Vec<PathBuf>,
        dest_dir: PathBuf,
    },
    Duplicate(PathBuf),
    /// Find Copies' confirmed trash, checked against the review first.
    TrashCopies(Vec<crate::copies::TrashGroup>),
}

impl Job {
    /// The files this job changes or reads, in order.
    pub fn paths(&self) -> Vec<PathBuf> {
        match self {
            Job::Trash(paths)
            | Job::Restore(paths)
            | Job::Move { paths, .. }
            | Job::Copy { paths, .. } => paths.clone(),
            Job::Rename { path, .. } | Job::Duplicate(path) => vec![path.clone()],
            Job::TrashCopies(groups) => groups
                .iter()
                .flat_map(|group| group.extras.iter().map(|(path, _)| path.clone()))
                .collect(),
        }
    }
}

/// What a job did, file by file.
#[derive(Debug, Default)]
pub struct Report {
    /// Each finished source, with its new path for move, rename, copy and
    /// duplicate.
    pub done: Vec<(PathBuf, Option<PathBuf>)>,
    pub failed: Vec<(PathBuf, String)>,
    /// Stopped before every file was tried; the untried ones are unchanged.
    pub cancelled: bool,
}

pub enum Event {
    Progress { done: usize, total: usize },
    Finished(Report),
}

type FileFn = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

/// The stores and system calls a job uses. Tests swap the trash for a
/// temporary directory.
#[derive(Clone)]
pub struct Context {
    pub index: Option<Arc<LibraryIndex>>,
    pub tags: Option<Arc<TagDatabase>>,
    pub trash: FileFn,
    pub restore: FileFn,
}

impl Context {
    pub fn new(index: Option<Arc<LibraryIndex>>, tags: Option<Arc<TagDatabase>>) -> Self {
        Self {
            index,
            tags,
            trash: Arc::new(|path: &Path| {
                gtk4::gio::File::for_path(path)
                    .trash(None::<&gtk4::gio::Cancellable>)
                    .map_err(|e| e.to_string())
            }),
            restore: Arc::new(crate::file_ops::restore_from_trash),
        }
    }
}

/// Runs `job` file by file, calling `progress` after each one and checking
/// `cancelled` before each one.
pub fn run(
    job: Job,
    ctx: &Context,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> Report {
    let mut report = Report::default();
    let total = job.paths().len();
    let mut step = |report: &mut Report, path: &Path, outcome: Result<Option<PathBuf>, String>| {
        match outcome {
            Ok(dest) => report.done.push((path.to_path_buf(), dest)),
            Err(error) => report.failed.push((path.to_path_buf(), error)),
        }
        progress(report.done.len() + report.failed.len(), total);
    };
    let stores = || match (&ctx.index, &ctx.tags) {
        (Some(index), Some(tags)) => Ok((index.clone(), tags.clone())),
        _ => Err("bucket or tag database is unavailable".to_string()),
    };
    match job {
        Job::Trash(paths) => {
            for path in paths {
                if cancelled() {
                    report.cancelled = true;
                    break;
                }
                let outcome = (ctx.trash)(&path).map(|()| None);
                step(&mut report, &path, outcome);
            }
        }
        Job::Restore(paths) => {
            for path in paths {
                if cancelled() {
                    report.cancelled = true;
                    break;
                }
                let outcome = (ctx.restore)(&path).map(|()| None);
                step(&mut report, &path, outcome);
            }
        }
        Job::Move { paths, dest_dir } => {
            for path in paths {
                if cancelled() {
                    report.cancelled = true;
                    break;
                }
                let outcome = stores()
                    .and_then(|(index, tags)| move_into(&index, &tags, &path, &dest_dir).map(Some));
                step(&mut report, &path, outcome);
            }
        }
        Job::Rename { path, new_name } => {
            if cancelled() {
                report.cancelled = true;
            } else {
                let outcome =
                    crate::file_ops::rename_destination(&path, &new_name).and_then(|dest| {
                        let (index, tags) = stores()?;
                        index.relocate_file(&path, &dest, &tags)?;
                        Ok(Some(dest))
                    });
                step(&mut report, &path, outcome);
            }
        }
        Job::Copy { paths, dest_dir } => {
            for path in paths {
                if cancelled() {
                    report.cancelled = true;
                    break;
                }
                let outcome = crate::file_ops::copy_to_folder(&path, &dest_dir).map(Some);
                step(&mut report, &path, outcome);
            }
        }
        Job::Duplicate(path) => {
            if cancelled() {
                report.cancelled = true;
            } else {
                let outcome = crate::file_ops::duplicate_file(&path).map(Some);
                step(&mut report, &path, outcome);
            }
        }
        Job::TrashCopies(groups) => {
            'groups: for group in &groups {
                for verdict in crate::copies::verify(group) {
                    if cancelled() {
                        report.cancelled = true;
                        break 'groups;
                    }
                    match verdict {
                        crate::copies::Verdict::Trash(path) => {
                            let outcome = (ctx.trash)(&path).map(|()| None);
                            step(&mut report, &path, outcome);
                        }
                        crate::copies::Verdict::Skip(path, reason) => {
                            step(&mut report, &path, Err(reason.to_string()))
                        }
                    }
                }
            }
        }
    }
    report
}

/// Moves `src` into `dest_dir` under the first free name, carrying buckets
/// and tags. A name whose missing file still has tags or bucket membership
/// counts as taken.
fn move_into(
    index: &LibraryIndex,
    tags: &TagDatabase,
    src: &Path,
    dest_dir: &Path,
) -> Result<PathBuf, String> {
    let taken = |p: &Path| {
        p.symlink_metadata().is_ok()
            || !tags.tags_for_path(p).is_empty()
            || index.retains_bucket_path(p).unwrap_or(false)
    };
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    // Re-pick a free name if one appears between choosing and moving.
    let mut result = Err("No unused name available".to_string());
    for _ in 0..5 {
        let dest = crate::file_ops::next_available_path(dest_dir, &stem, ext, taken);
        result = index.relocate_file(src, &dest, tags).map(|()| dest.clone());
        if result.is_ok() || !taken(&dest) {
            break;
        }
    }
    result
}

struct Queued {
    job: Job,
    ctx: Context,
    cancel: Arc<AtomicBool>,
    reply: async_channel::Sender<Event>,
}

/// The single worker thread. Dropping it (or `shutdown`) stops the running
/// job after its current file, skips queued ones, and waits for the thread.
pub struct Worker {
    tx: Option<std::sync::mpsc::Sender<Queued>>,
    thread: Option<std::thread::JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl Worker {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Queued>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let stop = shutdown.clone();
        let thread = std::thread::Builder::new()
            .name("file-jobs".into())
            .spawn(move || {
                while let Ok(queued) = rx.recv() {
                    let Queued {
                        job,
                        ctx,
                        cancel,
                        reply,
                    } = queued;
                    let cancelled =
                        || cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed);
                    let report = run(job, &ctx, &cancelled, &mut |done, total| {
                        let _ = reply.send_blocking(Event::Progress { done, total });
                    });
                    let _ = reply.send_blocking(Event::Finished(report));
                }
            })
            .expect("spawn file-jobs thread");
        Self {
            tx: Some(tx),
            thread: Some(thread),
            shutdown,
        }
    }

    /// Queues `job` behind any earlier ones. Returns its cancel flag and its
    /// events; the last event is always `Finished` unless the worker stops.
    pub fn submit(
        &self,
        job: Job,
        ctx: Context,
    ) -> (Arc<AtomicBool>, async_channel::Receiver<Event>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let (reply, events) = async_channel::unbounded();
        if let Some(tx) = &self.tx {
            let _ = tx.send(Queued {
                job,
                ctx,
                cancel: cancel.clone(),
                reply,
            });
        }
        (cancel, events)
    }

    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fixture {
        root: PathBuf,
        ctx: Context,
        /// Trashed files land here, by name.
        bin: PathBuf,
    }

    fn fixture(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "sharpr-file-jobs-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let index = Arc::new(LibraryIndex::open_in_memory().unwrap());
        let tags = Arc::new(TagDatabase::open_in_memory().unwrap());
        crate::buckets::migrate(&index, &tags, &root.join("backups")).unwrap();
        let (trash_bin, restore_bin) = (bin.clone(), bin.clone());
        let ctx = Context {
            index: Some(index),
            tags: Some(tags),
            trash: Arc::new(move |path| {
                std::fs::rename(path, trash_bin.join(path.file_name().unwrap()))
                    .map_err(|e| e.to_string())
            }),
            restore: Arc::new(move |path| {
                if path.exists() {
                    return Err("another file now has its name".into());
                }
                std::fs::rename(restore_bin.join(path.file_name().unwrap()), path)
                    .map_err(|e| e.to_string())
            }),
        };
        Fixture { root, ctx, bin }
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn run_all(job: Job, ctx: &Context) -> Report {
        run(job, ctx, &|| false, &mut |_, _| {})
    }

    #[test]
    fn a_failed_move_leaves_that_file_and_its_buckets_alone_and_moves_the_rest() {
        let f = fixture("move");
        let index = f.ctx.index.clone().unwrap();
        let tags = f.ctx.tags.clone().unwrap();
        let (a, b, c) = (
            f.root.join("a.png"),
            f.root.join("b.png"),
            f.root.join("c.png"),
        );
        write(&a, b"a original");
        write(&c, b"c original");
        // b is missing, so its move fails between a and c.
        let bucket = index
            .create_collection("test", None, "Kept", &[], None, None)
            .unwrap()
            .id;
        index
            .add_bucket_members(bucket, &[(a.clone(), None), (b.clone(), None)])
            .unwrap();
        tags.add_tag(&c, "legacy");
        let dest = f.root.join("dest");
        std::fs::create_dir_all(&dest).unwrap();

        let report = run_all(
            Job::Move {
                paths: vec![a.clone(), b.clone(), c.clone()],
                dest_dir: dest.clone(),
            },
            &f.ctx,
        );

        assert_eq!(
            report.done,
            vec![
                (a.clone(), Some(dest.join("a.png"))),
                (c.clone(), Some(dest.join("c.png")))
            ]
        );
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, b);
        assert!(!report.cancelled);
        assert_eq!(std::fs::read(dest.join("a.png")).unwrap(), b"a original");
        assert_eq!(std::fs::read(dest.join("c.png")).unwrap(), b"c original");
        let members: Vec<PathBuf> = index
            .bucket_members(&[bucket])
            .unwrap()
            .into_iter()
            .map(|row| row.last_path)
            .collect();
        assert!(members.contains(&dest.join("a.png")));
        assert!(
            members.contains(&b),
            "the failed file keeps its bucket path"
        );
        assert_eq!(tags.tags_for_path(&dest.join("c.png")), vec!["legacy"]);
    }

    #[test]
    fn cancelling_stops_between_files_and_leaves_the_rest_untouched() {
        let f = fixture("cancel");
        let paths: Vec<PathBuf> = (0..4).map(|i| f.root.join(format!("{i}.png"))).collect();
        for path in &paths {
            write(path, path.to_string_lossy().as_bytes());
        }
        let done = Mutex::new(0);
        let report = run(
            Job::Trash(paths.clone()),
            &f.ctx,
            &|| *done.lock().unwrap() >= 2,
            &mut |finished, total| {
                assert_eq!(total, 4);
                *done.lock().unwrap() = finished;
            },
        );
        assert!(report.cancelled);
        assert_eq!(report.done.len(), 2);
        assert!(report.failed.is_empty());
        assert!(!paths[0].exists() && !paths[1].exists());
        for path in &paths[2..] {
            assert_eq!(
                std::fs::read(path).unwrap(),
                path.to_string_lossy().as_bytes()
            );
        }
    }

    #[test]
    fn rename_never_takes_an_existing_name() {
        let f = fixture("rename");
        let (a, b) = (f.root.join("a.png"), f.root.join("b.png"));
        write(&a, b"a original");
        write(&b, b"b original");
        let report = run_all(
            Job::Rename {
                path: a.clone(),
                new_name: "b.png".into(),
            },
            &f.ctx,
        );
        assert!(report.done.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert_eq!(std::fs::read(&a).unwrap(), b"a original");
        assert_eq!(std::fs::read(&b).unwrap(), b"b original");

        let report = run_all(
            Job::Rename {
                path: a.clone(),
                new_name: "c.png".into(),
            },
            &f.ctx,
        );
        assert_eq!(report.done, vec![(a.clone(), Some(f.root.join("c.png")))]);
        assert_eq!(std::fs::read(f.root.join("c.png")).unwrap(), b"a original");
    }

    #[test]
    fn copies_and_duplicates_leave_the_source_unchanged() {
        let f = fixture("copy");
        let a = f.root.join("a.png");
        write(&a, b"a original");
        let dest = f.root.join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        write(&dest.join("a.png"), b"already there");

        let report = run_all(
            Job::Copy {
                paths: vec![a.clone()],
                dest_dir: dest.clone(),
            },
            &f.ctx,
        );
        assert_eq!(
            report.done,
            vec![(a.clone(), Some(dest.join("a (copy).png")))]
        );
        assert_eq!(std::fs::read(dest.join("a.png")).unwrap(), b"already there");
        assert_eq!(
            std::fs::read(dest.join("a (copy).png")).unwrap(),
            b"a original"
        );

        let report = run_all(Job::Duplicate(a.clone()), &f.ctx);
        assert_eq!(
            report.done,
            vec![(a.clone(), Some(f.root.join("a (copy).png")))]
        );
        assert_eq!(std::fs::read(&a).unwrap(), b"a original");
    }

    #[test]
    fn the_worker_runs_jobs_in_order_so_undo_follows_its_trash() {
        let f = fixture("order");
        let a = f.root.join("a.png");
        write(&a, b"a original");
        let worker = Worker::new();
        let (_, trash) = worker.submit(Job::Trash(vec![a.clone()]), f.ctx.clone());
        let (_, restore) = worker.submit(Job::Restore(vec![a.clone()]), f.ctx.clone());
        let finished = |events: async_channel::Receiver<Event>| loop {
            if let Event::Finished(report) = events.recv_blocking().unwrap() {
                return report;
            }
        };
        assert_eq!(finished(trash).done.len(), 1);
        assert_eq!(finished(restore).done.len(), 1);
        assert_eq!(std::fs::read(&a).unwrap(), b"a original");
        assert!(std::fs::read_dir(&f.bin).unwrap().next().is_none());
    }

    #[test]
    fn find_copies_trash_skips_files_changed_since_the_review() {
        let f = fixture("copies");
        let (keep, extra, changed) = (
            f.root.join("keep.png"),
            f.root.join("extra.png"),
            f.root.join("changed.png"),
        );
        for path in [&keep, &extra, &changed] {
            write(path, b"same");
        }
        let stamp =
            |p: &Path| crate::fingerprint::file_stamp(&std::fs::symlink_metadata(p).unwrap());
        let group = crate::copies::TrashGroup {
            kept: vec![(keep.clone(), stamp(&keep))],
            extras: vec![
                (extra.clone(), stamp(&extra)),
                (changed.clone(), stamp(&changed)),
            ],
        };
        write(&changed, b"edited after the review");
        let report = run_all(Job::TrashCopies(vec![group]), &f.ctx);
        assert_eq!(report.done, vec![(extra.clone(), None)]);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(std::fs::read(&changed).unwrap(), b"edited after the review");
        assert_eq!(std::fs::read(&keep).unwrap(), b"same");
    }
}
