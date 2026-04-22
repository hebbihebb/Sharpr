use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;
use sha2::{Digest, Sha256};

use std::path::PathBuf;

use crate::config::AppSettings;
use crate::duplicates::phash;
use crate::library_index::{BasicImageInfo, LibraryIndex};
use crate::model::library::{CachedImageData, RawImageEntry, SortOrder};
use crate::model::{ImageEntry, LibraryManager};
use crate::tags::smart::SmartModel;
use crate::thumbnails::ThumbnailWorker;
use crate::ui::filmstrip::FilmstripPane;
use crate::ui::ops_indicator::OpsIndicator;
use crate::ui::preferences::build_preferences_window;
use crate::ui::sidebar::SidebarPane;
use crate::ui::tag_browser::TagBrowser;
use crate::ui::viewer::{ViewerPane, ZoomMode};
use crate::upscale::{
    downloader::{self, DownloadEvent},
    OnnxUpscaleModel, UpscaleBackendKind, UpscaleDetector, UpscaleModel,
};

// ---------------------------------------------------------------------------
// Shared application state (main thread only, Rc<RefCell<>>)
// ---------------------------------------------------------------------------

pub struct AppState {
    pub library: LibraryManager,
    pub settings: AppSettings,
    pub sort_order: SortOrder,
    pub library_index: Option<Arc<LibraryIndex>>,
    pub library_index_error: Option<String>,
    pub tags: Option<Arc<crate::tags::TagDatabase>>,
    pub smart_tagger: Option<Arc<crate::tags::smart::LocalTagger>>,
    /// Cached path to the active Vulkan upscaler binary after successful detection.
    pub upscale_binary: Option<PathBuf>,
    pub ops: crate::ops::queue::OpQueue,
    /// Paths additionally highlighted by Ctrl/Shift-click for bulk collection actions.
    pub selected_paths: HashSet<PathBuf>,
    /// ID of the collection currently loaded as the virtual view, if any.
    pub active_collection: Option<i64>,
    /// Folders disabled by the user. Images under these paths must not be indexed or shown.
    pub disabled_folders: Vec<PathBuf>,
}

/// Which virtual content source is currently displayed in the filmstrip.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub enum VirtualSource {
    Duplicates,
    Search,
    Quality(crate::quality::QualityClass),
    Collection(i64),
}

#[derive(Clone)]
struct PrefetchRequest {
    path: PathBuf,
    index: u32,
    direction: i32,
    distance: u32,
}

struct PrefetchResult {
    request: PrefetchRequest,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
}

struct ThumbnailOpState {
    total: u32,
    received: u32,
    handle: crate::ops::queue::OpHandle,
}

struct MetadataIndexResult {
    path: PathBuf,
    width: u32,
    height: u32,
}

struct VirtualMetadataResult {
    path: PathBuf,
    cached: CachedImageData,
}

enum FolderOpenResult {
    /// Immediately-available rows from the DB before filesystem reconciliation.
    Cached {
        rows: Vec<crate::library_index::IndexedImage>,
    },
    /// Final reconciled rows after filesystem scan + DB upsert.
    Indexed {
        rows: Vec<crate::library_index::IndexedImage>,
        metadata_pending: Vec<BasicImageInfo>,
        stale_removed: usize,
        basic_count: usize,
    },
    Raw {
        entries: Vec<RawImageEntry>,
        index_error: Option<String>,
    },
}

fn trigger_prefetch(state: &Rc<RefCell<AppState>>, index: u32) {
    let count = state.borrow().library.image_count();
    // Create a channel to receive decoded bytes back on the main thread.
    let (tx, rx) = async_channel::unbounded::<PrefetchResult>();

    for delta in [-1i32, 1i32] {
        queue_prefetch(state, &tx, count, index as i32 + delta, delta, 1);
    }

    // Drain results onto the main thread (channel closes when both senders drop).
    let state_drain = state.clone();
    let tx_drain = tx.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(result) = rx.recv().await {
            let PrefetchResult {
                request,
                bytes,
                width,
                height,
            } = result;
            {
                state_drain.borrow_mut().library.insert_prefetch(
                    request.path.clone(),
                    bytes,
                    width,
                    height,
                );
            }

            if request.distance < 3 {
                let next_index = request.index as i32 + request.direction;
                queue_prefetch(
                    &state_drain,
                    &tx_drain,
                    count,
                    next_index,
                    request.direction,
                    request.distance + 1,
                );
            }
        }
    });
}

fn path_is_disabled(path: &std::path::Path, disabled_folders: &[PathBuf]) -> bool {
    disabled_folders
        .iter()
        .any(|folder| path.starts_with(folder))
}

fn queue_prefetch(
    state: &Rc<RefCell<AppState>>,
    tx: &async_channel::Sender<PrefetchResult>,
    count: u32,
    index: i32,
    direction: i32,
    distance: u32,
) {
    if index < 0 || index >= count as i32 {
        return;
    }

    let path = {
        let state_ref = state.borrow();
        state_ref
            .library
            .entry_at(index as u32)
            .map(|e: ImageEntry| e.path())
    };
    let Some(path) = path else { return };

    {
        let mut state_ref = state.borrow_mut();
        if state_ref.library.prefetch_pending(&path) {
            return;
        }
        state_ref.library.mark_prefetch_in_flight(path.clone());
    }

    let request = PrefetchRequest {
        path,
        index: index as u32,
        direction,
        distance,
    };
    let tx = tx.clone();
    rayon::spawn(move || {
        if let Some((bytes, width, height)) = prefetch_decode(&request.path) {
            let _ = tx.send_blocking(PrefetchResult {
                request,
                bytes,
                width,
                height,
            });
        }
    });
}

fn prefetch_decode(path: &std::path::Path) -> Option<(Vec<u8>, u32, u32)> {
    let file = std::fs::File::open(path).ok()?;
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;
    let rgba = img.into_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some((rgba.into_raw(), w, h))
}

fn start_metadata_indexer(
    index: Arc<LibraryIndex>,
    folder: PathBuf,
    pending: Vec<BasicImageInfo>,
    state: Rc<RefCell<AppState>>,
) {
    crate::bench_event!(
        "index.metadata.queue",
        serde_json::json!({
            "folder": folder.display().to_string(),
            "count": pending.len(),
        }),
    );
    if pending.is_empty() {
        return;
    }

    let (tx, rx) = async_channel::unbounded::<MetadataIndexResult>();
    let worker_folder = folder.clone();
    rayon::spawn(move || {
        let started = Instant::now();
        if index.is_folder_ignored(&worker_folder).unwrap_or(false) {
            crate::bench_event!(
                "index.metadata.skip",
                serde_json::json!({
                    "folder": worker_folder.display().to_string(),
                    "reason": "disabled_folder",
                }),
            );
            return;
        }
        let total = pending.len();
        let mut completed = 0usize;
        for info in pending {
            match image::image_dimensions(&info.path) {
                Ok((width, height)) => {
                    let quality = crate::quality::scorer::score_file_info(
                        Some((width, height)),
                        info.file_size,
                        &info.extension,
                    );
                    if index
                        .update_image_metadata(&info.path, width, height, quality.class)
                        .is_ok()
                    {
                        completed += 1;
                        let _ = tx.send_blocking(MetadataIndexResult {
                            path: info.path,
                            width,
                            height,
                        });
                    }
                }
                Err(err) => {
                    let _ = index.mark_image_error(&info.path, &err.to_string());
                }
            }
        }
        crate::bench_event!(
            "index.metadata.finish",
            serde_json::json!({
                "folder": worker_folder.display().to_string(),
                "completed": completed,
                "total": total,
                "duration_ms": crate::bench::duration_ms(started),
            }),
        );
    });

    glib::MainContext::default().spawn_local(async move {
        while let Ok(result) = rx.recv().await {
            let mut st = state.borrow_mut();
            if st.library.current_folder.as_deref() == Some(folder.as_path()) {
                st.library
                    .update_entry_metadata(&result.path, result.width, result.height);
            }
        }
    });
}

fn load_virtual_async(state: &Rc<RefCell<AppState>>, paths: &[PathBuf]) {
    load_virtual_async_with_collection(state, paths, None);
}

fn load_collection_async(state: &Rc<RefCell<AppState>>, paths: &[PathBuf], collection_id: i64) {
    load_virtual_async_with_collection(state, paths, Some(collection_id));
}

fn load_virtual_async_with_collection(
    state: &Rc<RefCell<AppState>>,
    paths: &[PathBuf],
    active_collection: Option<i64>,
) {
    let paths: Vec<PathBuf> = {
        let st = state.borrow();
        paths
            .iter()
            .filter(|path| !path_is_disabled(path, &st.disabled_folders))
            .cloned()
            .collect()
    };
    let pending = {
        let mut st = state.borrow_mut();
        st.active_collection = active_collection;
        st.library.load_virtual(&paths)
    };
    if pending.is_empty() {
        return;
    }

    let (tx, rx) = async_channel::unbounded::<VirtualMetadataResult>();
    rayon::spawn(move || {
        let started = Instant::now();
        let total = pending.len();
        let mut cache = rustc_hash::FxHashMap::default();
        for path in pending {
            let cached = LibraryManager::cached_image_data_static(&path, &mut cache);
            let _ = tx.send_blocking(VirtualMetadataResult { path, cached });
        }
        crate::bench_event!(
            "virtual_view.metadata.finish",
            serde_json::json!({
                "total": total,
                "duration_ms": crate::bench::duration_ms(started),
            }),
        );
    });

    let state_rx = state.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(result) = rx.recv().await {
            let mut st = state_rx.borrow_mut();
            if st.library.current_folder.is_none() {
                st.library
                    .apply_cached_image_data(&result.path, result.cached);
            }
        }
    });
}

fn maybe_download_model(model: SmartModel) {
    let model_path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(format!("sharpr/models/{}", model.filename()));
    if model_path.exists() {
        return;
    }
    rayon::spawn(move || {
        let Some(dir) = model_path.parent() else {
            return;
        };
        let _ = std::fs::create_dir_all(dir);
        let tmp = model_path.with_extension("onnx.tmp");

        let result = (|| -> Result<(), String> {
            let response = ureq::get(model.url())
                .call()
                .map_err(|err| format!("download failed: {err}"))?;
            let mut reader = response.into_reader();
            let mut file = std::fs::File::create(&tmp)
                .map_err(|err| format!("create temp model failed: {err}"))?;
            std::io::copy(&mut reader, &mut file)
                .map_err(|err| format!("write temp model failed: {err}"))?;
            drop(file);

            let downloaded = std::fs::read(&tmp)
                .map_err(|err| format!("read downloaded model failed: {err}"))?;
            let actual_hash = format!("{:x}", Sha256::digest(&downloaded));
            let expected_hash = model.sha256();
            if actual_hash != expected_hash {
                return Err(format!(
                    "downloaded model hash mismatch: expected {expected_hash}, got {actual_hash}"
                ));
            }

            std::fs::rename(&tmp, &model_path)
                .map_err(|err| format!("install downloaded model failed: {err}"))?;
            Ok(())
        })();

        if let Err(err) = result {
            let _ = std::fs::remove_file(&tmp);
            eprintln!("Smart tagger model download aborted: {err}");
        }
    });
}

impl AppState {
    fn new(ops: crate::ops::queue::OpQueue) -> Self {
        let settings = AppSettings::load();
        let mut library = LibraryManager::new();
        library.set_thumbnail_cache_max(settings.thumbnail_cache_max as usize);
        let upscale_binary = settings.upscaler_binary_path.clone();
        let smart_model = SmartModel::from_id(&settings.smart_tagger_model);
        let model_path = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(format!("sharpr/models/{}", smart_model.filename()));
        let smart_tagger = if model_path.exists() {
            Some(Arc::new(crate::tags::smart::LocalTagger::new(model_path)))
        } else {
            None
        };
        let (library_index, library_index_error) = match LibraryIndex::open() {
            Ok(index) => (Some(Arc::new(index)), None),
            Err(err) => {
                let message = err.to_string();
                crate::bench_event!(
                    "index.open.fail",
                    serde_json::json!({
                        "error": message,
                    }),
                );
                (None, Some(message))
            }
        };
        let disabled_folders = library_index
            .as_ref()
            .and_then(|index| index.ignored_folders().ok())
            .unwrap_or_default();
        let state = Self {
            library,
            settings,
            sort_order: SortOrder::default(),
            library_index,
            library_index_error,
            tags: crate::tags::TagDatabase::open().ok().map(Arc::new),
            smart_tagger,
            upscale_binary,
            ops,
            selected_paths: HashSet::new(),
            active_collection: None,
            disabled_folders,
        };
        maybe_download_model(smart_model);
        state
    }
}

// ---------------------------------------------------------------------------
// GObject subclass
// ---------------------------------------------------------------------------

mod imp {
    use super::*;
    use crate::thumbnails::worker::{HashResult, ThumbnailResult};
    use async_channel::Receiver;

    pub struct SharprWindow {
        pub state: Rc<RefCell<AppState>>,
        pub viewer: RefCell<Option<ViewerPane>>,
        pub thumbnail_worker: RefCell<Option<ThumbnailWorker>>,
        // Cloned receiver so the async task can hold it.
        pub result_rx: RefCell<Option<Receiver<ThumbnailResult>>>,
        pub hash_result_rx: RefCell<Option<Receiver<HashResult>>>,
        pub(super) thumbnail_ops: RefCell<HashMap<PathBuf, ThumbnailOpState>>,
        pub toast_overlay: RefCell<Option<libadwaita::ToastOverlay>>,
    }

    impl Default for SharprWindow {
        fn default() -> Self {
            let (ops_queue, _ops_rx) = crate::ops::queue::new_queue();
            Self {
                state: Rc::new(RefCell::new(AppState::new(ops_queue))),
                viewer: RefCell::new(None),
                thumbnail_worker: RefCell::new(None),
                result_rx: RefCell::new(None),
                hash_result_rx: RefCell::new(None),
                thumbnail_ops: RefCell::new(HashMap::new()),
                toast_overlay: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SharprWindow {
        const NAME: &'static str = "SharprWindow";
        type Type = super::SharprWindow;
        type ParentType = libadwaita::ApplicationWindow;
    }

    impl ObjectImpl for SharprWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }

    impl WidgetImpl for SharprWindow {}
    impl WindowImpl for SharprWindow {}
    impl ApplicationWindowImpl for SharprWindow {}
    impl AdwApplicationWindowImpl for SharprWindow {}
}

glib::wrapper! {
    pub struct SharprWindow(ObjectSubclass<imp::SharprWindow>)
        @extends libadwaita::ApplicationWindow,
                 gtk4::ApplicationWindow,
                 gtk4::Window,
                 gtk4::Widget,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl SharprWindow {
    pub fn new(app: &libadwaita::Application) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    pub fn app_state(&self) -> std::rc::Rc<std::cell::RefCell<crate::ui::window::AppState>> {
        self.imp().state.clone()
    }

    pub fn add_toast(&self, toast: libadwaita::Toast) {
        if let Some(overlay) = self.imp().toast_overlay.borrow().as_ref() {
            overlay.add_toast(toast);
        }
    }

    pub fn reload_smart_tagger_model(&self, model: SmartModel) {
        let model_path = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(format!("sharpr/models/{}", model.filename()));

        if model_path.exists() {
            let tagger = Arc::new(crate::tags::smart::LocalTagger::new(model_path));
            self.app_state().borrow_mut().smart_tagger = Some(tagger);
            if let Some(viewer) = self.imp().viewer.borrow().as_ref() {
                viewer.show_smart_tag_btn();
            }
            return;
        }

        let (tx, rx) = async_channel::unbounded::<Arc<crate::tags::smart::LocalTagger>>();
        let window_weak = self.downgrade();

        rayon::spawn(move || {
            let Some(dir) = model_path.parent() else {
                return;
            };
            let _ = std::fs::create_dir_all(dir);
            let tmp = model_path.with_extension("onnx.tmp");

            let result = (|| -> Result<(), String> {
                let response = ureq::get(model.url())
                    .call()
                    .map_err(|err| format!("download failed: {err}"))?;
                let mut reader = response.into_reader();
                let mut file = std::fs::File::create(&tmp)
                    .map_err(|err| format!("create temp model failed: {err}"))?;
                std::io::copy(&mut reader, &mut file)
                    .map_err(|err| format!("write temp model failed: {err}"))?;
                drop(file);

                let downloaded = std::fs::read(&tmp)
                    .map_err(|err| format!("read downloaded model failed: {err}"))?;
                let actual_hash = format!("{:x}", Sha256::digest(&downloaded));
                let expected_hash = model.sha256();
                if actual_hash != expected_hash {
                    return Err(format!(
                        "downloaded model hash mismatch: expected {expected_hash}, got {actual_hash}"
                    ));
                }

                std::fs::rename(&tmp, &model_path)
                    .map_err(|err| format!("install downloaded model failed: {err}"))?;
                Ok(())
            })();

            if let Err(err) = result {
                let _ = std::fs::remove_file(&tmp);
                eprintln!("Smart tagger model download aborted: {err}");
                return;
            }

            let tagger = Arc::new(crate::tags::smart::LocalTagger::new(model_path));
            let _ = tx.send_blocking(tagger);
        });

        glib::MainContext::default().spawn_local(async move {
            if let Ok(tagger) = rx.recv().await {
                if let Some(window) = window_weak.upgrade() {
                    window.app_state().borrow_mut().smart_tagger = Some(tagger);
                    if let Some(viewer) = window.imp().viewer.borrow().as_ref() {
                        viewer.show_smart_tag_btn();
                    }
                }
            }
        });
    }

    fn setup(&self) {
        self.set_title(Some("Image Library"));
        let (ww, wh) = {
            let st = self.imp().state.borrow();
            (st.settings.window_width, st.settings.window_height)
        };
        self.set_default_size(ww, wh);

        // -----------------------------------------------------------------------
        // Thumbnail worker pool
        // -----------------------------------------------------------------------
        let cpu_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        // Double the thread count for I/O bound thumbnail loading, cap at 16.
        let thread_count = (cpu_count * 2).clamp(4, 16);
        let (worker, result_rx, hash_result_rx) = ThumbnailWorker::spawn(thread_count);
        *self.imp().thumbnail_worker.borrow_mut() = Some(worker);
        *self.imp().result_rx.borrow_mut() = Some(result_rx);
        *self.imp().hash_result_rx.borrow_mut() = Some(hash_result_rx);

        let state = self.imp().state.clone();
        let (ops_queue, ops_rx) = crate::ops::queue::new_queue();
        state.borrow_mut().ops = ops_queue;
        let state_close = state.clone();
        self.connect_close_request(move |win| {
            let (w, h) = (win.width(), win.height());
            let mut st = state_close.borrow_mut();
            st.settings.window_width = w;
            st.settings.window_height = h;
            st.settings.save();
            glib::Propagation::Proceed
        });

        // Clean up any upscale artifacts left behind by a previous crash.
        {
            let last_folder = self.imp().state.borrow().settings.last_folder.clone();
            if let Some(folder) = last_folder {
                let upscaled_dir = folder.join("upscaled");
                if let Ok(entries) = std::fs::read_dir(&upscaled_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if name.contains(".pending-") || name.contains(".ncnn-intermediate") {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }

        // -----------------------------------------------------------------------
        // Build panes
        // -----------------------------------------------------------------------
        let sidebar = SidebarPane::new(state.clone());
        let filmstrip = FilmstripPane::new(state.clone());
        let viewer = ViewerPane::new(state.clone());
        *self.imp().viewer.borrow_mut() = Some(viewer.clone());
        viewer.set_metadata_visible(state.borrow().settings.metadata_visible);
        if state.borrow().smart_tagger.is_some() {
            viewer.show_smart_tag_btn();
        }

        if let Some(worker) = self.imp().thumbnail_worker.borrow().as_ref() {
            filmstrip.set_thumbnail_sender(
                worker.visible_sender(),
                worker.preload_sender(),
                worker.generation_arc(),
                worker.pending_set(),
            );
        }

        let suppress_search_restore = Rc::new(Cell::new(false));
        let content_stack = gtk4::Stack::new();
        let toast_overlay = libadwaita::ToastOverlay::new();
        *self.imp().toast_overlay.borrow_mut() = Some(toast_overlay.clone());
        let tag_browser = state.borrow().tags.clone().map(TagBrowser::new);
        let outer_split = libadwaita::OverlaySplitView::new();
        outer_split.set_max_sidebar_width(280.0);
        outer_split.set_min_sidebar_width(200.0);
        outer_split.connect_collapsed_notify(|split| {
            if split.is_collapsed() {
                split.set_show_sidebar(false);
            }
        });

        let open_folder: Rc<dyn Fn(PathBuf)> = {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let window_weak = self.downgrade();
            let suppress_search_restore_c = suppress_search_restore.clone();
            let content_stack = content_stack.clone();
            Rc::new(move |path: PathBuf| {
                if path_is_disabled(&path, &state_c.borrow().disabled_folders) {
                    toast_overlay_c.add_toast(libadwaita::Toast::new("Folder is disabled"));
                    sidebar_c.set_folder_ignored(&path, true);
                    return;
                }
                crate::bench_event!(
                    "folder.open.request",
                    serde_json::json!({
                        "path": path.display().to_string(),
                    }),
                );
                if let Some(win) = window_weak.upgrade() {
                    let _ = win.bump_thumbnail_generation("folder.open");
                    win.complete_thumbnail_ops();
                }
                content_stack.set_visible_child_name("viewer");
                let cache_max = AppSettings::load().thumbnail_cache_max as usize;
                state_c
                    .borrow_mut()
                    .library
                    .set_thumbnail_cache_max(cache_max);
                state_c.borrow_mut().library.reset_for_folder(&path);
                {
                    let mut st = state_c.borrow_mut();
                    st.settings.last_folder = Some(path.clone());
                    st.settings.save();
                    st.selected_paths.clear();
                    st.active_collection = None;
                }

                let (index, mut index_error, sort_order) = {
                    let st = state_c.borrow();
                    (
                        st.library_index.clone(),
                        st.library_index_error.clone(),
                        st.sort_order,
                    )
                };
                let (tx, rx) = async_channel::unbounded::<FolderOpenResult>();
                let scan_path = path.clone();
                rayon::spawn(move || {
                    let started = Instant::now();
                    if let Some(index) = index {
                        // Send cached rows immediately so the UI can render before the
                        // filesystem scan finishes.
                        if let Ok(cached) = index.images_in_folder(&scan_path, sort_order) {
                            if !cached.is_empty() {
                                crate::bench_event!(
                                    "folder.open.cached_rows",
                                    serde_json::json!({
                                        "path": scan_path.display().to_string(),
                                        "row_count": cached.len(),
                                    }),
                                );
                                let _ = tx.send_blocking(FolderOpenResult::Cached { rows: cached });
                            }
                        }

                        let scan_started = Instant::now();
                        let entries = LibraryManager::scan_folder_basic(&scan_path);
                        crate::bench_event!(
                            "index.folder_scan_basic.finish",
                            serde_json::json!({
                                "path": scan_path.display().to_string(),
                                "entry_count": entries.len(),
                                "duration_ms": crate::bench::duration_ms(scan_started),
                            }),
                        );

                        match index.reconcile_folder(&scan_path, &entries, sort_order) {
                            Ok((rows, stale_removed, metadata_pending)) => {
                                crate::bench_event!(
                                    "folder.open.stale_rows_removed",
                                    serde_json::json!({
                                        "path": scan_path.display().to_string(),
                                        "count": stale_removed,
                                    }),
                                );
                                crate::bench_event!(
                                    "folder.open.db_rows",
                                    serde_json::json!({
                                        "path": scan_path.display().to_string(),
                                        "row_count": rows.len(),
                                    }),
                                );
                                crate::bench_event!(
                                    "folder.scan.finish",
                                    serde_json::json!({
                                        "path": scan_path.display().to_string(),
                                        "source": "index",
                                        "duration_ms": crate::bench::duration_ms(started),
                                    }),
                                );
                                let _ = tx.send_blocking(FolderOpenResult::Indexed {
                                    rows,
                                    metadata_pending,
                                    stale_removed,
                                    basic_count: entries.len(),
                                });
                                return;
                            }
                            Err(err) => {
                                index_error = Some(err.to_string());
                                crate::bench_event!(
                                    "index.reconcile_folder.fail",
                                    serde_json::json!({
                                        "path": scan_path.display().to_string(),
                                        "error": err.to_string(),
                                    }),
                                );
                            }
                        }
                    }

                    let entries = LibraryManager::scan_folder_raw(&scan_path);
                    crate::bench_event!(
                        "folder.scan.finish",
                        serde_json::json!({
                            "path": scan_path.display().to_string(),
                            "source": "raw",
                            "entry_count": entries.len(),
                            "duration_ms": crate::bench::duration_ms(started),
                        }),
                    );
                    let _ = tx.send_blocking(FolderOpenResult::Raw {
                        entries,
                        index_error,
                    });
                });

                let filmstrip_rx = filmstrip_c.clone();
                let viewer_rx = viewer_c.clone();
                let sidebar_rx = sidebar_c.clone();
                let state_rx = state_c.clone();
                let suppress_search_restore_rx = suppress_search_restore_c.clone();
                let path_rx = path.clone();
                let window_weak_rx = window_weak.clone();
                let toast_overlay_rx = toast_overlay_c.clone();
                glib::MainContext::default().spawn_local(async move {
                    let Ok(first_result) = rx.recv().await else {
                        return;
                    };

                    if state_rx.borrow().library.current_folder.as_deref()
                        != Some(path_rx.as_path())
                    {
                        crate::bench_event!(
                            "folder.open.stale_result",
                            serde_json::json!({ "path": path_rx.display().to_string() }),
                        );
                        return;
                    }

                    let mut metadata_pending = Vec::new();
                    let mut got_cached = false;

                    // Load the first result into the store.
                    {
                        let started = Instant::now();
                        let mut st = state_rx.borrow_mut();
                        let entry_count = match first_result {
                            FolderOpenResult::Cached { rows } => {
                                got_cached = true;
                                let count = rows.len();
                                st.library.load_indexed_folder(&path_rx, rows);
                                crate::bench_event!(
                                    "folder.open.cached_loaded",
                                    serde_json::json!({
                                        "path": path_rx.display().to_string(),
                                        "row_count": count,
                                    }),
                                );
                                count
                            }
                            FolderOpenResult::Indexed {
                                rows,
                                metadata_pending: pending,
                                stale_removed,
                                basic_count,
                            } => {
                                metadata_pending = pending;
                                let row_count = rows.len();
                                st.library.load_indexed_folder(&path_rx, rows);
                                crate::bench_event!(
                                    "folder.open.index_loaded",
                                    serde_json::json!({
                                        "path": path_rx.display().to_string(),
                                        "basic_count": basic_count,
                                        "stale_removed": stale_removed,
                                        "metadata_pending": metadata_pending.len(),
                                    }),
                                );
                                row_count
                            }
                            FolderOpenResult::Raw {
                                entries: mut raw_entries,
                                index_error,
                            } => {
                                if let Some(error) = index_error {
                                    toast_overlay_rx.add_toast(libadwaita::Toast::new(&format!(
                                        "Library index unavailable; using direct folder scan ({error})"
                                    )));
                                }
                                let order = st.sort_order;
                                crate::model::library::sort_raw_entries(&mut raw_entries, order);
                                let mut new_entries: Vec<ImageEntry> =
                                    Vec::with_capacity(raw_entries.len());
                                for (index, raw) in raw_entries.into_iter().enumerate() {
                                    let entry = ImageEntry::new(raw.path.clone());
                                    entry.set_file_size(raw.file_size);
                                    entry.set_dimensions(raw.width, raw.height);
                                    if let Some(texture) = st.library.cached_thumbnail(&raw.path) {
                                        entry.set_thumbnail(Some(texture));
                                    }
                                    st.library
                                        .path_to_index
                                        .insert(raw.path.clone(), index as u32);
                                    st.library.all_known_paths.insert(raw.path);
                                    new_entries.push(entry);
                                }
                                let entry_count = new_entries.len();
                                st.library.store.splice(0, 0, &new_entries);
                                entry_count
                            }
                        };
                        crate::bench_event!(
                            "folder.store_populate.finish",
                            serde_json::json!({
                                "path": path_rx.display().to_string(),
                                "entry_count": entry_count,
                                "duration_ms": crate::bench::duration_ms(started),
                            }),
                        );
                    }

                    sidebar_rx.select_folder(&path_rx);
                    sidebar_rx.set_search_selected(false);
                    sidebar_rx.set_duplicates_selected(false);
                    sidebar_rx.set_tags_selected(false);
                    sidebar_rx.set_quality_selected(None);

                    viewer_rx.clear();
                    filmstrip_rx.refresh();

                    let thumb_total = state_rx.borrow().library.image_count();
                    crate::bench_event!(
                        "folder.open.ready",
                        serde_json::json!({
                            "path": path_rx.display().to_string(),
                            "image_count": thumb_total,
                        }),
                    );
                    let thumb_op = if thumb_total > 0 {
                        Some(
                            state_rx
                                .borrow()
                                .ops
                                .add(format!("Loading thumbnails ({thumb_total})")),
                        )
                    } else {
                        None
                    };

                    if let Some(win) = window_weak_rx.upgrade() {
                        if let Some(handle) = thumb_op {
                            win.imp().thumbnail_ops.borrow_mut().insert(
                                path_rx.clone(),
                                ThumbnailOpState {
                                    total: thumb_total,
                                    received: 0,
                                    handle,
                                },
                            );
                        }
                    }

                    let target_index = {
                        let st = state_rx.borrow();
                        st.library
                            .restore_index_for(&path_rx)
                            .filter(|&idx| st.library.entry_at(idx).is_some())
                            .or_else(|| st.library.entry_at(0).map(|_| 0))
                    };

                    if let Some(index) = target_index {
                        state_rx.borrow_mut().library.selected_index = Some(index);
                        filmstrip_rx.navigate_to(index);
                        let path = state_rx
                            .borrow()
                            .library
                            .entry_at(index)
                            .map(|e: ImageEntry| e.path());
                        if let Some(path) = path {
                            viewer_rx.load_image(path);
                        }
                    }

                    if filmstrip_rx.is_search_active() {
                        suppress_search_restore_rx.set(true);
                        filmstrip_rx.deactivate_search();
                    }

                    // If we showed cached rows, wait for the reconciled result and apply
                    // any changes silently (no full UI reset unless the path set changed).
                    if got_cached {
                        if let Ok(FolderOpenResult::Indexed {
                            rows,
                            metadata_pending: pending,
                            stale_removed,
                            basic_count,
                        }) = rx.recv().await
                        {
                            if state_rx.borrow().library.current_folder.as_deref()
                                == Some(path_rx.as_path())
                            {
                                metadata_pending = pending;

                                let cached_paths: std::collections::HashSet<PathBuf> = {
                                    let st = state_rx.borrow();
                                    (0..st.library.image_count())
                                        .filter_map(|i| st.library.entry_at(i).map(|e| e.path()))
                                        .collect()
                                };
                                let reconciled_paths: std::collections::HashSet<PathBuf> =
                                    rows.iter().map(|r| r.path.clone()).collect();

                                crate::bench_event!(
                                    "folder.open.index_loaded",
                                    serde_json::json!({
                                        "path": path_rx.display().to_string(),
                                        "basic_count": basic_count,
                                        "stale_removed": stale_removed,
                                        "metadata_pending": metadata_pending.len(),
                                    }),
                                );

                                if cached_paths != reconciled_paths {
                                    // Folder contents changed since the cached result —
                                    // save selection, reload, restore.
                                    let selected_path = {
                                        let st = state_rx.borrow();
                                        st.library
                                            .selected_index
                                            .and_then(|i| st.library.entry_at(i))
                                            .map(|e| e.path())
                                    };
                                    state_rx
                                        .borrow_mut()
                                        .library
                                        .load_indexed_folder(&path_rx, rows);
                                    filmstrip_rx.refresh();
                                    let target = {
                                        let st = state_rx.borrow();
                                        selected_path
                                            .as_deref()
                                            .and_then(|p| st.library.index_of_path(p))
                                            .or_else(|| {
                                                if st.library.image_count() > 0 {
                                                    Some(0)
                                                } else {
                                                    None
                                                }
                                            })
                                    };
                                    if let Some(idx) = target {
                                        state_rx.borrow_mut().library.selected_index = Some(idx);
                                        filmstrip_rx.navigate_to(idx);
                                        let p = state_rx
                                            .borrow()
                                            .library
                                            .entry_at(idx)
                                            .map(|e| e.path());
                                        if let Some(p) = p {
                                            viewer_rx.load_image(p);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let Some(index) = state_rx.borrow().library_index.clone() {
                        start_metadata_indexer(
                            index,
                            path_rx.clone(),
                            metadata_pending,
                            state_rx.clone(),
                        );
                    }
                });
            })
        };

        // Sidebar folder selection → scan library → refresh filmstrip.
        {
            let open_folder_c = open_folder.clone();
            sidebar.connect_folder_selected(move |path| {
                open_folder_c(path);
            });
        }

        {
            let state_c = state.clone();
            let sidebar_c = sidebar.clone();
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let toast_overlay_c = toast_overlay.clone();
            let window_weak = self.downgrade();
            sidebar.connect_folder_ignored_changed(move |path, ignored| {
                let Some(index) = state_c.borrow().library_index.clone() else {
                    toast_overlay_c.add_toast(libadwaita::Toast::new("Library index unavailable"));
                    sidebar_c.set_folder_ignored(&path, !ignored);
                    return;
                };

                match index.set_folder_ignored(&path, ignored) {
                    Ok(()) => {
                        let disabled_folders = {
                            let mut st = state_c.borrow_mut();
                            if ignored {
                                if !st.disabled_folders.iter().any(|p| p == &path) {
                                    st.disabled_folders.push(path.clone());
                                }
                            } else {
                                st.disabled_folders.retain(|p| !path.starts_with(p));
                            }
                            let disabled_folders = st.disabled_folders.clone();
                            st.selected_paths
                                .retain(|selected| !path_is_disabled(selected, &disabled_folders));

                            if ignored
                                && st
                                    .library
                                    .current_folder
                                    .as_deref()
                                    .map(|folder| folder.starts_with(&path))
                                    .unwrap_or(false)
                            {
                                st.library.load_virtual(&[]);
                                st.active_collection = None;
                            } else if st.library.current_folder.is_none() {
                                let visible_paths: Vec<PathBuf> = (0..st.library.image_count())
                                    .filter_map(|i| st.library.entry_at(i).map(|e| e.path()))
                                    .filter(|image_path| {
                                        !path_is_disabled(image_path, &st.disabled_folders)
                                    })
                                    .collect();
                                st.library.load_virtual(&visible_paths);
                            }
                            st.disabled_folders.clone()
                        };

                        sidebar_c.set_ignored_folders(&disabled_folders);
                        viewer_c.clear();
                        filmstrip_c.refresh_virtual();
                        if let Some(win) = window_weak.upgrade() {
                            let _ = win.bump_thumbnail_generation("folder.ignored_changed");
                            win.complete_thumbnail_ops();
                        }
                        let label = if ignored {
                            "Folder disabled"
                        } else {
                            "Folder enabled"
                        };
                        toast_overlay_c.add_toast(libadwaita::Toast::new(label));
                    }
                    Err(err) => {
                        sidebar_c.set_folder_ignored(&path, !ignored);
                        toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                            "Could not update folder: {err}"
                        )));
                    }
                }
            });
        }

        {
            let state_sort = state.clone();
            let open_folder_sort = open_folder.clone();
            filmstrip.set_sort_order_changed_cb(move |order| {
                let current_folder = state_sort.borrow().settings.last_folder.clone();
                state_sort.borrow_mut().sort_order = order;
                if let Some(folder) = current_folder {
                    open_folder_sort(folder);
                }
            });
        }

        // Duplicates row → group detection → load_virtual.
        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let suppress_search_restore_c = suppress_search_restore.clone();
            let content_stack = content_stack.clone();
            let toast_overlay_c = toast_overlay.clone();
            let window_weak = self.downgrade();
            sidebar.connect_duplicates_selected(move || {
                let expected_gen = window_weak.upgrade().and_then(|win| {
                    let gen = win.bump_thumbnail_generation("smart.duplicates");
                    win.complete_thumbnail_ops();
                    gen
                });
                content_stack.set_visible_child_name("viewer");
                let hashes = {
                    let st = state_c.borrow();
                    st.library
                        .all_hashes_snapshot()
                        .into_iter()
                        .filter(|(path, _)| !path_is_disabled(path, &st.disabled_folders))
                        .collect::<Vec<_>>()
                };
                crate::bench_event!(
                    "smart.duplicates.request",
                    serde_json::json!({
                        "hash_count": hashes.len(),
                    }),
                );
                if hashes.is_empty() {
                    toast_overlay_c.add_toast(libadwaita::Toast::new(
                        "Browse your library first — hashes are computed as thumbnails load",
                    ));
                    return;
                }

                let op = state_c.borrow().ops.add("Finding duplicates");
                let (tx, rx) = async_channel::bounded::<Vec<PathBuf>>(1);
                rayon::spawn(move || {
                    let started = Instant::now();
                    let paths = phash::group_duplicates(&hashes)
                        .into_iter()
                        .filter(|group| group.len() > 1)
                        .flatten()
                        .collect();
                    crate::bench_event!(
                        "smart.duplicates.finish",
                        serde_json::json!({
                            "duration_ms": crate::bench::duration_ms(started),
                        }),
                    );
                    let _ = tx.send_blocking(paths);
                });

                let filmstrip_rx = filmstrip_c.clone();
                let viewer_rx = viewer_c.clone();
                let sidebar_rx = sidebar_c.clone();
                let state_rx = state_c.clone();
                let suppress_search_restore_rx = suppress_search_restore_c.clone();
                let window_weak_rx = window_weak.clone();
                glib::MainContext::default().spawn_local(async move {
                    let Ok(paths) = rx.recv().await else {
                        op.fail("Detection failed");
                        return;
                    };
                    if let (Some(expected), Some(win)) = (expected_gen, window_weak_rx.upgrade()) {
                        if win.current_thumbnail_generation() != Some(expected) {
                            crate::bench_event!(
                                "smart.duplicates.stale_result",
                                serde_json::json!({
                                    "expected_gen": expected,
                                }),
                            );
                            op.complete();
                            return;
                        }
                    }

                    if paths.is_empty() {
                        op.fail("No duplicates found");
                        return;
                    }

                    let result_count = paths.len();
                    load_virtual_async(&state_rx, &paths);
                    crate::bench_event!(
                        "virtual_view.load",
                        serde_json::json!({
                            "source": "duplicates",
                            "image_count": result_count,
                        }),
                    );
                    sidebar_rx.set_duplicates_selected(true);
                    sidebar_rx.set_search_selected(false);
                    sidebar_rx.set_tags_selected(false);
                    sidebar_rx.set_quality_selected(None);
                    filmstrip_rx.refresh_virtual();
                    let first = state_rx
                        .borrow()
                        .library
                        .entry_at(0)
                        .map(|e: ImageEntry| e.path());
                    if let Some(p) = first {
                        filmstrip_rx.navigate_to(0);
                        viewer_rx.load_image(p);
                    } else {
                        viewer_rx.clear();
                    }
                    if filmstrip_rx.is_search_active() {
                        suppress_search_restore_rx.set(true);
                        filmstrip_rx.deactivate_search();
                    }
                    op.complete();
                });
            });
        }

        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let content_stack = content_stack.clone();
            let window_weak = self.downgrade();
            sidebar.connect_search_activated(move || {
                if let Some(win) = window_weak.upgrade() {
                    let _ = win.bump_thumbnail_generation("smart.search");
                    win.complete_thumbnail_ops();
                }
                content_stack.set_visible_child_name("viewer");
                crate::bench_event!("smart.search.activate", serde_json::json!({}));
                sidebar_c.set_search_selected(true);
                sidebar_c.set_duplicates_selected(false);
                sidebar_c.set_tags_selected(false);
                sidebar_c.set_quality_selected(None);
                // Show an empty filmstrip immediately so the user knows to type.
                load_virtual_async(&state_c, &[]);
                crate::bench_event!(
                    "virtual_view.load",
                    serde_json::json!({
                        "source": "search",
                        "image_count": 0,
                    }),
                );
                viewer_c.clear();
                filmstrip_c.refresh_virtual();
                filmstrip_c.activate_search();
            });
        }

        if let Some(tag_browser) = tag_browser.clone() {
            let sidebar_c = sidebar.clone();
            let content_stack_c = content_stack.clone();
            sidebar.connect_tags_selected(move || {
                content_stack_c.set_visible_child_name("tags");
                tag_browser.refresh();
                sidebar_c.set_tags_selected(true);
                sidebar_c.set_quality_selected(None);
            });
        }

        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let suppress_search_restore_c = suppress_search_restore.clone();
            let content_stack = content_stack.clone();
            let window_weak = self.downgrade();
            sidebar.connect_quality_selected(move |class| {
                let expected_gen = window_weak.upgrade().and_then(|win| {
                    let gen = win.bump_thumbnail_generation("smart.quality");
                    win.complete_thumbnail_ops();
                    gen
                });
                content_stack.set_visible_child_name("viewer");
                crate::bench_event!(
                    "smart.quality.request",
                    serde_json::json!({
                        "class": class.label(),
                    }),
                );

                let (indexed_paths, current_folder, metadata_cache) =
                    { state_c.borrow().library.bg_scan_quality_prep() };
                let library_root = state_c.borrow().settings.library_root.clone();
                let disabled_folders = state_c.borrow().disabled_folders.clone();
                let library_index = state_c.borrow().library_index.clone();
                let op = state_c
                    .borrow()
                    .ops
                    .add(format!("Scanning for {} quality", class.label()));

                // Channel carries (matched_paths, Option<scan_state_to_store_back>).
                // None = result came from the DB index (no scan state to update).
                type ScanState = (
                    Vec<PathBuf>,
                    rustc_hash::FxHashMap<PathBuf, crate::model::library::CachedImageData>,
                );
                let (tx, rx) = async_channel::bounded::<(Vec<PathBuf>, Option<ScanState>)>(1);
                rayon::spawn(move || {
                    let started = Instant::now();

                    // Try the persistent index first — much faster than a filesystem walk.
                    if let Some(index) = library_index {
                        match index.images_by_quality(class) {
                            Ok(paths) if !paths.is_empty() => {
                                crate::bench_event!(
                                    "smart.quality.scan_finish",
                                    serde_json::json!({
                                        "class": class.label(),
                                        "source": "index",
                                        "result_count": paths.len(),
                                        "duration_ms": crate::bench::duration_ms(started),
                                    }),
                                );
                                let _ = tx.send_blocking((paths, None));
                                return;
                            }
                            Ok(_) => {} // not indexed yet — fall through to filesystem scan
                            Err(err) => {
                                crate::bench_event!(
                                    "smart.quality.index_fail",
                                    serde_json::json!({
                                        "class": class.label(),
                                        "error": err.to_string(),
                                    }),
                                );
                            }
                        }
                    }

                    // Fallback: full filesystem scan.
                    let (new_indexed, new_cache, paths) =
                        crate::model::library::LibraryManager::compute_paths_for_quality_class(
                            library_root,
                            class,
                            indexed_paths,
                            current_folder,
                            metadata_cache,
                            disabled_folders,
                        );
                    crate::bench_event!(
                        "smart.quality.scan_finish",
                        serde_json::json!({
                            "class": class.label(),
                            "source": "scan",
                            "indexed_count": new_indexed.len(),
                            "result_count": paths.len(),
                            "duration_ms": crate::bench::duration_ms(started),
                        }),
                    );
                    let _ = tx.send_blocking((paths, Some((new_indexed, new_cache))));
                });

                let filmstrip_rx = filmstrip_c.clone();
                let viewer_rx = viewer_c.clone();
                let sidebar_rx = sidebar_c.clone();
                let state_rx = state_c.clone();
                let suppress_rx = suppress_search_restore_c.clone();
                let window_weak_rx = window_weak.clone();

                glib::MainContext::default().spawn_local(async move {
                    let Ok((paths, scan_state)) = rx.recv().await else {
                        op.fail("Quality scan failed");
                        return;
                    };
                    if let (Some(expected), Some(win)) = (expected_gen, window_weak_rx.upgrade()) {
                        if win.current_thumbnail_generation() != Some(expected) {
                            crate::bench_event!(
                                "smart.quality.stale_result",
                                serde_json::json!({
                                    "class": class.label(),
                                    "expected_gen": expected,
                                }),
                            );
                            op.complete();
                            return;
                        }
                    }

                    let result_count = paths.len();
                    if let Some((new_indexed, new_cache)) = scan_state {
                        state_rx
                            .borrow_mut()
                            .library
                            .bg_scan_quality_finish(new_indexed, new_cache);
                    }
                    load_virtual_async(&state_rx, &paths);
                    crate::bench_event!(
                        "virtual_view.load",
                        serde_json::json!({
                            "source": "quality",
                            "class": class.label(),
                            "image_count": result_count,
                        }),
                    );
                    sidebar_rx.set_quality_selected(Some(class));
                    sidebar_rx.set_search_selected(false);
                    sidebar_rx.set_duplicates_selected(false);
                    sidebar_rx.set_tags_selected(false);
                    viewer_rx.clear();
                    filmstrip_rx.refresh_virtual();
                    let first = state_rx
                        .borrow()
                        .library
                        .entry_at(0)
                        .map(|e: crate::model::ImageEntry| e.path());

                    if let Some(path) = first {
                        state_rx.borrow_mut().library.selected_index = Some(0);
                        filmstrip_rx.navigate_to(0);
                        viewer_rx.load_image(path);
                    }
                    if filmstrip_rx.is_search_active() {
                        suppress_rx.set(true);
                        filmstrip_rx.deactivate_search();
                    }
                    op.complete();
                });
            });
        }

        // Helper: refresh the sidebar collection list from the DB.
        let refresh_sidebar_collections = {
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            move || {
                if let Some(idx) = state_c.borrow().library_index.clone() {
                    let collections = idx.list_collections().unwrap_or_default();
                    sidebar_c.refresh_collections(&collections);
                }
            }
        };

        // Populate sidebar collections on startup.
        refresh_sidebar_collections();

        // "New Collection" + button → AlertDialog for name → create → refresh.
        {
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            let window_weak = self.downgrade();
            sidebar.connect_collection_add_requested(move || {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                let dialog = libadwaita::AlertDialog::new(Some("New Collection"), None);
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("create", "Create");
                dialog.set_default_response(Some("create"));
                dialog.set_close_response("cancel");
                dialog.set_response_appearance("create", libadwaita::ResponseAppearance::Suggested);
                let entry = gtk4::Entry::new();
                entry.set_placeholder_text(Some("Collection name"));
                let entry_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                entry_box.set_margin_top(6);
                entry_box.append(&entry);
                dialog.set_extra_child(Some(&entry_box));
                let state_d = state_c.clone();
                let toast_d = toast_overlay_c.clone();
                let refresh_d = refresh_c.clone();
                let entry_clone = entry.clone();
                dialog.connect_response(None, move |_, response| {
                    if response != "create" {
                        return;
                    }
                    let name = entry_clone.text().to_string();
                    if let Some(idx) = state_d.borrow().library_index.clone() {
                        let started = std::time::Instant::now();
                        match idx.create_collection(&name) {
                            Ok(coll) => {
                                crate::bench_event!(
                                    "collection.create",
                                    serde_json::json!({
                                        "collection_id": coll.id,
                                        "name": coll.name,
                                        "duration_ms": crate::bench::duration_ms(started),
                                    }),
                                );
                                refresh_d();
                                toast_d.add_toast(libadwaita::Toast::new(&format!(
                                    "Collection \u{201c}{}\u{201d} created",
                                    coll.name
                                )));
                            }
                            Err(e) => {
                                toast_d.add_toast(libadwaita::Toast::new(&format!(
                                    "Could not create collection: {e}"
                                )));
                            }
                        }
                    }
                });
                dialog.present(Some(&win));
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
                if let Some(win) = window_weak.upgrade() {
                    let _ = win.bump_thumbnail_generation("collection.load");
                    win.complete_thumbnail_ops();
                }
                content_stack.set_visible_child_name("viewer");
                let paths = state_c
                    .borrow()
                    .library_index
                    .clone()
                    .and_then(|idx| idx.collection_paths(id).ok())
                    .unwrap_or_default();
                let started = std::time::Instant::now();
                {
                    let mut s = state_c.borrow_mut();
                    s.selected_paths.clear();
                }
                load_collection_async(&state_c, &paths, id);
                crate::bench_event!(
                    "collection.load",
                    serde_json::json!({
                        "collection_id": id,
                        "path_count": paths.len(),
                        "duration_ms": crate::bench::duration_ms(started),
                    }),
                );
                sidebar_c.set_duplicates_selected(false);
                sidebar_c.set_search_selected(false);
                sidebar_c.set_tags_selected(false);
                sidebar_c.set_quality_selected(None);
                filmstrip_c.refresh_virtual();
                let first = state_c
                    .borrow()
                    .library
                    .entry_at(0)
                    .map(|e: ImageEntry| e.path());
                if let Some(p) = first {
                    state_c.borrow_mut().library.selected_index = Some(0);
                    filmstrip_c.navigate_to(0);
                    viewer_c.load_image(p);
                } else {
                    viewer_c.clear();
                    toast_overlay_c.add_toast(libadwaita::Toast::new("No images in collection"));
                }
            });
        }

        // Rename collection → update DB → refresh sidebar.
        {
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            sidebar.connect_collection_rename_requested(move |id, new_name| {
                if let Some(idx) = state_c.borrow().library_index.clone() {
                    let started = std::time::Instant::now();
                    match idx.rename_collection(id, &new_name) {
                        Ok(()) => {
                            crate::bench_event!(
                                "collection.rename",
                                serde_json::json!({
                                    "collection_id": id,
                                    "duration_ms": crate::bench::duration_ms(started),
                                }),
                            );
                            refresh_c();
                        }
                        Err(e) => {
                            toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                                "Could not rename collection: {e}"
                            )));
                        }
                    }
                }
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
                if let Some(idx) = state_c.borrow().library_index.clone() {
                    let started = std::time::Instant::now();
                    match idx.delete_collection(id) {
                        Ok(()) => {
                            crate::bench_event!(
                                "collection.delete",
                                serde_json::json!({
                                    "collection_id": id,
                                    "duration_ms": crate::bench::duration_ms(started),
                                }),
                            );
                            let was_active = state_c.borrow().active_collection == Some(id);
                            if was_active {
                                let mut s = state_c.borrow_mut();
                                s.active_collection = None;
                                s.selected_paths.clear();
                                drop(s);
                                load_virtual_async(&state_c, &[]);
                            }
                            refresh_c();
                            if was_active {
                                viewer_c.clear();
                                filmstrip_c.refresh_virtual();
                            }
                        }
                        Err(e) => {
                            toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                                "Could not delete collection: {e}"
                            )));
                        }
                    }
                }
            });
        }

        // Drag-and-drop from filmstrip to sidebar collection row.
        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            sidebar.connect_drop_paths_to_collection(move |id, paths| {
                let Some(idx) = state_c.borrow().library_index.clone() else {
                    return;
                };
                let started = std::time::Instant::now();
                match idx.add_paths_to_collection(id, &paths) {
                    Ok(added) => {
                        let name = idx
                            .list_collections()
                            .unwrap_or_default()
                            .into_iter()
                            .find(|c| c.id == id)
                            .map(|c| c.name)
                            .unwrap_or_else(|| "collection".to_string());
                        crate::bench_event!(
                            "collection.drop_add",
                            serde_json::json!({
                                "collection_id": id,
                                "path_count": paths.len(),
                                "new_count": added,
                                "duration_ms": crate::bench::duration_ms(started),
                            })
                        );
                        refresh_c();
                        // If we're viewing this collection, append newly added paths.
                        if state_c.borrow().active_collection == Some(id) {
                            let all_paths = idx.collection_paths(id).unwrap_or_default();
                            load_collection_async(&state_c, &all_paths, id);
                            filmstrip_c.refresh_virtual();
                            let first = state_c
                                .borrow()
                                .library
                                .entry_at(0)
                                .map(|e: ImageEntry| e.path());
                            if let Some(p) = first {
                                state_c.borrow_mut().library.selected_index = Some(0);
                                filmstrip_c.navigate_to(0);
                                viewer_c.load_image(p);
                            }
                        }
                        toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                            "Added {} image{} to \u{201c}{}\u{201d}",
                            added,
                            if added == 1 { "" } else { "s" },
                            name
                        )));
                    }
                    Err(e) => {
                        toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                            "Could not add to collection: {e}"
                        )));
                    }
                }
            });
        }

        // Double-click on filmstrip item → open in default viewer.
        {
            let state_c = state.clone();
            filmstrip.connect_item_activated(move |position| {
                let path = state_c
                    .borrow()
                    .library
                    .entry_at(position)
                    .map(|e: ImageEntry| e.path());
                if let Some(path) = path {
                    let uri = gio::File::for_path(&path).uri();
                    let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
                }
            });
        }

        // Filmstrip selection → viewer load.
        {
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            filmstrip.connect_image_selected(move |index| {
                let path = {
                    let Ok(mut state) = state_c.try_borrow_mut() else {
                        return;
                    };
                    state.library.selected_index = Some(index);
                    state
                        .library
                        .entry_at(index)
                        .map(|entry: ImageEntry| entry.path())
                };
                if let Some(path) = path {
                    viewer_c.load_image(path);
                }

                // Queue prefetch for the images immediately before and after this one.
                trigger_prefetch(&state_c, index);
            });
        }

        // Start draining thumbnail results.
        self.start_thumbnail_poll(state.clone(), filmstrip.clone());
        self.start_hash_poll(state.clone());

        // -----------------------------------------------------------------------
        // Layout: AdwOverlaySplitView (outer) → AdwOverlaySplitView (inner)
        // -----------------------------------------------------------------------

        let menu = Self::build_primary_menu();
        let viewer_menu_btn = Self::make_menu_button(&menu);
        viewer_menu_btn.set_visible(true);

        // Inner split: filmstrip sidebar | viewer content.
        let inner_split = libadwaita::OverlaySplitView::new();
        inner_split.set_sidebar_position(gtk4::PackType::Start);

        let filmstrip_page = libadwaita::NavigationPage::builder()
            .title("Photos")
            .tag("filmstrip")
            .child(&filmstrip)
            .build();
        inner_split.set_sidebar(Some(&filmstrip_page));

        let (
            viewer_header,
            sidebar_toggle,
            preview_title_btn,
            commit_btn,
            discard_btn,
            edit_commit_btn,
            edit_discard_btn,
        ) = self.build_viewer_header(&viewer_menu_btn);

        let upscale_banner = libadwaita::Banner::new("");
        upscale_banner.set_button_label(Some("Dismiss"));
        upscale_banner.set_revealed(false);
        upscale_banner.connect_button_clicked(|banner| {
            banner.set_revealed(false);
        });

        self.setup_actions(&viewer, state.clone(), &upscale_banner);

        let viewer_toolbar = libadwaita::ToolbarView::new();
        viewer_toolbar.add_top_bar(&viewer_header);
        viewer_toolbar.add_top_bar(&upscale_banner);

        // content_stack lives inside viewer_toolbar so the header bar (hamburger,
        // window controls) is always visible regardless of which page is shown.
        content_stack.add_named(&viewer, Some("viewer"));

        if let Some(tag_browser) = tag_browser.as_ref() {
            content_stack.add_named(tag_browser, Some("tags"));
        }
        content_stack.set_visible_child_name("viewer");

        viewer_toolbar.set_content(Some(&content_stack));
        viewer_toolbar.set_top_bar_style(libadwaita::ToolbarStyle::Raised);

        // Give the viewer a reference to the Commit/Discard buttons so the
        // async upscale callback can show/hide them without extra clones.
        viewer.set_comparison_buttons(commit_btn.clone(), discard_btn.clone());
        viewer.set_edit_buttons(edit_commit_btn.clone(), edit_discard_btn.clone());

        // Commit / Discard buttons for the comparison view.
        {
            let viewer_c = viewer.clone();
            preview_title_btn.connect_clicked(move |_| {
                viewer_c.toggle_debug_comparison();
            });
        }
        {
            let viewer_c = viewer.clone();
            commit_btn.connect_clicked(move |_| {
                viewer_c.commit_upscale();
            });
        }
        {
            let viewer_c = viewer.clone();
            discard_btn.connect_clicked(move |_| {
                viewer_c.discard_upscale();
            });
        }
        {
            let viewer_c = viewer.clone();
            edit_commit_btn.connect_clicked(move |_| {
                viewer_c.save_edit();
            });
        }
        {
            let viewer_c = viewer.clone();
            edit_discard_btn.connect_clicked(move |_| {
                viewer_c.discard_edit();
            });
        }

        let viewer_page = libadwaita::NavigationPage::builder()
            .title("Preview")
            .tag("viewer")
            .child(&viewer_toolbar)
            .build();
        inner_split.set_content(Some(&viewer_page));

        // Update the NavigationPage title to reflect which panel is active.
        {
            let viewer_page_c = viewer_page.clone();
            let preview_title_btn_c = preview_title_btn.clone();
            content_stack.connect_visible_child_notify(move |stack| {
                let name = stack.visible_child_name().unwrap_or_default();
                let is_tags = name == "tags";
                viewer_page_c.set_title(if is_tags { "Tags" } else { "Preview" });
                preview_title_btn_c.set_label(if is_tags { "Tags" } else { "Preview" });
                preview_title_btn_c.set_sensitive(!is_tags);
            });
        }

        // Outer split: explorer sidebar | inner_split.
        let sidebar_overlay = gtk4::Overlay::new();
        sidebar_overlay.set_child(Some(&sidebar));

        let ops_indicator = OpsIndicator::new();
        ops_indicator.set_halign(gtk4::Align::Fill);
        ops_indicator.set_valign(gtk4::Align::End);
        ops_indicator.set_margin_start(12);
        ops_indicator.set_margin_end(12);
        ops_indicator.set_margin_bottom(16);
        sidebar_overlay.add_overlay(&ops_indicator);

        let sidebar_page = libadwaita::NavigationPage::builder()
            .title("Library")
            .tag("sidebar")
            .child(&sidebar_overlay)
            .build();
        outer_split.set_sidebar(Some(&sidebar_page));

        let content_page = libadwaita::NavigationPage::builder()
            .title("Image Library")
            .tag("content")
            .child(&inner_split)
            .build();
        outer_split.set_content(Some(&content_page));

        sidebar_toggle.set_active(true);
        sidebar_toggle
            .bind_property("active", &outer_split, "show-sidebar")
            .flags(glib::BindingFlags::SYNC_CREATE | glib::BindingFlags::BIDIRECTIONAL)
            .build();

        let outer_split_c = outer_split.clone();
        inner_split.connect_collapsed_notify(move |inner| {
            if inner.is_collapsed() {
                outer_split_c.set_collapsed(true);
                outer_split_c.set_show_sidebar(false);
            }
        });

        // -----------------------------------------------------------------------
        // Adaptive breakpoints
        // -----------------------------------------------------------------------

        // < 1500px: collapse explorer sidebar.
        let bp_sidebar = libadwaita::Breakpoint::new(
            libadwaita::BreakpointCondition::parse("max-width: 1500px").unwrap(),
        );
        bp_sidebar.add_setter(&outer_split, "collapsed", Some(&true.to_value()));
        self.add_breakpoint(bp_sidebar);

        // < 800px: collapse filmstrip into overlay.
        let bp_filmstrip = libadwaita::Breakpoint::new(
            libadwaita::BreakpointCondition::parse("max-width: 800px").unwrap(),
        );
        bp_filmstrip.add_setter(&inner_split, "collapsed", Some(&true.to_value()));
        self.add_breakpoint(bp_filmstrip);

        toast_overlay.set_child(Some(&outer_split));

        self.set_content(Some(&toast_overlay));

        // Drive the indicator from the ops event channel.
        {
            let indicator = ops_indicator.clone();
            glib::MainContext::default().spawn_local(async move {
                use crate::ops::queue::OpEvent;
                while let Ok(event) = ops_rx.recv().await {
                    match event {
                        OpEvent::Added { id, title } => indicator.push_op(id, &title),
                        OpEvent::Progress { id, fraction } => indicator.update_op(id, fraction),
                        OpEvent::Completed(id) => indicator.complete_op(id),
                        OpEvent::Failed { id, msg } => indicator.fail_op(id, &msg),
                        OpEvent::Dismissed(id) => indicator.remove_op(id),
                    }
                }
            });
        }

        let builder = gtk4::Builder::from_resource("/io/github/hebbihebb/Sharpr/help-overlay.ui");
        let help_overlay: gtk4::ShortcutsWindow = builder.object("help_overlay").unwrap();
        self.set_help_overlay(Some(&help_overlay));
        filmstrip.set_search_capture_widget(self.upcast_ref::<gtk4::Widget>());

        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            let sidebar_c = sidebar.clone();
            let content_stack = content_stack.clone();
            let pending_search = Rc::new(Cell::new(None::<glib::SourceId>));
            filmstrip.connect_search_changed(move |text| {
                content_stack.set_visible_child_name("viewer");
                sidebar_c.set_search_selected(!text.trim().is_empty());
                sidebar_c.set_tags_selected(false);
                sidebar_c.set_quality_selected(None);
                filmstrip_c.show_autocomplete(vec![]);

                if let Some(source_id) = pending_search.take() {
                    source_id.remove();
                }

                let text = text.trim().to_string();
                if text.is_empty() {
                    load_virtual_async(&state_c, &[]);
                    viewer_c.clear();
                    filmstrip_c.refresh_virtual();
                    return;
                }

                if text.len() < 2 {
                    return;
                }

                let filmstrip_timeout = filmstrip_c.clone();
                let viewer_timeout = viewer_c.clone();
                let state_timeout = state_c.clone();
                let sidebar_timeout = sidebar_c.clone();
                let pending_search_timeout = pending_search.clone();
                let content_stack_timeout = content_stack.clone();
                let source_id =
                    glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
                        pending_search_timeout.set(None);

                        let Some(tags) = state_timeout.borrow().tags.clone() else {
                            return glib::ControlFlow::Break;
                        };

                        let query = text.clone();
                        let db_query = query.clone();
                        let (tx, rx) = async_channel::bounded::<Vec<PathBuf>>(1);
                        rayon::spawn(move || {
                            let _ = tx.send_blocking(tags.search_paths(&db_query));
                        });

                        let filmstrip_rx = filmstrip_timeout.clone();
                        let viewer_rx = viewer_timeout.clone();
                        let state_rx = state_timeout.clone();
                        let sidebar_rx = sidebar_timeout.clone();
                        let content_stack_rx = content_stack_timeout.clone();
                        glib::MainContext::default().spawn_local(async move {
                            if let Ok(db_paths) = rx.recv().await {
                                let q = query.to_lowercase();
                                let library_paths: Vec<PathBuf> = {
                                    let state = state_rx.borrow();
                                    (0..state.library.image_count())
                                        .filter_map(|i| state.library.entry_at(i))
                                        .map(|e| e.path())
                                        .filter(|p| {
                                            p.file_name()
                                                .and_then(|n| n.to_str())
                                                .map(|n| n.to_lowercase().contains(&q))
                                                .unwrap_or(false)
                                        })
                                        .collect()
                                };

                                let mut seen = std::collections::HashSet::new();
                                let merged: Vec<PathBuf> = db_paths
                                    .iter()
                                    .chain(library_paths.iter())
                                    .filter(|p| seen.insert((*p).clone()))
                                    .cloned()
                                    .collect();

                                load_virtual_async(&state_rx, &merged);
                                content_stack_rx.set_visible_child_name("viewer");
                                sidebar_rx.set_search_selected(true);
                                sidebar_rx.set_duplicates_selected(false);
                                sidebar_rx.set_tags_selected(false);
                                sidebar_rx.set_quality_selected(None);
                                viewer_rx.clear();
                                filmstrip_rx.refresh_virtual();
                                let first_path = state_rx
                                    .borrow()
                                    .library
                                    .entry_at(0)
                                    .map(|e: ImageEntry| e.path());
                                if let Some(path) = first_path {
                                    state_rx.borrow_mut().library.selected_index = Some(0);
                                    filmstrip_rx.navigate_to(0);
                                    viewer_rx.load_image(path);
                                }
                            }
                        });

                        glib::ControlFlow::Break
                    });
                pending_search.set(Some(source_id));
            });
        }

        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            let sidebar_c = sidebar.clone();
            let content_stack = content_stack.clone();
            filmstrip.connect_search_activate(move |tag| {
                let tag = tag.trim();
                if tag.is_empty() {
                    return;
                }
                let Some(tags) = state_c.borrow().tags.clone() else {
                    return;
                };
                let tag = tag.to_string();
                let (tx, rx) = async_channel::bounded::<Vec<PathBuf>>(1);
                rayon::spawn(move || {
                    let _ = tx.send_blocking(tags.search_paths(&tag));
                });
                let filmstrip_rx = filmstrip_c.clone();
                let viewer_rx = viewer_c.clone();
                let state_rx = state_c.clone();
                let sidebar_rx = sidebar_c.clone();
                let content_stack_rx = content_stack.clone();
                glib::MainContext::default().spawn_local(async move {
                    if let Ok(paths) = rx.recv().await {
                        load_virtual_async(&state_rx, &paths);
                        content_stack_rx.set_visible_child_name("viewer");
                        sidebar_rx.set_search_selected(true);
                        sidebar_rx.set_duplicates_selected(false);
                        sidebar_rx.set_tags_selected(false);
                        sidebar_rx.set_quality_selected(None);
                        viewer_rx.clear();
                        filmstrip_rx.refresh_virtual();
                        let first_path = state_rx
                            .borrow()
                            .library
                            .entry_at(0)
                            .map(|e: ImageEntry| e.path());
                        if let Some(path) = first_path {
                            state_rx.borrow_mut().library.selected_index = Some(0);
                            filmstrip_rx.navigate_to(0);
                            viewer_rx.load_image(path);
                        }
                    }
                });
            });
        }

        {
            let state_c = state.clone();
            let sidebar_c = sidebar.clone();
            let suppress_search_restore_c = suppress_search_restore.clone();
            let open_folder_c = open_folder.clone();
            filmstrip.connect_search_dismissed(move || {
                if suppress_search_restore_c.replace(false) {
                    return;
                }
                sidebar_c.set_search_selected(false);
                sidebar_c.set_tags_selected(false);
                sidebar_c.set_quality_selected(None);
                if state_c.borrow().library.current_folder.is_none() {
                    // Extract last_folder in a separate let so the Ref<AppState> temporary
                    // is dropped at the semicolon — not held alive through the if let body
                    // where open_folder_c calls borrow_mut() and would panic.
                    let last_folder = state_c.borrow().settings.last_folder.clone();
                    if let Some(folder) = last_folder {
                        if folder.is_dir() {
                            open_folder_c(folder);
                        }
                    }
                }
            });
        }

        if let Some(tag_browser) = tag_browser {
            let content_stack_c = content_stack.clone();
            let sidebar_c = sidebar.clone();
            let filmstrip_c = filmstrip.clone();
            tag_browser.connect_tag_activated(move |tag| {
                content_stack_c.set_visible_child_name("viewer");
                sidebar_c.set_tags_selected(false);
                sidebar_c.set_quality_selected(None);
                filmstrip_c.emit_search_activate(tag);
            });
        }

        // -----------------------------------------------------------------------
        // Alt+Left / Alt+Right — navigate between images.
        // Scoped to the window so it fires regardless of focus position.
        // -----------------------------------------------------------------------
        // "Add to Collection…" context menu → pick / create collection → add paths.
        {
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            let window_weak = self.downgrade();
            filmstrip.connect_add_to_collection_requested(move |paths| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                let Some(idx) = state_c.borrow().library_index.clone() else {
                    toast_overlay_c.add_toast(libadwaita::Toast::new("Library index unavailable"));
                    return;
                };
                let collections = idx.list_collections().unwrap_or_default();

                let dialog = libadwaita::AlertDialog::new(Some("Add to Collection"), None);
                dialog.add_response("done", "Done");
                dialog.set_default_response(Some("done"));
                dialog.set_close_response("done");
                dialog.set_response_appearance("done", libadwaita::ResponseAppearance::Suggested);

                let list_box = gtk4::ListBox::new();
                list_box.add_css_class("boxed-list");
                list_box.set_selection_mode(gtk4::SelectionMode::Single);

                // "New Collection…" entry at top
                let new_row = gtk4::ListBoxRow::new();
                let new_label = gtk4::Label::new(Some("New Collection\u{2026}"));
                new_label.set_halign(gtk4::Align::Start);
                new_label.set_margin_top(8);
                new_label.set_margin_bottom(8);
                new_label.set_margin_start(12);
                new_row.set_child(Some(&new_label));
                list_box.append(&new_row);

                for coll in &collections {
                    let row = gtk4::ListBoxRow::new();
                    unsafe {
                        row.set_data("collection-id", coll.id);
                    }
                    let lbl = gtk4::Label::new(Some(&coll.name));
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
                let done_shortcut = gtk4::EventControllerKey::new();
                let dialog_weak = dialog.downgrade();
                done_shortcut.connect_key_pressed(move |_, key, _, _| {
                    if key == gtk4::gdk::Key::Return || key == gtk4::gdk::Key::KP_Enter {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.close();
                        }
                        return glib::Propagation::Stop;
                    }
                    glib::Propagation::Proceed
                });
                extra.add_controller(done_shortcut);
                dialog.set_extra_child(Some(&extra));

                let paths_c = paths.clone();
                let state_d = state_c.clone();
                let toast_d = toast_overlay_c.clone();
                let refresh_d = refresh_c.clone();
                let win_weak2 = win.downgrade();
                list_box.connect_row_activated(move |_, row| {
                    let coll_id: Option<i64> =
                        unsafe { row.data("collection-id").map(|p| *p.as_ref()) };
                    if coll_id.is_none() {
                        // New collection
                        let Some(win2) = win_weak2.upgrade() else {
                            return;
                        };
                        let new_dialog = libadwaita::AlertDialog::new(Some("New Collection"), None);
                        new_dialog.add_response("cancel", "Cancel");
                        new_dialog.add_response("create", "Create");
                        new_dialog.set_default_response(Some("create"));
                        new_dialog.set_close_response("cancel");
                        new_dialog.set_response_appearance(
                            "create",
                            libadwaita::ResponseAppearance::Suggested,
                        );
                        let entry = gtk4::Entry::new();
                        entry.set_placeholder_text(Some("Collection name"));
                        let eb = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                        eb.set_margin_top(6);
                        eb.append(&entry);
                        new_dialog.set_extra_child(Some(&eb));
                        let paths_cc = paths_c.clone();
                        let state_dd = state_d.clone();
                        let toast_dd = toast_d.clone();
                        let refresh_dd = refresh_d.clone();
                        let create_collection = Rc::new(move |name: String| {
                            if let Some(idx) = state_dd.borrow().library_index.clone() {
                                let started = std::time::Instant::now();
                                match idx.create_collection(&name) {
                                    Ok(coll) => {
                                        let added = idx
                                            .add_paths_to_collection(coll.id, &paths_cc)
                                            .unwrap_or(0);
                                        crate::bench_event!(
                                            "collection.create",
                                            serde_json::json!({
                                                "collection_id": coll.id,
                                                "name": coll.name,
                                                "duration_ms": crate::bench::duration_ms(started),
                                            })
                                        );
                                        crate::bench_event!(
                                            "collection.add_paths",
                                            serde_json::json!({
                                                "collection_id": coll.id,
                                                "path_count": paths_cc.len(),
                                                "new_count": added,
                                                "duration_ms": crate::bench::duration_ms(started),
                                            })
                                        );
                                        refresh_dd();
                                        toast_dd.add_toast(libadwaita::Toast::new(&format!(
                                            "Added {} image{} to \u{201c}{}\u{201d}",
                                            added,
                                            if added == 1 { "" } else { "s" },
                                            coll.name
                                        )));
                                    }
                                    Err(e) => {
                                        toast_dd.add_toast(libadwaita::Toast::new(&format!(
                                            "Could not create collection: {e}"
                                        )));
                                    }
                                }
                            }
                        });
                        let entry_c = entry.clone();
                        let create_c = create_collection.clone();
                        new_dialog.connect_response(None, move |_, response| {
                            if response == "create" {
                                create_c(entry_c.text().to_string());
                            }
                        });
                        let new_dialog_weak = new_dialog.downgrade();
                        entry.connect_activate(move |entry| {
                            create_collection(entry.text().to_string());
                            if let Some(dialog) = new_dialog_weak.upgrade() {
                                dialog.close();
                            }
                        });
                        new_dialog.present(Some(&win2));
                        entry.grab_focus();
                        return;
                    }
                    let id = coll_id.unwrap();
                    if let Some(idx) = state_d.borrow().library_index.clone() {
                        let started = std::time::Instant::now();
                        match idx.add_paths_to_collection(id, &paths_c) {
                            Ok(added) => {
                                let name = idx
                                    .list_collections()
                                    .unwrap_or_default()
                                    .into_iter()
                                    .find(|c| c.id == id)
                                    .map(|c| c.name)
                                    .unwrap_or_else(|| "collection".to_string());
                                crate::bench_event!(
                                    "collection.add_paths",
                                    serde_json::json!({
                                        "collection_id": id,
                                        "path_count": paths_c.len(),
                                        "new_count": added,
                                        "duration_ms": crate::bench::duration_ms(started),
                                    })
                                );
                                refresh_d();
                                toast_d.add_toast(libadwaita::Toast::new(&format!(
                                    "Added {} image{} to \u{201c}{}\u{201d}",
                                    added,
                                    if added == 1 { "" } else { "s" },
                                    name
                                )));
                            }
                            Err(e) => {
                                toast_d.add_toast(libadwaita::Toast::new(&format!(
                                    "Could not add to collection: {e}"
                                )));
                            }
                        }
                    }
                });

                dialog.present(Some(&win));
            });
        }

        // "Remove from Collection" context menu → remove from active collection → reload view.
        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            filmstrip.connect_remove_from_collection_requested(move |paths| {
                let Some(idx) = state_c.borrow().library_index.clone() else {
                    return;
                };
                let Some(id) = state_c.borrow().active_collection else {
                    return;
                };
                let started = std::time::Instant::now();
                match idx.remove_paths_from_collection(id, &paths) {
                    Ok(removed) => {
                        crate::bench_event!(
                            "collection.remove_paths",
                            serde_json::json!({
                                "collection_id": id,
                                "path_count": paths.len(),
                                "removed_count": removed,
                                "duration_ms": crate::bench::duration_ms(started),
                            })
                        );
                        let remaining = idx.collection_paths(id).unwrap_or_default();
                        load_collection_async(&state_c, &remaining, id);
                        state_c.borrow_mut().selected_paths.clear();
                        filmstrip_c.refresh_virtual();
                        let first = state_c
                            .borrow()
                            .library
                            .entry_at(0)
                            .map(|e: ImageEntry| e.path());
                        if let Some(p) = first {
                            state_c.borrow_mut().library.selected_index = Some(0);
                            filmstrip_c.navigate_to(0);
                            viewer_c.load_image(p);
                        } else {
                            viewer_c.clear();
                        }
                        refresh_c();
                        if let Ok(colls) = idx.list_collections() {
                            sidebar_c.refresh_collections(&colls);
                        }
                        toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                            "Removed {} image{} from collection",
                            removed,
                            if removed == 1 { "" } else { "s" }
                        )));
                    }
                    Err(e) => {
                        toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                            "Could not remove from collection: {e}"
                        )));
                    }
                }
            });
        }

        self.setup_nav_shortcuts(
            state.clone(),
            filmstrip.clone(),
            viewer.clone(),
            outer_split.clone(),
        );

        // -----------------------------------------------------------------------
        // Restore last folder
        // -----------------------------------------------------------------------
        let last_folder = state.borrow().settings.last_folder.clone();
        let start_folder = last_folder
            .filter(|p| p.is_dir())
            .or_else(|| sidebar.first_folder_path());

        // Defer by one idle cycle so widgets are realized before we call
        // open_folder(). Without this, smart_list.unselect_all() fires before
        // realization and the Duplicates row stays visually highlighted.
        if let Some(folder) = start_folder {
            glib::idle_add_local_once(move || {
                open_folder(folder);
            });
        }
    }

    /// Wire Alt+Left / Alt+Right to advance the image selection.
    fn setup_nav_shortcuts(
        &self,
        state: Rc<RefCell<AppState>>,
        filmstrip: FilmstripPane,
        viewer: ViewerPane,
        outer_split: libadwaita::OverlaySplitView,
    ) {
        let shortcuts = gtk4::ShortcutController::new();
        // Managed scope means the window itself handles these even when a child
        // widget has focus.
        shortcuts.set_scope(gtk4::ShortcutScope::Managed);

        let make_action = |state: Rc<RefCell<AppState>>,
                           filmstrip: FilmstripPane,
                           viewer: ViewerPane,
                           delta: i32| {
            gtk4::CallbackAction::new(move |_, _| {
                let new_index = {
                    let Ok(mut st) = state.try_borrow_mut() else {
                        return glib::Propagation::Proceed;
                    };
                    st.library.navigate(delta)
                };
                if let Some(index) = new_index {
                    filmstrip.navigate_to(index);
                    let path = state
                        .borrow()
                        .library
                        .entry_at(index)
                        .map(|e: ImageEntry| e.path());
                    if let Some(p) = path {
                        viewer.load_image(p);
                    }
                }
                glib::Propagation::Stop
            })
        };

        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Alt>Left").unwrap()),
            Some(make_action(
                state.clone(),
                filmstrip.clone(),
                viewer.clone(),
                -1,
            )),
        ));
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Alt>Right").unwrap()),
            Some(make_action(
                state.clone(),
                filmstrip.clone(),
                viewer.clone(),
                1,
            )),
        ));

        // F11 — Toggle fullscreen.
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("F11").unwrap()),
            Some(gtk4::CallbackAction::new(move |widget, _| {
                if let Some(win) = widget.downcast_ref::<gtk4::Window>() {
                    if win.is_fullscreen() {
                        win.unfullscreen();
                    } else {
                        win.fullscreen();
                    }
                }
                glib::Propagation::Stop
            })),
        ));

        // F9 — Toggle Library sidebar.
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("F9").unwrap()),
            Some(gtk4::CallbackAction::new(move |_, _| {
                outer_split.set_show_sidebar(!outer_split.shows_sidebar());
                glib::Propagation::Stop
            })),
        ));

        // Del — Move selected image to trash.
        {
            let state_d = state.clone();
            let filmstrip_d = filmstrip.clone();
            let viewer_d = viewer.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("Delete").unwrap()),
                Some(gtk4::CallbackAction::new(move |_, _| {
                    let (path, index) = {
                        let Ok(st) = state_d.try_borrow() else {
                            return glib::Propagation::Proceed;
                        };
                        let index = match st.library.selected_index {
                            Some(i) => i,
                            None => return glib::Propagation::Proceed,
                        };
                        let path = match st.library.entry_at(index) {
                            Some(e) => e.path(),
                            None => return glib::Propagation::Proceed,
                        };
                        (path, index)
                    };

                    if gio::File::for_path(&path)
                        .trash(None::<&gio::Cancellable>)
                        .is_ok()
                    {
                        if let Some(tags) = state_d.borrow().tags.clone() {
                            tags.remove_path(&path);
                        }
                        state_d.borrow_mut().library.remove_path(&path);
                        let new_count = state_d.borrow().library.image_count();
                        if new_count == 0 {
                            viewer_d.clear();
                        } else {
                            let new_index = index.min(new_count - 1);
                            filmstrip_d.navigate_to(new_index);
                            let next_path = state_d
                                .borrow()
                                .library
                                .entry_at(new_index)
                                .map(|e: ImageEntry| e.path());
                            if let Some(p) = next_path {
                                viewer_d.load_image(p);
                            }
                        }
                    }
                    glib::Propagation::Stop
                })),
            ));
        }

        // Context-menu "Move to Trash" from filmstrip (duplicates mode).
        {
            let state_tr = state.clone();
            let filmstrip_tr = filmstrip.clone();
            let viewer_tr = viewer.clone();
            filmstrip.connect_trash_requested(move |path| {
                if gio::File::for_path(&path)
                    .trash(None::<&gio::Cancellable>)
                    .is_ok()
                {
                    if let Some(tags) = state_tr.borrow().tags.clone() {
                        tags.remove_path(&path);
                    }
                    state_tr.borrow_mut().library.remove_path(&path);
                    let new_count = state_tr.borrow().library.image_count();
                    if new_count == 0 {
                        viewer_tr.clear();
                    } else {
                        let index = state_tr.borrow().library.selected_index.unwrap_or(0);
                        let new_index = index.min(new_count - 1);
                        filmstrip_tr.navigate_to(new_index);
                        let next_path = state_tr
                            .borrow()
                            .library
                            .entry_at(new_index)
                            .map(|e: ImageEntry| e.path());
                        if let Some(p) = next_path {
                            viewer_tr.load_image(p);
                        }
                    }
                }
            });
        }

        // Ctrl+T — open the tag editor for the selected image.
        {
            let state_t = state.clone();
            let viewer_t = viewer.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("<Control>T").unwrap()),
                Some(gtk4::CallbackAction::new(move |widget, _| {
                    if let Some(window) = widget.downcast_ref::<gtk4::Window>() {
                        if let Some(focus) = gtk4::prelude::GtkWindowExt::focus(window) {
                            if focus.is::<gtk4::Text>() || focus.is::<gtk4::SearchEntry>() {
                                return glib::Propagation::Proceed;
                            }
                        }
                    }

                    let has_selection = state_t
                        .try_borrow()
                        .ok()
                        .and_then(|state| state.library.selected_index)
                        .is_some();
                    if !has_selection {
                        return glib::Propagation::Stop;
                    }

                    viewer_t.open_tag_popover();
                    glib::Propagation::Stop
                })),
            ));
        }

        self.add_controller(shortcuts);
    }

    /// Drain thumbnail results from the worker pool on the GLib main context.
    /// For each result: construct a `MemoryTexture` on the main thread,
    /// find the matching `ImageEntry` in the store, and update it.
    fn bump_thumbnail_generation(&self, reason: &str) -> Option<u64> {
        let gen = self
            .imp()
            .thumbnail_worker
            .borrow()
            .as_ref()
            .map(|worker| worker.bump_generation())?;
        crate::bench_event!(
            "thumbnail.generation_bump",
            serde_json::json!({
                "gen": gen,
                "reason": reason,
            }),
        );
        Some(gen)
    }

    fn current_thumbnail_generation(&self) -> Option<u64> {
        self.imp()
            .thumbnail_worker
            .borrow()
            .as_ref()
            .map(|worker| worker.current_generation())
    }

    fn complete_thumbnail_ops(&self) {
        for (_, op_state) in self.imp().thumbnail_ops.borrow_mut().drain() {
            op_state.handle.complete();
        }
    }

    fn start_thumbnail_poll(&self, state: Rc<RefCell<AppState>>, filmstrip: FilmstripPane) {
        let result_rx = self.imp().result_rx.borrow().clone();
        let Some(rx) = result_rx else { return };
        let window_weak = self.downgrade();

        glib::MainContext::default().spawn_local(async move {
            while let Ok(result) = rx.recv().await {
                use gdk4::{MemoryFormat, MemoryTexture};
                use glib::Bytes;

                let result_path = result.path.clone();
                let source = result.source;
                let worker_ms = result.worker_ms;
                let bytes = Bytes::from_owned(result.rgba_bytes);
                let texture = MemoryTexture::new(
                    result.width as i32,
                    result.height as i32,
                    MemoryFormat::R8g8b8a8,
                    &bytes,
                    (result.width * 4) as usize,
                );

                // Use the path→index lookup built by LibraryManager to avoid a
                // linear scan for every completed thumbnail.
                let mut applied_index = None;
                {
                    let st = state.borrow();
                    if let Some(idx) = st.library.index_of_path(&result_path) {
                        if let Some(entry) = st.library.entry_at(idx) {
                            entry.set_thumbnail(Some(texture.clone().upcast::<gdk4::Texture>()));
                            applied_index = Some(idx);
                        }
                    } else {
                        crate::bench_event!(
                            "thumbnail.apply_skipped",
                            serde_json::json!({
                                "path": result_path.display().to_string(),
                                "source": source,
                                "reason": "not_in_active_view",
                            }),
                        );
                    }
                }

                // Cache the texture in LibraryManager (needs mut borrow).
                {
                    let mut st = state.borrow_mut();
                    st.library.insert_thumbnail(
                        result_path.clone(),
                        texture.upcast(),
                        applied_index.is_some(),
                    );
                    if let Some(idx) = applied_index {
                        let (active_cache_len, global_cache_len, global_cache_max) =
                            st.library.thumbnail_cache_stats();
                        crate::bench_event!(
                            "thumbnail.apply",
                            serde_json::json!({
                                "path": result_path.display().to_string(),
                                "index": idx,
                                "source": source,
                                "worker_ms": worker_ms,
                                "active_cache_len": active_cache_len,
                                "global_cache_len": global_cache_len,
                                "global_cache_max": global_cache_max,
                            }),
                        );
                    }
                }

                filmstrip.mark_thumbnail_ready(&result_path);

                if let Some(window) = window_weak.upgrade() {
                    if let Some(folder) = result_path.parent().map(|p| p.to_path_buf()) {
                        let mut thumbnail_ops = window.imp().thumbnail_ops.borrow_mut();
                        if let Some(op_state) = thumbnail_ops.get_mut(&folder) {
                            op_state.received += 1;
                            op_state.handle.progress(Some(
                                op_state.received as f32 / op_state.total.max(1) as f32,
                            ));
                            if op_state.total > 0 && op_state.received >= op_state.total {
                                if let Some(completed) = thumbnail_ops.remove(&folder) {
                                    completed.handle.complete();
                                }
                            }
                        }
                    }
                }
            }

            if let Some(window) = window_weak.upgrade() {
                for (_, op_state) in window.imp().thumbnail_ops.borrow_mut().drain() {
                    op_state.handle.complete();
                }
            }
        });
    }

    fn start_hash_poll(&self, state: Rc<RefCell<AppState>>) {
        let hash_rx = self.imp().hash_result_rx.borrow().clone();
        let Some(rx) = hash_rx else { return };

        glib::MainContext::default().spawn_local(async move {
            while let Ok(result) = rx.recv().await {
                let path = result.path.clone();
                let hash = result.hash;
                if path_is_disabled(&path, &state.borrow().disabled_folders) {
                    crate::bench_event!(
                        "hash.apply_skipped",
                        serde_json::json!({
                            "path": path.display().to_string(),
                            "reason": "disabled_folder",
                        }),
                    );
                    continue;
                }
                state.borrow_mut().library.insert_hash(result.path, hash);
                crate::bench_event!(
                    "hash.apply",
                    serde_json::json!({
                        "path": path.display().to_string(),
                    }),
                );
                // Persist phash to the index so duplicate detection is cross-session.
                if let Some(index) = state.borrow().library_index.clone() {
                    let path_clone = path.clone();
                    rayon::spawn(move || {
                        if let Err(err) = index.update_image_phash(&path_clone, hash) {
                            crate::bench_event!(
                                "hash.index_persist_fail",
                                serde_json::json!({
                                    "path": path_clone.display().to_string(),
                                    "error": err.to_string(),
                                }),
                            );
                        }
                    });
                }
                if let Some(tags_arc) = state.borrow().tags.clone() {
                    rayon::spawn(move || {
                        let started = Instant::now();
                        let meta = crate::metadata::exif::ImageMetadata::load(&path);
                        let tag_list = crate::tags::indexer::auto_tags(&path, &meta);
                        tags_arc.insert_auto_tags(&path, &tag_list);
                        crate::bench_event!(
                            "tags.auto_index.finish",
                            serde_json::json!({
                                "path": path.display().to_string(),
                                "tag_count": tag_list.len(),
                                "duration_ms": crate::bench::duration_ms(started),
                            }),
                        );
                    });
                }
            }
        });
    }

    fn build_primary_menu() -> gio::Menu {
        let menu = gio::Menu::new();

        let view_section = gio::Menu::new();
        let zoom_subsection = gio::Menu::new();
        zoom_subsection.append(Some("Fit to Window"), Some("win.zoom-mode::fit"));
        zoom_subsection.append(Some("1:1 Pixels"), Some("win.zoom-mode::1:1"));
        view_section.append_section(None, &zoom_subsection);
        view_section.append(Some("Show Overlay"), Some("win.show-metadata"));
        menu.append_section(Some("View"), &view_section);

        let transform_section = gio::Menu::new();
        transform_section.append(Some("Rotate 90° Clockwise"), Some("win.rotate-cw"));
        transform_section.append(Some("Rotate 90° Counter-Clockwise"), Some("win.rotate-ccw"));
        transform_section.append(Some("Flip Horizontal"), Some("win.flip-h"));
        transform_section.append(Some("Flip Vertical"), Some("win.flip-v"));
        menu.append_section(Some("Rotate & Flip"), &transform_section);

        let upscale_section = gio::Menu::new();
        upscale_section.append(Some("Upscale Image…"), Some("win.upscale"));
        menu.append_section(Some("AI Upscale"), &upscale_section);

        let app_section = gio::Menu::new();
        app_section.append(Some("Keyboard Shortcuts"), Some("win.show-help-overlay"));
        app_section.append(Some("Manual"), Some("win.show-manual"));
        app_section.append(Some("Preferences"), Some("win.show-preferences"));
        app_section.append(Some("About Sharpr"), Some("app.about"));
        menu.append_section(Some("App"), &app_section);

        menu
    }

    fn make_menu_button(menu: &gio::Menu) -> gtk4::MenuButton {
        let popover = gtk4::PopoverMenu::from_model(Some(menu));
        let btn = gtk4::MenuButton::new();
        btn.set_icon_name("open-menu-symbolic");
        btn.set_tooltip_text(Some("Main Menu"));
        btn.set_popover(Some(&popover));
        btn.add_css_class("flat");
        btn
    }

    fn setup_actions(
        &self,
        viewer: &ViewerPane,
        state: Rc<RefCell<AppState>>,
        upscale_banner: &libadwaita::Banner,
    ) {
        let zoom_initial = match viewer.zoom_mode() {
            ZoomMode::Fit => "fit",
            ZoomMode::OneToOne => "1:1",
        };
        let zoom_action = gio::SimpleAction::new_stateful(
            "zoom-mode",
            Some(glib::VariantTy::STRING),
            &zoom_initial.to_variant(),
        );
        {
            let viewer_weak = viewer.downgrade();
            zoom_action.connect_activate(move |action, param| {
                let (Some(viewer), Some(param)) = (viewer_weak.upgrade(), param) else {
                    return;
                };
                let mode = if param.str() == Some("1:1") {
                    ZoomMode::OneToOne
                } else {
                    ZoomMode::Fit
                };
                viewer.set_zoom_mode(mode);
                action.set_state(param);
            });
        }
        self.add_action(&zoom_action);

        let meta_action = gio::SimpleAction::new_stateful(
            "show-metadata",
            None,
            &viewer.metadata_visible().to_variant(),
        );
        {
            let viewer_weak = viewer.downgrade();
            meta_action.connect_activate(move |action, _| {
                let Some(viewer) = viewer_weak.upgrade() else {
                    return;
                };
                let new_val = !action.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
                viewer.set_metadata_visible(new_val);
                action.set_state(&new_val.to_variant());
            });
        }
        self.add_action(&meta_action);

        let manual_action = gio::SimpleAction::new("show-manual", None);
        {
            let window_weak = self.downgrade();
            manual_action.connect_activate(move |_, _| {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                crate::ui::help_window::show_help_window(&window);
            });
        }
        self.add_action(&manual_action);

        let pref_action = gio::SimpleAction::new("show-preferences", None);
        {
            let window_weak = self.downgrade();
            let state_c = state.clone();
            pref_action.connect_activate(move |_, _| {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                let settings = state_c.borrow().settings.clone();
                let prefs = build_preferences_window(&settings, &window);
                prefs.present();
            });
        }
        self.add_action(&pref_action);

        for name in ["rotate-cw", "rotate-ccw", "flip-h", "flip-v"] {
            let a = gio::SimpleAction::new(name, None);
            let viewer_weak = viewer.downgrade();
            let op = name.to_owned();
            a.connect_activate(move |_, _| {
                if let Some(v) = viewer_weak.upgrade() {
                    v.apply_transform(&op);
                }
            });
            self.add_action(&a);
        }

        let upscale_action = gio::SimpleAction::new("upscale", None);
        {
            let state_c = state.clone();
            let banner_c = upscale_banner.clone();
            let viewer_weak = viewer.downgrade();
            let action_weak = upscale_action.downgrade();
            upscale_action.connect_activate(move |_action, _| {
                let Some(viewer) = viewer_weak.upgrade() else { return };

                // For CLI, lazily detect the binary once.
                let (saved_backend, saved_cli_model, saved_onnx_model, comfyui_enabled, ops_queue) = {
                    let mut st = state_c.borrow_mut();
                    if st.upscale_binary.is_none() {
                        st.upscale_binary = AppSettings::load()
                            .upscaler_binary_path
                            .filter(|path| path.is_file());
                    }
                    if st.upscale_binary.is_none() {
                        st.upscale_binary = UpscaleDetector::find_realesrgan();
                    }
                    (
                        st.settings.upscale_backend.clone(),
                        st.settings.upscaler_default_model.clone(),
                        st.settings.onnx_upscale_model.clone(),
                        st.settings.comfyui_enabled,
                        st.ops.clone(),
                    )
                };

                let backend_kind = UpscaleBackendKind::from_settings(&saved_backend);
                if backend_kind == UpscaleBackendKind::Cli
                    && state_c.borrow().upscale_binary.is_none()
                {
                    banner_c.set_title(
                        "AI upscaling requires a supported Vulkan backend such as upscayl-bin or realesrgan-ncnn-vulkan.",
                    );
                    banner_c.set_revealed(true);
                    return;
                }
                banner_c.set_revealed(false);

                let path = state_c
                    .borrow()
                    .library
                    .selected_entry()
                    .map(|e: ImageEntry| e.path());
                if let Some(p) = path {
                    let dialog = libadwaita::AlertDialog::new(Some("Upscale Image"), None);
                    dialog.add_response("cancel", "Cancel");
                    dialog.add_response("upscale", "Upscale");
                    dialog.set_default_response(Some("upscale"));
                    dialog.set_close_response("cancel");
                    dialog.set_response_appearance(
                        "upscale",
                        libadwaita::ResponseAppearance::Suggested,
                    );

                    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
                    content.set_margin_top(6);
                    content.set_margin_bottom(6);

                    // ── Backend selection ────────────────────────────────────
                    let backend_label = gtk4::Label::new(Some("Backend"));
                    backend_label.set_halign(gtk4::Align::Start);
                    content.append(&backend_label);

                    let cli_btn = gtk4::CheckButton::with_label(
                        "CLI Upscaler (realesrgan, GPU-accelerated)",
                    );
                    let onnx_btn = gtk4::CheckButton::with_label(
                        "ONNX – Swin2SR (CPU/GPU, no binary needed)",
                    );
                    onnx_btn.set_group(Some(&cli_btn));
                    let comfyui_btn = gtk4::CheckButton::with_label(
                        "ComfyUI – Remote/Local server (JSON API)",
                    );
                    comfyui_btn.set_group(Some(&cli_btn));
                    comfyui_btn.set_sensitive(comfyui_enabled);
                    comfyui_btn.set_visible(comfyui_enabled);
                    if backend_kind == UpscaleBackendKind::Onnx {
                        onnx_btn.set_active(true);
                    } else if backend_kind == UpscaleBackendKind::ComfyUi && comfyui_enabled {
                        comfyui_btn.set_active(true);
                    } else {
                        cli_btn.set_active(true);
                    }
                    content.append(&cli_btn);
                    content.append(&onnx_btn);
                    content.append(&comfyui_btn);

                    // ── Per-backend model sections (stack) ───────────────────
                    let backend_stack = gtk4::Stack::new();
                    backend_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
                    backend_stack.set_transition_duration(120);

                    // CLI page
                    let cli_page = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                    let cli_model_label = gtk4::Label::new(Some("CLI Model"));
                    cli_model_label.set_halign(gtk4::Align::Start);
                    let standard_btn =
                        gtk4::CheckButton::with_label("Standard – best for photos");
                    let anime_btn =
                        gtk4::CheckButton::with_label("Anime / Art – best for illustration");
                    anime_btn.set_group(Some(&standard_btn));
                    if saved_cli_model == "anime" {
                        anime_btn.set_active(true);
                    } else {
                        standard_btn.set_active(true);
                    }
                    cli_page.append(&cli_model_label);
                    cli_page.append(&standard_btn);
                    cli_page.append(&anime_btn);
                    backend_stack.add_named(&cli_page, Some("cli"));

                    // ONNX page
                    let onnx_page = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                    let onnx_model_label = gtk4::Label::new(Some("ONNX Model"));
                    onnx_model_label.set_halign(gtk4::Align::Start);

                    let onnx_variants = [
                        OnnxUpscaleModel::Swin2srLightweightX2,
                        OnnxUpscaleModel::Swin2srCompressedX4,
                        OnnxUpscaleModel::Swin2srRealX4,
                    ];
                    let onnx_labels: Vec<&str> =
                        onnx_variants.iter().map(|m| m.info().display_name).collect();
                    let onnx_dropdown = gtk4::DropDown::from_strings(&onnx_labels);
                    let saved_onnx = OnnxUpscaleModel::from_settings(&saved_onnx_model);
                    let onnx_selected_idx = onnx_variants
                        .iter()
                        .position(|&m| m == saved_onnx)
                        .unwrap_or(0) as u32;
                    onnx_dropdown.set_selected(onnx_selected_idx);

                    // Download row — visible when the selected model file is absent.
                    let download_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                    download_row.set_halign(gtk4::Align::Start);
                    let download_btn = gtk4::Button::new();
                    let download_status = gtk4::Label::new(None);
                    download_status.add_css_class("dim-label");

                    let refresh_download_row = {
                        let download_btn = download_btn.clone();
                        let download_status = download_status.clone();
                        move |model: OnnxUpscaleModel| {
                            use crate::upscale::backends::onnx::OnnxBackend;
                            let present = OnnxBackend::model_path(model).exists();
                            if present {
                                download_btn.set_visible(false);
                                download_status.set_text("Downloaded ✓");
                            } else {
                                let info = model.info();
                                download_btn.set_label(&format!(
                                    "Download ({} MB)",
                                    info.download_size_mb
                                ));
                                download_btn.set_visible(true);
                                download_status.set_text("");
                            }
                        }
                    };
                    refresh_download_row(onnx_variants[onnx_selected_idx as usize]);

                    {
                        let refresh = refresh_download_row.clone();
                        onnx_dropdown.connect_selected_notify(move |dd| {
                            let idx = dd.selected() as usize;
                            if let Some(&m) = onnx_variants.get(idx) {
                                refresh(m);
                            }
                        });
                    }

                    {
                        let download_btn_c = download_btn.clone();
                        let download_status_c = download_status.clone();
                        let onnx_dropdown_c = onnx_dropdown.clone();
                        let ops_queue_c = ops_queue.clone();
                        download_btn.connect_clicked(move |btn| {
                            let idx = onnx_dropdown_c.selected() as usize;
                            let Some(&model) = onnx_variants.get(idx) else { return };
                            btn.set_sensitive(false);
                            download_status_c.set_text("Downloading…");
                            let rx = downloader::download_model(model);
                            let op = ops_queue_c.add(format!(
                                "Downloading {}",
                                model.info().display_name
                            ));
                            let btn_weak = download_btn_c.downgrade();
                            let status_weak = download_status_c.downgrade();
                            glib::MainContext::default().spawn_local(async move {
                                let mut op = Some(op);
                                while let Ok(event) = rx.recv().await {
                                    match event {
                                        DownloadEvent::Progress(f) => {
                                            if let Some(op) = op.as_ref() {
                                                op.progress(Some(f));
                                            }
                                        }
                                        DownloadEvent::Done => {
                                            if let Some(op) = op.take() { op.complete(); }
                                            if let Some(b) = btn_weak.upgrade() { b.set_visible(false); }
                                            if let Some(s) = status_weak.upgrade() { s.set_text("Downloaded ✓"); }
                                        }
                                        DownloadEvent::Failed(msg) => {
                                            if let Some(op) = op.take() { op.fail(msg.clone()); }
                                            if let Some(b) = btn_weak.upgrade() { b.set_sensitive(true); }
                                            if let Some(s) = status_weak.upgrade() {
                                                s.set_text(&format!("Failed: {msg}"));
                                            }
                                        }
                                    }
                                }
                            });
                        });
                    }

                    download_row.append(&download_btn);
                    download_row.append(&download_status);
                    onnx_page.append(&onnx_model_label);
                    onnx_page.append(&onnx_dropdown);
                    onnx_page.append(&download_row);
                    backend_stack.add_named(&onnx_page, Some("onnx"));

                    {
                        let stack_c = backend_stack.clone();
                        onnx_btn.connect_toggled(move |btn| {
                            stack_c.set_visible_child_name(if btn.is_active() { "onnx" } else { "cli" });
                        });
                    }
                    backend_stack.set_visible_child_name(match backend_kind {
                        UpscaleBackendKind::Onnx => "onnx",
                        _ => "cli",
                    });
                    content.append(&backend_stack);

                    // ── Scale ────────────────────────────────────────────────
                    let scale_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                    let scale_label = gtk4::Label::new(Some("Scale"));
                    scale_label.set_halign(gtk4::Align::Start);
                    scale_box.append(&scale_label);
                    let scale_dropdown =
                        gtk4::DropDown::from_strings(&["Smart (auto)", "2×", "3×", "4×"]);
                    scale_dropdown.set_selected(0);
                    scale_box.append(&scale_dropdown);
                    content.append(&scale_box);

                    dialog.set_extra_child(Some(&content));

                    let viewer_weak_c = viewer_weak.clone();
                    let action_weak_c = action_weak.clone();
                    let state_cc = state_c.clone();
                    dialog.choose(
                        &viewer,
                        None::<&gio::Cancellable>,
                        move |response| {
                            if response != "upscale" {
                                return;
                            }
                            let Some(viewer) = viewer_weak_c.upgrade() else {
                                return;
                            };
                            if let Some(action) = action_weak_c.upgrade() {
                                action.set_enabled(false);
                            }

                            // Persist backend + model choices.
                            let chosen_backend = if onnx_btn.is_active() {
                                UpscaleBackendKind::Onnx
                            } else if comfyui_btn.is_active() {
                                UpscaleBackendKind::ComfyUi
                            } else {
                                UpscaleBackendKind::Cli
                            };
                            let chosen_onnx_model = onnx_variants
                                .get(onnx_dropdown.selected() as usize)
                                .copied()
                                .unwrap_or_default();
                            {
                                let mut st = state_cc.borrow_mut();
                                st.settings.set_upscale_backend(chosen_backend.settings_key());
                                st.settings
                                    .set_onnx_upscale_model(chosen_onnx_model.settings_key());
                                st.settings.set_upscaler_default_model(
                                    if anime_btn.is_active() { "anime" } else { "standard" },
                                );
                            }

                            let proxy_btn = gtk4::Button::new();
                            let action_weak_c = action_weak_c.clone();
                            proxy_btn.connect_sensitive_notify(move |btn| {
                                if btn.is_sensitive() {
                                    if let Some(a) = action_weak_c.upgrade() {
                                        a.set_enabled(true);
                                    }
                                }
                            });
                            proxy_btn.set_sensitive(false);

                            let cli_model = if anime_btn.is_active() {
                                UpscaleModel::Anime
                            } else {
                                UpscaleModel::Standard
                            };
                            let scale = match scale_dropdown.selected() {
                                1 => 2,
                                2 => 3,
                                3 => 4,
                                _ => 0,
                            };
                            viewer.start_upscale(p.clone(), scale, cli_model, proxy_btn);
                        },
                    );
                }
            });
        }
        self.add_action(&upscale_action);
    }

    /// Build the viewer header bar.
    /// Returns `(header, sidebar_toggle, preview_title_btn, commit_btn, discard_btn, edit_commit_btn, edit_discard_btn)`.
    /// Commit and Discard are initially hidden; the comparison view shows them.
    fn build_viewer_header(
        &self,
        menu_btn: &gtk4::MenuButton,
    ) -> (
        libadwaita::HeaderBar,
        gtk4::ToggleButton,
        gtk4::Button,
        gtk4::Button,
        gtk4::Button,
        gtk4::Button,
        gtk4::Button,
    ) {
        let header = libadwaita::HeaderBar::new();

        let sidebar_toggle = gtk4::ToggleButton::new();
        sidebar_toggle.set_icon_name("sidebar-show-symbolic");
        sidebar_toggle.set_tooltip_text(Some("Toggle Library"));
        sidebar_toggle.add_css_class("flat");
        header.pack_start(&sidebar_toggle);

        let preview_title_btn = gtk4::Button::with_label("Preview");
        preview_title_btn.set_visible(false);

        let commit_btn = gtk4::Button::with_label("Save");
        commit_btn.set_tooltip_text(Some("Save upscaled image"));
        commit_btn.add_css_class("suggested-action");
        commit_btn.set_visible(false);
        header.pack_end(&commit_btn);

        let discard_btn = gtk4::Button::with_label("Discard");
        discard_btn.set_tooltip_text(Some("Discard upscaled image"));
        discard_btn.add_css_class("destructive-action");
        discard_btn.set_visible(false);
        header.pack_end(&discard_btn);

        let edit_commit_btn = gtk4::Button::with_label("Save Edit");
        edit_commit_btn.set_tooltip_text(Some("Save the rotated/flipped image to disk"));
        edit_commit_btn.add_css_class("suggested-action");
        edit_commit_btn.set_visible(false);
        header.pack_end(&edit_commit_btn);

        let edit_discard_btn = gtk4::Button::with_label("Discard Edit");
        edit_discard_btn.set_tooltip_text(Some("Revert to the original image"));
        edit_discard_btn.add_css_class("destructive-action");
        edit_discard_btn.set_visible(false);
        header.pack_end(&edit_discard_btn);

        header.pack_end(menu_btn);

        (
            header,
            sidebar_toggle,
            preview_title_btn,
            commit_btn,
            discard_btn,
            edit_commit_btn,
            edit_discard_btn,
        )
    }
}
