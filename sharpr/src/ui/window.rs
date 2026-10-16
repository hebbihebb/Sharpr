use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;
use std::path::{Path, PathBuf};

use crate::config::{AppSettings, FolderMode};
use crate::library_index::{BasicImageInfo, Collection, IndexedImage, LibraryIndex};
use crate::model::library::{CachedImageData, RawImageEntry};
use crate::model::{ImageEntry, LibraryManager};
use crate::thumbnails::worker::ThumbnailWorkerResponse;
use crate::thumbnails::ThumbnailWorker;
use crate::ui::collection_dialogs::{
    show_change_color_dialog, show_new_collection_dialog, show_rename_collection_dialog,
};
use crate::ui::compare_item::ordered_selection;
use crate::ui::filmstrip::FilmstripPane;
use crate::ui::ops_indicator::OpsIndicator;
use crate::ui::preferences::build_preferences_dialog;
use crate::ui::sidebar::SidebarPane;
use crate::ui::state::{AppState, ViewScope};
use crate::ui::viewer::{ViewerPane, ZoomMode};

mod bucket_repeat;
mod compare_controller;
mod edit_buckets;
mod library_root;
pub(crate) mod mode_header;
mod survey_controller;
/// Gets Compare's survivors when someone else opened it (Find Copies).
type CompareDone = Box<dyn FnOnce(Vec<PathBuf>)>;
pub(crate) use compare_controller::file_actions_blocked;
use compare_controller::is_comparing;
#[cfg(test)]
mod gtk_tests;
mod search_controller;
mod tag_controller;

// ---------------------------------------------------------------------------
// Shared application state (main thread only, Rc<RefCell<>>)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub(crate) struct SidePanels {
    outer_shown: bool,
    inner_shown: bool,
    outer_collapsed: bool,
    inner_collapsed: bool,
    width: i32,
}

pub type OpenFolderCallback = Rc<dyn Fn(PathBuf)>;
pub type RefreshCollectionsCallback = Rc<dyn Fn()>;

pub(super) fn apply_scope_to_sidebar(scope: &ViewScope, sidebar: &SidebarPane) {
    match scope {
        ViewScope::Folder(path) => {
            sidebar.select_folder(path);
        }
        ViewScope::Collection(id) => {
            sidebar.set_collection_selected(*id);
        }
        ViewScope::Duplicates | ViewScope::Search | ViewScope::Compare | ViewScope::Survey => {
            sidebar.clear_collection_selection();
        }
    }
}

fn is_active_folder_load(state: &AppState, path: &Path) -> bool {
    matches!(&state.scope, ViewScope::Folder(scope_path) if scope_path == path)
        && state.library.current_folder.as_deref() == Some(path)
}

fn remove_path_from_action_selection(state: &mut AppState, path: &Path) {
    state.selected_paths.remove(path);
    state
        .selected_paths
        .retain(|selected| state.library.entry_for_path(selected).is_some());
}

/// Saves perceptual hashes on one background thread, a batch per
/// transaction. One pool job per hash used to flood the shared rayon pool
/// and make folder opens wait behind thousands of single-row writes.
fn spawn_phash_writer(index: Arc<LibraryIndex>) -> async_channel::Sender<(PathBuf, u64)> {
    let (tx, rx) = async_channel::unbounded::<(PathBuf, u64)>();
    let _ = std::thread::Builder::new()
        .name("phash-writer".into())
        .spawn(move || {
            while let Ok(first) = rx.recv_blocking() {
                let mut batch = vec![first];
                while batch.len() < 512 {
                    match rx.try_recv() {
                        Ok(next) => batch.push(next),
                        Err(_) => break,
                    }
                }
                if let Err(err) = index.update_image_phashes(&batch) {
                    crate::bench_warn!(
                        "hash.index_persist_fail",
                        serde_json::json!({"count": batch.len(), "error": err.to_string()})
                    );
                }
            }
        });
    tx
}

/// Look prototype: Sort, then the filters, in one menu. A filter here adds
/// the same chip as typing it (option A), so what is active always shows.
fn sort_and_filter_menu(sort: &gio::MenuModel) -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append_section(None, sort);
    let filters = gio::Menu::new();
    for (label, word) in [
        ("Landscape", "landscape"),
        ("Portrait", "portrait"),
        ("Square", "square"),
        ("16:9", "16:9"),
        ("Not 16:9", "-16:9"),
        ("Narrower than 16:9", "<16:9"),
        ("Wider than 16:9", ">16:9"),
        ("4K and Larger", "4k"),
        ("Below 4K", "<4k"),
    ] {
        let item = gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some("win.add-filter-chip"), Some(&word.to_variant()));
        if word == "<4k" {
            item.set_icon(&gio::ThemedIcon::new("sharpr-below-4k-symbolic"));
        }
        filters.append_item(&item);
    }
    menu.append_section(Some("Filter"), &filters);
    menu
}

/// True while a text field has keyboard focus. Single-key shortcuts then leave
/// the key to the text field, so Delete never trashes images while editing text.
fn text_input_has_focus(widget: &gtk4::Widget) -> bool {
    widget
        .downcast_ref::<gtk4::Window>()
        .and_then(gtk4::prelude::GtkWindowExt::focus)
        .is_some_and(|focus| focus.is::<gtk4::Text>() || focus.is::<gtk4::SearchEntry>())
}

/// Moves `paths` to the system trash and drops them from the view, then
/// offers Undo (D25). Tags are kept, so restoring a file from the trash
/// restores its buckets. Files that cannot be trashed are reported instead of
/// being skipped silently.
fn trash_paths(
    window: Option<&SharprWindow>,
    state: &Rc<RefCell<AppState>>,
    filmstrip: &FilmstripPane,
    viewer: &ViewerPane,
    paths: &[PathBuf],
) {
    let mut failures = Vec::new();
    let mut trashed = Vec::new();
    for path in paths {
        match gio::File::for_path(path).trash(None::<&gio::Cancellable>) {
            Ok(()) => {
                trashed.push(path.clone());
                let mut state = state.borrow_mut();
                state.library.remove_path(path);
                remove_path_from_action_selection(&mut state, path);
            }
            Err(error) => failures.push((path.clone(), error.to_string())),
        }
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
    if let (Some(window), false) = (window, trashed.is_empty()) {
        let toast = libadwaita::Toast::new(&format!(
            "Moved {} to the trash",
            crate::ui::copies_dialog::images_text(trashed.len())
        ));
        toast.set_button_label(Some("Undo"));
        toast.set_timeout(10);
        let weak = window.downgrade();
        toast.connect_button_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.restore_trashed(trashed.clone());
            }
        });
        window.add_toast(toast);
    }
    if let Some((path, error)) = failures.first() {
        crate::bench_warn!(
            "file.trash.fail",
            serde_json::json!({
                "count": failures.len(),
                "path": path.display().to_string(),
                "error": error,
            })
        );
        if let Some(window) = window {
            let message = if failures.len() == 1 {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                format!("Could not move “{name}” to the trash: {error}")
            } else {
                format!(
                    "Could not move {} files to the trash: {error}",
                    failures.len()
                )
            };
            window.add_toast(libadwaita::Toast::new(&message));
        }
    }
}

fn search_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|term| term.trim().to_lowercase())
        .filter(|term| !term.is_empty())
        .collect()
}

fn collections_for_sidebar(state: &AppState) -> Vec<Collection> {
    state.collections.clone()
}

fn flatten_collection_paths(collections: &[Collection]) -> Vec<(i64, String)> {
    let mut by_parent: HashMap<Option<i64>, Vec<Collection>> = HashMap::new();
    for collection in collections {
        by_parent
            .entry(collection.parent_id)
            .or_default()
            .push(collection.clone());
    }
    for children in by_parent.values_mut() {
        children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }

    fn walk(
        parent_id: Option<i64>,
        prefix: Option<String>,
        by_parent: &HashMap<Option<i64>, Vec<Collection>>,
        out: &mut Vec<(i64, String)>,
    ) {
        if let Some(children) = by_parent.get(&parent_id) {
            for child in children {
                let label = match &prefix {
                    Some(prefix) => format!("{prefix} > {}", child.name),
                    None => child.name.clone(),
                };
                out.push((child.id, label.clone()));
                walk(Some(child.id), Some(label), by_parent, out);
            }
        }
    }

    let mut out = Vec::new();
    walk(None, None, &by_parent, &mut out);
    out
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

fn path_is_disabled(path: &std::path::Path, disabled_folders: &[PathBuf]) -> bool {
    disabled_folders
        .iter()
        .any(|folder| path.starts_with(folder))
}

fn path_in_active_library(path: &Path, active_root: Option<&Path>) -> bool {
    active_root
        .map(|root| path.starts_with(root))
        .unwrap_or(true)
}

pub(super) fn filter_paths_for_library(
    paths: Vec<PathBuf>,
    active_root: Option<&Path>,
    disabled_folders: &[PathBuf],
) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|path| path_in_active_library(path, active_root))
        .filter(|path| !path_is_disabled(path, disabled_folders))
        .collect()
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
            match crate::export::oriented_source_dimensions(&info.path) {
                Ok((width, height)) => {
                    let quality = crate::quality::scorer::score_file_info(Some((width, height)));
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

fn start_raw_dimension_hydrator(
    paths: Vec<PathBuf>,
    folder: PathBuf,
    state: Rc<RefCell<AppState>>,
) {
    if paths.is_empty() {
        return;
    }
    crate::bench_event!(
        "folder.raw_scan.hydrate.start",
        serde_json::json!({
            "folder": folder.display().to_string(),
            "count": paths.len(),
        }),
    );
    let (tx, rx) = async_channel::unbounded::<MetadataIndexResult>();
    let folder_worker = folder.clone();
    rayon::spawn(move || {
        let started = Instant::now();
        let total = paths.len();
        let mut completed = 0usize;
        for path in paths {
            if let Ok((width, height)) = crate::export::oriented_source_dimensions(&path) {
                completed += 1;
                let _ = tx.send_blocking(MetadataIndexResult {
                    path,
                    width,
                    height,
                });
            }
        }
        crate::bench_event!(
            "folder.raw_scan.hydrate.finish",
            serde_json::json!({
                "folder": folder_worker.display().to_string(),
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

/// The line under the search box: coverage and strength, the selection
/// count, "Add to Bucket…", and "Show More".
#[derive(Clone)]
pub struct SearchStatus {
    pub bar: gtk4::Box,
    pub label: gtk4::Label,
    pub selected: gtk4::Label,
    pub add: gtk4::Button,
    pub more: gtk4::Button,
}

pub(super) fn load_virtual_async(state: &Rc<RefCell<AppState>>, paths: &[PathBuf]) {
    let paths: Vec<PathBuf> = {
        let st = state.borrow();
        paths
            .iter()
            .filter(|path| !path_is_disabled(path, &st.disabled_folders))
            .cloned()
            .collect()
    };
    let (generation, pending) = {
        let mut st = state.borrow_mut();
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
            if st.library.current_folder.is_none() && st.library.virtual_generation == generation {
                st.library
                    .apply_cached_image_data(&result.path, result.cached);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// GObject subclass
// ---------------------------------------------------------------------------

mod imp {
    use super::*;
    use crate::thumbnails::worker::{HashResult, SharpnessResult, ThumbnailWorkerResponse};
    use async_channel::Receiver;

    pub struct SharprWindow {
        pub state: Rc<RefCell<AppState>>,
        pub viewer: RefCell<Option<ViewerPane>>,
        pub thumbnail_worker: RefCell<Option<ThumbnailWorker>>,
        pub metadata_worker: RefCell<Option<crate::image_pipeline::worker::MetadataWorker>>,
        // Cloned receiver so the async task can hold it.
        pub result_rx: RefCell<Option<Receiver<ThumbnailWorkerResponse>>>,
        pub hash_result_rx: RefCell<Option<Receiver<HashResult>>>,
        pub sharpness_result_rx: RefCell<Option<Receiver<SharpnessResult>>>,
        pub global_thumbnail_op: RefCell<Option<crate::ops::queue::OpHandle>>,
        pub toast_overlay: RefCell<Option<libadwaita::ToastOverlay>>,
        pub sidebar: RefCell<Option<SidebarPane>>,
        pub filmstrip: RefCell<Option<FilmstripPane>>,
        pub inline_search_entry: RefCell<Option<gtk4::SearchEntry>>,
        pub pending_created_copy: RefCell<Option<PathBuf>>,
        pub inline_search_revealer: RefCell<Option<gtk4::Revealer>>,
        pub search_whole_library: RefCell<Option<gtk4::CheckButton>>,
        pub search_chips: RefCell<Option<gtk4::FlowBox>>,
        pub search_suggestions: RefCell<Option<gtk4::FlowBox>>,
        pub search_chip_bar: RefCell<Option<gtk4::Revealer>>,
        pub bucket_open_generation: Cell<u64>,
        /// The bucket open whose membership is still loading. Searching then
        /// waits, because the filmstrip still shows the previous view.
        pub bucket_open_pending: Cell<Option<u64>>,
        /// Increases whenever a search session starts (a browse snapshot is taken).
        pub browse_session: Cell<u64>,
        pub inline_search_generation: Cell<u64>,
        pub browse_generation: Arc<std::sync::atomic::AtomicU64>,
        pub(super) browse_worker: search_controller::BrowseWorker,
        pub inline_search_suppressed: Cell<bool>,
        pub(super) browse_snapshot: RefCell<Option<search_controller::BrowseSnapshot>>,
        pub(super) browse_options: RefCell<search_controller::BrowseOptions>,
        pub presentation_mode: Cell<bool>,
        pub presentation_sidebar_was_visible: Cell<bool>,
        pub presentation_filmstrip_was_visible: Cell<bool>,
        pub presentation_header: RefCell<Option<libadwaita::HeaderBar>>,
        pub(super) mode_header: RefCell<Option<mode_header::ModeHeader>>,
        /// Look prototype: the last pointer position, where pickers open.
        pub last_pointer: Cell<Option<(f64, f64)>>,
        pub presentation_outer_split: RefCell<Option<libadwaita::OverlaySplitView>>,
        pub presentation_inner_split: RefCell<Option<libadwaita::OverlaySplitView>>,
        pub compare_page: RefCell<Option<crate::ui::compare_page::ComparePage>>,
        pub(crate) survey_page: RefCell<Option<Rc<crate::ui::survey::SurveyPage>>>,
        pub content_stack: RefCell<Option<gtk4::Stack>>,
        pub open_folder_cb: RefCell<Option<OpenFolderCallback>>,
        pub refresh_sidebar_collections_cb: RefCell<Option<RefreshCollectionsCallback>>,
        /// Background content fingerprinting (ROADMAP Phase 4).
        pub fingerprints: RefCell<Option<Rc<crate::fingerprint::Service>>>,
        /// Background visual-search indexing (ROADMAP Phase 5).
        pub visual_index: RefCell<Option<Arc<crate::visual_search::index::Service>>>,
        /// Turns search words into a query vector; set with `visual_index`.
        pub visual_searcher: RefCell<Option<crate::visual_search::search::Searcher>>,
        /// How many visual-search results to show; "Show More" raises it.
        pub visual_limit: Cell<usize>,
        /// Coverage and match strength of the current search, with "Show More".
        pub search_status: RefCell<Option<SearchStatus>>,
        /// The shown search results that match strongly, for "Add to Bucket…".
        pub search_strong: RefCell<Vec<PathBuf>>,
        /// The crop in progress in the viewer, if any (Phase 6).
        pub crop_session: RefCell<Option<crate::ui::crop_mode::CropSession>>,
        /// Counts crop requests; a preview that arrives after another
        /// request, or after the image changed, is dropped.
        pub crop_request: Cell<u64>,
        /// A crop's preview is being prepared.
        pub crop_preparing: Cell<bool>,
        /// Which side panels Compare hid, to show them again.
        pub(super) review_panels: Cell<Option<SidePanels>>,
        /// Who opened Compare and gets the survivors back instead of the
        /// browsing selection (Find Copies).
        pub compare_done: RefCell<Option<CompareDone>>,
    }

    impl Default for SharprWindow {
        fn default() -> Self {
            let (ops_queue, _ops_rx) = crate::ops::queue::new_queue();
            let browse_generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let browse_worker = search_controller::BrowseWorker::new(browse_generation.clone());
            Self {
                state: Rc::new(RefCell::new(AppState::new(ops_queue))),
                viewer: RefCell::new(None),
                thumbnail_worker: RefCell::new(None),
                metadata_worker: RefCell::new(None),
                result_rx: RefCell::new(None),
                hash_result_rx: RefCell::new(None),
                sharpness_result_rx: RefCell::new(None),
                global_thumbnail_op: RefCell::new(None),
                toast_overlay: RefCell::new(None),
                sidebar: RefCell::new(None),
                filmstrip: RefCell::new(None),
                inline_search_entry: RefCell::new(None),
                pending_created_copy: RefCell::new(None),
                inline_search_revealer: RefCell::new(None),
                search_whole_library: RefCell::new(None),
                search_chips: RefCell::new(None),
                search_suggestions: RefCell::new(None),
                search_chip_bar: RefCell::new(None),
                bucket_open_generation: Cell::new(0),
                bucket_open_pending: Cell::new(None),
                browse_session: Cell::new(0),
                inline_search_generation: Cell::new(0),
                browse_generation,
                browse_worker,
                inline_search_suppressed: Cell::new(false),
                browse_snapshot: RefCell::new(None),
                browse_options: RefCell::new(Default::default()),
                presentation_mode: Cell::new(false),
                presentation_sidebar_was_visible: Cell::new(false),
                presentation_filmstrip_was_visible: Cell::new(false),
                presentation_header: RefCell::new(None),
                mode_header: RefCell::new(None),
                last_pointer: Cell::new(None),
                presentation_outer_split: RefCell::new(None),
                presentation_inner_split: RefCell::new(None),
                compare_page: RefCell::new(None),
                survey_page: RefCell::new(None),
                content_stack: RefCell::new(None),
                open_folder_cb: RefCell::new(None),
                refresh_sidebar_collections_cb: RefCell::new(None),
                fingerprints: RefCell::new(None),
                visual_index: RefCell::new(None),
                visual_searcher: RefCell::new(None),
                visual_limit: Cell::new(crate::visual_search::search::PAGE),
                search_status: RefCell::new(None),
                search_strong: RefCell::new(Vec::new()),
                crop_session: RefCell::new(None),
                crop_request: Cell::new(0),
                crop_preparing: Cell::new(false),
                review_panels: Cell::new(None),
                compare_done: RefCell::new(None),
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
        @implements gio::ActionGroup, gio::ActionMap, gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Native, gtk4::Root, gtk4::ShortcutManager;
}

impl SharprWindow {
    pub fn new(app: &libadwaita::Application) -> Self {
        app.set_accels_for_action("win.show-help-overlay", &["<Control>question"]);
        glib::Object::builder().property("application", app).build()
    }

    pub fn app_state(&self) -> std::rc::Rc<std::cell::RefCell<AppState>> {
        self.imp().state.clone()
    }

    pub fn show_created_copy(&self, output: PathBuf) {
        let Some(folder) = output.parent().map(Path::to_path_buf) else {
            return;
        };
        *self.imp().pending_created_copy.borrow_mut() = Some(output);
        self.imp().state.borrow_mut().library.current_folder = None;
        if let Some(open) = self.imp().open_folder_cb.borrow().as_ref() {
            open(folder);
        }
    }

    pub(crate) fn imp_viewer(&self) -> Option<crate::ui::viewer::ViewerPane> {
        self.imp().viewer.borrow().clone()
    }

    /// Modes need an empty canvas even when the wide layout normally keeps
    /// its sidebars visible. Preserve their state for returning to browsing.
    pub(crate) fn hide_side_panels(&self) -> SidePanels {
        let outer = self
            .imp()
            .presentation_outer_split
            .borrow()
            .clone()
            .unwrap();
        let inner = self
            .imp()
            .presentation_inner_split
            .borrow()
            .clone()
            .unwrap();
        let saved = SidePanels {
            outer_shown: outer.shows_sidebar(),
            inner_shown: inner.shows_sidebar(),
            outer_collapsed: outer.is_collapsed(),
            inner_collapsed: inner.is_collapsed(),
            width: self.width(),
        };
        inner.set_collapsed(true);
        inner.set_show_sidebar(false);
        outer.set_collapsed(true);
        outer.set_show_sidebar(false);
        saved
    }

    pub(crate) fn restore_side_panels(&self, saved: SidePanels) {
        // A resize during the mode must restore the current adaptive layout.
        let resized = saved.width != self.width();
        if let Some(inner) = self.imp().presentation_inner_split.borrow().as_ref() {
            inner.set_collapsed(if resized {
                self.width() < 800
            } else {
                saved.inner_collapsed
            });
            inner.set_show_sidebar(saved.inner_shown);
        }
        if let Some(outer) = self.imp().presentation_outer_split.borrow().as_ref() {
            outer.set_collapsed(if resized {
                self.width() <= 1500
            } else {
                saved.outer_collapsed
            });
            outer.set_show_sidebar(saved.outer_shown);
        }
    }

    pub(crate) fn set_crop_session(&self, session: Option<crate::ui::crop_mode::CropSession>) {
        *self.imp().crop_session.borrow_mut() = session;
    }

    /// The crop in progress, if any.
    pub fn crop_session(&self) -> Option<crate::ui::crop_mode::CropSession> {
        self.imp().crop_session.borrow().clone()
    }

    /// Look prototype: a compact picker beside the pointer, Eagle-style (I7),
    /// or in the middle of the window when the pointer hasn't moved. It
    /// closes on a click away and is discarded when closed.
    pub(crate) fn popover_at_pointer(&self, child: &impl IsA<gtk4::Widget>) -> gtk4::Popover {
        let host = self.content().expect("window content");
        let (x, y) = self
            .imp()
            .last_pointer
            .get()
            .unwrap_or((self.width() as f64 / 2.0, self.height() as f64 / 2.0));
        let at = self
            .compute_point(&host, &gtk4::graphene::Point::new(x as f32, y as f32))
            .unwrap_or_else(|| gtk4::graphene::Point::new(x as f32, y as f32));
        let popover = gtk4::Popover::new();
        popover.add_css_class("sharpr-picker");
        popover.set_child(Some(child));
        popover.set_parent(&host);
        popover.set_pointing_to(Some(&gdk4::Rectangle::new(
            at.x() as i32,
            at.y() as i32,
            1,
            1,
        )));
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
        popover
    }

    /// Look prototype: the monospace face for the whole interface, not only
    /// numbers. On by default; `SHARPR_LOOK_MONO=numbers` turns it off.
    pub fn set_mono_everywhere(&self, on: bool) {
        if on {
            self.add_css_class("sharpr-mono-all");
        } else {
            self.remove_css_class("sharpr-mono-all");
        }
    }

    /// Undo for the filmstrip's trash (D25): puts `paths` back on a worker,
    /// never over a file now at the same name, and reopens their folder if
    /// it is the one in view. Those that can't return stay in the trash.
    pub(crate) fn restore_trashed(&self, paths: Vec<PathBuf>) {
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let mut restored = Vec::new();
            let mut failed = Vec::new();
            for path in paths {
                match crate::file_ops::restore_from_trash(&path) {
                    Ok(()) => restored.push(path),
                    Err(error) => failed.push((path, error)),
                }
            }
            let _ = tx.send_blocking((restored, failed));
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok((restored, failed)) = rx.recv().await else {
                return;
            };
            let Some(win) = weak.upgrade() else { return };
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
                None => format!(
                    "Restored {}",
                    crate::ui::copies_dialog::images_text(restored.len())
                ),
                Some((path, error)) => {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let mut message = format!("Couldn’t restore “{name}”: {error}");
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

    /// Moves copies chosen in "Find Copies…" to the trash on a worker, drops
    /// them from the view, and reports what happened (Phase 7).
    /// Trashes the confirmed copies, after checking each against what was
    /// reviewed (`copies::verify`).
    pub(crate) fn trash_copies(&self, groups: Vec<crate::copies::TrashGroup>) {
        let operation = self
            .imp()
            .state
            .borrow()
            .ops
            .add("Moving copies to the trash…");
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let mut trashed = Vec::new();
            let mut failed = Vec::new();
            for group in &groups {
                for verdict in crate::copies::verify(group) {
                    match verdict {
                        crate::copies::Verdict::Trash(path) => {
                            match gio::File::for_path(&path).trash(None::<&gio::Cancellable>) {
                                Ok(()) => trashed.push(path),
                                Err(error) => failed.push((path, error.to_string())),
                            }
                        }
                        crate::copies::Verdict::Skip(path, reason) => {
                            failed.push((path, reason.to_string()))
                        }
                    }
                }
            }
            let _ = tx.send_blocking((trashed, failed));
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok((trashed, failed)) = rx.recv().await else {
                operation.fail("Trash worker stopped");
                return;
            };
            operation.complete();
            let Some(win) = weak.upgrade() else { return };
            {
                let mut st = win.imp().state.borrow_mut();
                for path in &trashed {
                    st.library.remove_path(path);
                    remove_path_from_action_selection(&mut st, path);
                }
            }
            if let Some(filmstrip) = win.imp().filmstrip.borrow().as_ref() {
                filmstrip.refresh_virtual();
                filmstrip.refresh_multi_selection_visuals();
            }
            crate::bench_event!(
                "copies.trash",
                serde_json::json!({"trashed": trashed.len(), "failed": failed.len()})
            );
            let mut message = format!(
                "Moved {} {} to the trash",
                trashed.len(),
                if trashed.len() == 1 { "file" } else { "files" }
            );
            if let Some((path, error)) = failed.first() {
                message.push_str(&format!(
                    "; {} couldn't be moved ({}: {error})",
                    failed.len(),
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            win.add_toast(libadwaita::Toast::new(&message));
        });
    }

    /// Exports `sources` as WebP in the background (ROADMAP Phase 6). A toast
    /// counts progress and can cancel; cancelling keeps finished images and
    /// leaves no partial file. The summary opens the folder, or lists
    /// failures.
    pub fn start_webp_export(
        &self,
        sources: Vec<PathBuf>,
        options: crate::export::batch::WebpOptions,
    ) {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let total = sources.len();
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicUsize::new(0));
        let operation = self.imp().state.borrow().ops.add("Exporting WebP…");
        let toast = libadwaita::Toast::new(&format!("Exporting {total} images as WebP…"));
        toast.set_timeout(0);
        toast.set_button_label(Some("Cancel"));
        {
            let cancel = cancel.clone();
            toast.connect_button_clicked(move |_| cancel.store(true, Ordering::Relaxed));
        }
        self.add_toast(toast.clone());
        let finished = Rc::new(std::cell::Cell::new(false));
        let (tx, rx) = async_channel::bounded(1);
        let destination = options.destination.clone();
        {
            let (cancel, done) = (cancel.clone(), done.clone());
            std::thread::spawn(move || {
                // Export yields to browsing: lowest CPU and I/O priority.
                #[cfg(not(test))]
                crate::fingerprint::lower_thread_priority();
                let report = crate::export::batch::export_webp(
                    &sources,
                    &options,
                    &|| cancel.load(Ordering::Relaxed),
                    &|finished, _| done.store(finished, Ordering::Relaxed),
                );
                let _ = tx.send_blocking(report);
            });
        }
        {
            let toast = toast.clone();
            let done = done.clone();
            let finished = finished.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                if finished.get() {
                    return glib::ControlFlow::Break;
                }
                let finished = done.load(Ordering::Relaxed);
                toast.set_title(&format!(
                    "Exporting {} of {total} images as WebP…",
                    finished + 1
                ));
                glib::ControlFlow::Continue
            });
        }
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(report) = rx.recv().await else {
                operation.fail("Export worker stopped");
                return;
            };
            finished.set(true);
            toast.dismiss();
            operation.complete();
            let Some(win) = weak.upgrade() else { return };
            win.add_edit_copies(
                crate::buckets::EditKind::Exported,
                report
                    .written_from
                    .iter()
                    .cloned()
                    .zip(report.written.iter().cloned())
                    .collect(),
            );
            let written = report.written.len();
            let place = destination
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| destination.display().to_string());
            let mut message = if report.canceled {
                format!("Export stopped: {written} of {total} images exported to {place}")
            } else {
                format!(
                    "Exported {written} {} to {place}",
                    if written == 1 { "image" } else { "images" }
                )
            };
            if !report.failed.is_empty() {
                message.push_str(&format!("; {} couldn't be exported", report.failed.len()));
            }
            let summary = libadwaita::Toast::new(&message);
            summary.set_timeout(8);
            if report.failed.is_empty() {
                if written > 0 {
                    summary.set_button_label(Some("Show"));
                    let weak = win.downgrade();
                    let first = report.written[0].clone();
                    summary.connect_button_clicked(move |_| {
                        if let Some(win) = weak.upgrade() {
                            win.show_created_copy(first.clone());
                        }
                    });
                }
            } else {
                summary.set_button_label(Some("Details"));
                let weak = win.downgrade();
                let failed = report.failed.clone();
                summary.connect_button_clicked(move |_| {
                    let Some(win) = weak.upgrade() else { return };
                    let body = failed
                        .iter()
                        .take(20)
                        .map(|(path, error)| {
                            format!(
                                "{}: {error}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let more = failed.len().saturating_sub(20);
                    let body = if more > 0 {
                        format!("{body}\n…and {more} more")
                    } else {
                        body
                    };
                    let alert = libadwaita::AlertDialog::new(
                        Some("Images That Couldn't Be Exported"),
                        Some(&body),
                    );
                    alert.add_response("close", "Close");
                    alert.present(Some(&win));
                });
            }
            win.add_toast(summary);
        });
    }

    /// Opens Crop Copy for the image the user right-clicked, or WebP
    /// export for the images the action covers (decision D21).
    pub fn present_copy_dialog(&self, kind: crate::ui::filmstrip::CopyKind, sources: Vec<PathBuf>) {
        use crate::ui::filmstrip::CopyKind;
        match kind {
            CopyKind::Crop => {
                if let Some(source) = sources.into_iter().next() {
                    crate::ui::crop_mode::start(self, source);
                }
            }
            CopyKind::Export => crate::ui::export_dialog::present(self, sources),
        }
    }

    /// Progress of the latest fingerprint pass, if one has started.
    pub fn fingerprint_progress(&self) -> Option<crate::fingerprint::Progress> {
        self.imp()
            .fingerprints
            .borrow()
            .as_ref()
            .and_then(|service| service.progress())
    }

    /// Where visual-search indexing stands, if the service is running.
    pub fn visual_search_status(&self) -> Option<crate::visual_search::index::Status> {
        self.imp()
            .visual_index
            .borrow()
            .as_ref()
            .map(|service| service.status())
    }

    /// Speed, time left, and the last finished pass of visual indexing.
    pub fn visual_search_pace(&self) -> crate::visual_search::index::Pace {
        self.imp()
            .visual_index
            .borrow()
            .as_ref()
            .map(|service| service.pace())
            .unwrap_or_default()
    }

    /// Deletes the visual-search vectors and indexes the library again.
    pub fn rebuild_visual_index(&self) {
        let job = {
            let st = self.imp().state.borrow();
            st.settings
                .active_library()
                .map(|library| (library.root.clone(), st.disabled_folders.clone()))
        };
        if let (Some((root, disabled)), Some(service)) =
            (job, self.imp().visual_index.borrow().as_ref())
        {
            service.rebuild(crate::visual_search::index::Job { root, disabled });
            self.add_toast(libadwaita::Toast::new("Rebuilding the visual search index"));
        }
    }

    /// The graphics card indexing runs on right now, if any.
    pub fn visual_search_device(&self) -> Option<String> {
        self.imp()
            .visual_index
            .borrow()
            .as_ref()
            .and_then(|service| service.device())
    }

    /// Lets indexing use the graphics card, from the next pass on.
    pub fn set_visual_search_gpu(&self, enabled: bool) {
        self.imp()
            .state
            .borrow_mut()
            .settings
            .set_visual_search_gpu(enabled);
        if let Some(service) = self.imp().visual_index.borrow().as_ref() {
            service.set_use_gpu(enabled);
        }
    }

    /// The enabled part of the library changed: running background passes
    /// over the old scope stop, and new ones cover the new scope.
    pub fn library_scope_changed(&self) {
        let job = {
            let st = self.imp().state.borrow();
            st.settings
                .active_library()
                .map(|library| (library.root.clone(), st.disabled_folders.clone()))
        };
        if let (Some((root, disabled)), Some(service)) =
            (job, self.imp().visual_index.borrow().as_ref())
        {
            service.request(crate::visual_search::index::Job { root, disabled });
        }
        self.request_fingerprints();
    }

    /// Asks for a fingerprint pass over the active library. Passes over the
    /// same library are coalesced and spaced out by the service.
    pub fn request_fingerprints(&self) {
        let Some(service) = self.imp().fingerprints.borrow().clone() else {
            return;
        };
        let job = {
            let st = self.imp().state.borrow();
            let Some(library) = st.settings.active_library() else {
                return;
            };
            crate::fingerprint::Job {
                root: library.root.clone(),
                disabled: st.disabled_folders.clone(),
            }
        };
        service.request(job);
    }

    /// Called after tags change outside a bucket edit, so filmstrip badge
    /// colours follow.
    pub fn tags_changed(&self) {
        if let Some(filmstrip) = self.imp().filmstrip.borrow().as_ref() {
            filmstrip.rebuild_path_colors();
        }
    }

    pub fn add_toast(&self, toast: libadwaita::Toast) {
        if let Some(overlay) = self.imp().toast_overlay.borrow().as_ref() {
            overlay.add_toast(toast);
        }
    }

    pub fn handle_collection_export_requested(&self) {
        let state = self.app_state();
        let index = state.borrow().library_index.clone();
        let library_id = state.borrow().settings.active_library_id.clone();

        let (Some(index), Some(library_id)) = (index, library_id) else {
            self.add_toast(libadwaita::Toast::new("Cannot export: Library not ready"));
            return;
        };

        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Export Buckets");

        let now = glib::DateTime::now_local().unwrap();
        let filename = format!(
            "sharpr-collections-{}.json",
            now.format("%Y-%m-%d").unwrap()
        );
        dialog.set_initial_name(Some(&filename));

        let window_weak = self.downgrade();
        let window_ptr = self.clone().upcast::<gtk4::Window>();
        dialog.save(
            Some(&window_ptr),
            None::<&gio::Cancellable>,
            move |result| {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        match crate::collection_export::export_collections(&index, &library_id) {
                            Ok(export) => match serde_json::to_string_pretty(&export) {
                                Ok(json) => {
                                    if let Err(e) = std::fs::write(&path, json) {
                                        window.add_toast(libadwaita::Toast::new(&format!(
                                            "Export failed: {e}"
                                        )));
                                    } else {
                                        window
                                            .add_toast(libadwaita::Toast::new("Buckets exported"));
                                    }
                                }
                                Err(e) => {
                                    window.add_toast(libadwaita::Toast::new(&format!(
                                        "Serialization failed: {e}"
                                    )));
                                }
                            },
                            Err(e) => {
                                window.add_toast(libadwaita::Toast::new(&format!(
                                    "Export failed: {e}"
                                )));
                            }
                        }
                    }
                }
            },
        );
    }

    pub fn handle_collection_import_requested(&self) {
        let state = self.app_state();
        let index = state.borrow().library_index.clone();
        let library_id = state.borrow().settings.active_library_id.clone();

        let (Some(index), Some(library_id)) = (index, library_id) else {
            self.add_toast(libadwaita::Toast::new("Cannot import: Library not ready"));
            return;
        };

        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Import Buckets");

        let filter = gtk4::FileFilter::new();
        filter.set_name(Some("JSON files"));
        filter.add_suffix("json");
        let filters = gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));

        let window_weak = self.downgrade();
        let window_ptr = self.clone().upcast::<gtk4::Window>();
        dialog.open(
            Some(&window_ptr),
            None::<&gio::Cancellable>,
            move |result| {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        match std::fs::read_to_string(&path) {
                            Ok(json) => {
                                match serde_json::from_str::<
                                    crate::collection_export::CollectionExport,
                                >(&json)
                                {
                                    Ok(export) => {
                                        match crate::collection_export::import_collections(
                                            &index,
                                            &export,
                                            &library_id,
                                        ) {
                                            Ok(summary) => {
                                                if let Some(sidebar) =
                                                    window.imp().sidebar.borrow().as_ref()
                                                {
                                                    if let Ok(collections) = index
                                                        .list_collections_for_library(Some(
                                                            &library_id,
                                                        ))
                                                    {
                                                        sidebar.refresh_collections(&collections);
                                                    }
                                                }
                                                window.add_toast(libadwaita::Toast::new(&format!(
                                                    "Imported {} collections, {} images added",
                                                    summary.collections_added, summary.paths_added
                                                )));
                                            }
                                            Err(e) => {
                                                window.add_toast(libadwaita::Toast::new(&format!(
                                                    "Import failed: {e}"
                                                )));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        window.add_toast(libadwaita::Toast::new(&format!(
                                            "Invalid file format: {e}"
                                        )));
                                    }
                                }
                            }
                            Err(e) => {
                                window.add_toast(libadwaita::Toast::new(&format!(
                                    "Could not read file: {e}"
                                )));
                            }
                        }
                    }
                }
            },
        );
    }

    fn setup(&self) {
        self.set_title(Some("Image Library"));
        let (ww, wh) = {
            let st = self.imp().state.borrow();
            (st.settings.window_width, st.settings.window_height)
        };
        self.set_default_size(ww, wh);
        self.set_size_request(1024, 600);

        // -----------------------------------------------------------------------
        // Thumbnail worker pool
        // -----------------------------------------------------------------------
        let cpu_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        // Double the thread count for I/O bound thumbnail loading, cap at 16.
        let thread_count = (cpu_count * 2).clamp(4, 16);
        let tags = self.imp().state.borrow().tags.clone();
        let (worker, result_rx, hash_result_rx, sharpness_result_rx) =
            ThumbnailWorker::spawn(thread_count, tags.clone());

        *self.imp().thumbnail_worker.borrow_mut() = Some(worker);
        *self.imp().result_rx.borrow_mut() = Some(result_rx);
        *self.imp().hash_result_rx.borrow_mut() = Some(hash_result_rx);
        *self.imp().sharpness_result_rx.borrow_mut() = Some(sharpness_result_rx);

        let (metadata_worker, metadata_result_rx) =
            crate::image_pipeline::worker::MetadataWorker::spawn();
        *self.imp().metadata_worker.borrow_mut() = Some(metadata_worker);

        // Fingerprints start once startup has settled, at the lowest priority.
        if let Some(index) = self.imp().state.borrow().library_index.clone() {
            // After each pass, members follow moved and edited files; views
            // refresh when fingerprints or members changed.
            let (changed_tx, changed_rx) = async_channel::unbounded::<()>();
            let relink_index = index.clone();
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
            let service = crate::fingerprint::Service::start(
                index,
                Box::new(move |job, pass| {
                    visual_index.request(crate::visual_search::index::Job {
                        root: job.root.clone(),
                        disabled: job.disabled.clone(),
                    });
                    match crate::buckets::relink(&relink_index) {
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
                }),
            );
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

        let state = self.imp().state.clone();
        let (ops_queue, ops_rx) = crate::ops::queue::new_queue();
        state.borrow_mut().ops = ops_queue;

        // -----------------------------------------------------------------------
        // Thumbnail workload monitor
        // -----------------------------------------------------------------------
        {
            let window_weak = self.downgrade();
            glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                let Some(window) = window_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };

                let pending = window
                    .imp()
                    .thumbnail_worker
                    .borrow()
                    .as_ref()
                    .map(|w| w.pending_count())
                    .unwrap_or(0);

                let mut op_handle = window.imp().global_thumbnail_op.borrow_mut();

                if pending > 0 {
                    if op_handle.is_none() {
                        let op = window.app_state().borrow().ops.add("Loading thumbnails…");
                        *op_handle = Some(op);
                    }
                } else if let Some(op) = op_handle.take() {
                    op.complete();
                }

                glib::ControlFlow::Continue
            });
        }

        let state_close = state.clone();
        self.connect_close_request(move |win| {
            let (w, h) = (win.width(), win.height());
            let mut st = state_close.borrow_mut();
            st.settings.window_width = w;
            st.settings.window_height = h;
            st.settings.save();
            glib::Propagation::Proceed
        });

        // -----------------------------------------------------------------------
        // Build panes
        // -----------------------------------------------------------------------
        let sidebar = SidebarPane::new(state.clone());
        let filmstrip = FilmstripPane::new(state.clone());
        let viewer = ViewerPane::new(state.clone());

        *self.imp().sidebar.borrow_mut() = Some(sidebar.clone());
        *self.imp().filmstrip.borrow_mut() = Some(filmstrip.clone());
        *self.imp().viewer.borrow_mut() = Some(viewer.clone());
        if let Some(worker) = self.imp().thumbnail_worker.borrow().as_ref() {
            filmstrip.set_thumbnail_sender(
                worker.visible_sender(),
                worker.preload_sender(),
                worker.generation_arc(),
                worker.pending_set(),
            );
        }

        if let Some(index) = state.borrow().library_index.clone() {
            filmstrip.set_library_index(index);
        }

        if let Some(worker) = self.imp().metadata_worker.borrow().as_ref() {
            viewer.set_metadata_worker(worker.handle(), metadata_result_rx);
        }

        let content_stack = gtk4::Stack::new();
        *self.imp().content_stack.borrow_mut() = Some(content_stack.clone());
        content_stack.set_transition_type(gtk4::StackTransitionType::SlideLeft);
        content_stack.set_transition_duration(200);

        let welcome_page = libadwaita::StatusPage::builder()
            .icon_name("folder-open-symbolic")
            .title("No folder open")
            .description("Open a folder from the sidebar to start browsing")
            .build();
        content_stack.add_named(&welcome_page, Some("welcome"));

        let toast_overlay = libadwaita::ToastOverlay::new();
        *self.imp().toast_overlay.borrow_mut() = Some(toast_overlay.clone());
        if let Some(error) = self.imp().state.borrow().tags_error.clone() {
            let toast =
                libadwaita::Toast::new(&format!("Tags and buckets are unavailable: {error}"));
            toast.set_timeout(0);
            toast_overlay.add_toast(toast);
        }
        if let Some(error) = self.imp().state.borrow().bucket_migration_error.clone() {
            let toast = libadwaita::Toast::new(&format!(
                "Buckets could not be upgraded, so they can’t be changed yet. Nothing was lost: {error}"
            ));
            toast.set_timeout(0);
            toast_overlay.add_toast(toast);
        }
        let outer_split = libadwaita::OverlaySplitView::new();
        *self.imp().presentation_outer_split.borrow_mut() = Some(outer_split.clone());
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
            let content_stack = content_stack.clone();
            let open_folder_now: Rc<dyn Fn(PathBuf)> = Rc::new(move |path: PathBuf| {
                let disabled_folders = state_c.borrow().disabled_folders.clone();
                if path_is_disabled(&path, &disabled_folders) {
                    toast_overlay_c.add_toast(libadwaita::Toast::new("Folder is disabled"));
                    sidebar_c.set_folder_ignored(&path, true);
                    return;
                }
                // Dedup: if this exact folder is already the active scope and the
                // library is currently loading it, a concurrent startup trigger or
                // retry idle is redundant — skip to preserve the in-progress scan.
                {
                    let st = state_c.borrow();
                    if matches!(&st.scope, ViewScope::Folder(f) if f == &path)
                        && st.library.current_folder.as_deref() == Some(path.as_path())
                    {
                        return;
                    }
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
                    win.clear_inline_search(true);
                }
                let current_page = content_stack.visible_child_name().unwrap_or_default();
                if current_page == "viewer"
                    || current_page == "compare"
                    || current_page == "survey"
                    || current_page == "welcome"
                {
                    content_stack.set_visible_child_name("viewer");
                }
                let cache_max = AppSettings::load().thumbnail_cache_max as usize;
                {
                    let mut st = state_c.borrow_mut();
                    st.library.set_thumbnail_cache_max(cache_max);
                    st.library.reset_for_folder(&path);
                    st.settings
                        .set_active_library_last_folder(Some(path.clone()));
                    st.selected_paths.clear();
                    st.scope = ViewScope::Folder(path.clone());
                }
                if let Some(win) = window_weak.upgrade() {
                    win.update_empty_title();
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
                // Its own thread, never queued behind background work in the
                // shared rayon pool (a 2.3 s wait was measured on October 12).
                let _ = std::thread::Builder::new()
                    .name("folder-open".into())
                    .spawn(move || {
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
                                            "duration_ms": crate::bench::duration_ms(started),
                                        }),
                                    );
                                    let _ =
                                        tx.send_blocking(FolderOpenResult::Cached { rows: cached });
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
                let created_window = window_weak.clone();
                let path_rx = path.clone();
                let toast_overlay_rx = toast_overlay_c.clone();
                glib::MainContext::default().spawn_local(async move {
                    let Ok(first_result) = rx.recv().await else {
                        return;
                    };

                    if !is_active_folder_load(&state_rx.borrow(), path_rx.as_path()) {
                        crate::bench_event!(
                            "folder.open.stale_result",
                            serde_json::json!({ "path": path_rx.display().to_string() }),
                        );
                        return;
                    }

                    let mut metadata_pending = Vec::new();
                    let mut raw_hydration_paths: Vec<PathBuf> = Vec::new();
                    let mut got_cached = false;

                    // Load the first result into the store in chunks to keep the UI responsive.
                    {
                        let started = Instant::now();
                        let mut rows = match first_result {
                            FolderOpenResult::Cached { rows } => {
                                got_cached = true;
                                rows
                            }
                            FolderOpenResult::Indexed {
                                rows,
                                metadata_pending: pending,
                                ..
                            } => {
                                metadata_pending = pending;
                                rows
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
                                let order = state_rx.borrow().sort_order;
                                crate::model::library::sort_raw_entries(&mut raw_entries, order);
                                raw_hydration_paths = raw_entries.iter().map(|e| e.path.clone()).collect();
                                raw_entries
                                    .into_iter()
                                    .map(|raw| IndexedImage {
                                        path: raw.path.clone(),
                                        filename: raw.filename,
                                        extension: raw.path.extension()
                                            .and_then(|e| e.to_str())
                                            .unwrap_or_default()
                                            .to_lowercase(),
                                        file_size: raw.file_size,
                                        modified_secs: raw.modified.and_then(|m| {
                                            m.duration_since(std::time::UNIX_EPOCH).ok()
                                        }).map(|d| d.as_secs() as i64),
                                        width: None,
                                        height: None,
                                        metadata_status: "raw".to_string(),
                                    })
                                    .collect()
                            }
                        };

                        let total_rows = rows.len();
                        {
                            let mut st = state_rx.borrow_mut();
                            if !is_active_folder_load(&st, path_rx.as_path()) {
                                return;
                            }
                            st.library.start_load_folder(&path_rx);
                        }

                        const CHUNK_SIZE: usize = 100;
                        let mut current_offset = 0;

                        while current_offset < total_rows {
                            let chunk_end = (current_offset + CHUNK_SIZE).min(total_rows);
                            let chunk: Vec<_> = rows.drain(0..(chunk_end - current_offset)).collect();

                            {
                                let mut st = state_rx.borrow_mut();
                                // Check if we are still on the same folder before applying chunk.
                                if is_active_folder_load(&st, path_rx.as_path()) {
                                    st.library.append_indexed_rows(chunk);
                                } else {
                                    return; // Stale folder open.
                                }
                            }

                            current_offset = chunk_end;

                            // Yield control back to GTK main loop asynchronously to avoid re-entrancy panic.
                            let (tx, rx) = async_channel::bounded::<()>(1);
                            glib::idle_add_local_once(move || {
                                let _ = tx.try_send(());
                            });
                            let _ = rx.recv().await;
                        }

                        {
                            let mut st = state_rx.borrow_mut();
                            if !is_active_folder_load(&st, path_rx.as_path()) {
                                return;
                            }
                            st.library.finish_load_folder();
                        }

                        crate::bench_event!(
                            "folder.store_populate.finish",
                            serde_json::json!({
                                "path": path_rx.display().to_string(),
                                "entry_count": total_rows,
                                "duration_ms": crate::bench::duration_ms(started),
                            }),
                        );
                    }

                    if !is_active_folder_load(&state_rx.borrow(), path_rx.as_path()) {
                        return;
                    }

                    let scope = state_rx.borrow().scope.clone();
                    apply_scope_to_sidebar(&scope, &sidebar_rx);

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
                    // New or changed files may need fingerprints.
                    if let Some(win) = created_window.upgrade() {
                        win.request_fingerprints();
                    }

                    let target_index = {
                        let st = state_rx.borrow();
                        let created = created_window.upgrade().and_then(|win| {
                            let path = win.imp().pending_created_copy.borrow().clone();
                            let selected = path.as_ref().and_then(|path| st.library.index_of_path(path));
                            if selected.is_some() { win.imp().pending_created_copy.borrow_mut().take(); }
                            selected
                        });
                        created.or_else(|| st.library
                            .restore_index_for(&path_rx))
                            .filter(|&idx| st.library.entry_at(idx).is_some())
                            .or_else(|| st.library.entry_at(0).map(|_| 0))
                    };

                    if let Some(index) = target_index {
                        state_rx.borrow_mut().library.selected_index = Some(index);
                        filmstrip_rx.navigate_to(index);

                        // Ensure the viewer actually loads the first image on cold start,
                        // as navigate_to might be a no-op if the first item was already
                        // automatically selected by the model refresh.
                        if let Some(path) = state_rx.borrow().library.entry_at(index).map(|e| e.path()) {
                            viewer_rx.load_image(path);
                        }
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
                            if is_active_folder_load(&state_rx.borrow(), path_rx.as_path()) {
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
                                        let created = created_window.upgrade().and_then(|win| {
                                            let path = win.imp().pending_created_copy.borrow().clone();
                                            let selected = path.as_ref().and_then(|path| st.library.index_of_path(path));
                                            if selected.is_some() { win.imp().pending_created_copy.borrow_mut().take(); }
                                            selected
                                        });
                                        created.or_else(|| selected_path
                                            .as_deref()
                                            .and_then(|p| st.library.index_of_path(p)))
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

                                        if let Some(path) = state_rx.borrow().library.entry_at(idx).map(|e| e.path()) {
                                            viewer_rx.load_image(path);
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

                    start_raw_dimension_hydrator(
                        raw_hydration_paths,
                        path_rx.clone(),
                        state_rx.clone(),
                    );
                });
            });
            let state_retry = state.clone();
            Rc::new(move |path: PathBuf| {
                if state_retry.try_borrow_mut().is_err() {
                    let open_folder_retry = open_folder_now.clone();
                    glib::idle_add_local_once(move || open_folder_retry(path));
                    return;
                }
                open_folder_now(path);
            })
        };
        *self.imp().open_folder_cb.borrow_mut() = Some(open_folder.clone());

        // Helper: refresh the sidebar collection list from the DB.
        let refresh_sidebar_collections = {
            let sidebar_c = sidebar.clone();
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            let generation = Rc::new(Cell::new(0u64));
            move || {
                let next = generation.get().wrapping_add(1);
                generation.set(next);
                let (index, library, root, disabled) = {
                    let st = state_c.borrow();
                    (
                        st.library_index.clone(),
                        st.settings.active_library_id.clone(),
                        st.settings.active_library().map(|l| l.root.clone()),
                        st.disabled_folders.clone(),
                    )
                };
                let Some(index) = index else { return };
                let state = state_c.clone();
                let sidebar = sidebar_c.clone();
                let filmstrip = filmstrip_c.clone();
                let viewer = viewer_c.clone();
                let current_generation = generation.clone();
                let expected_library = library.clone();
                crate::ui::collection_dialogs::run_collection_write(
                    move || {
                        let mut collections = index
                            .list_collections_for_library(library.as_deref())
                            .map_err(|e| e.to_string())?;
                        for collection in &mut collections {
                            collection.item_count = crate::buckets::view(
                                &index,
                                collection.id,
                                root.as_deref(),
                                &disabled,
                            )?
                            .paths
                            .len();
                        }
                        Ok(collections)
                    },
                    move |result| {
                        if current_generation.get() != next
                            || state.borrow().settings.active_library_id != expected_library
                        {
                            return;
                        }
                        match result {
                            Ok(collections) => {
                                state.borrow_mut().collections = collections.clone();
                                filmstrip.refresh_collection_colors(&collections);
                                sidebar.refresh_collections(&collections);
                                viewer.refresh_membership();
                                apply_scope_to_sidebar(&state.borrow().scope, &sidebar);
                            }
                            Err(error) => crate::bench_warn!(
                                "collections.refresh.fail",
                                serde_json::json!({"error":error})
                            ),
                        }
                    },
                );
            }
        };
        *self.imp().refresh_sidebar_collections_cb.borrow_mut() =
            Some(Rc::new(refresh_sidebar_collections.clone()));

        // Sidebar folder selection → scan library → refresh filmstrip.
        {
            let open_folder_c = open_folder.clone();
            let window = self.downgrade();
            sidebar.connect_folder_selected(move |path| {
                if window
                    .upgrade()
                    .is_some_and(|w| file_actions_blocked(w.upcast_ref()))
                {
                    return;
                }
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
                    st.settings
                        .set_active_library_ignored_folders(disabled_folders.clone());
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
                        let _ = st.library.load_virtual(&[]);
                        st.scope = ViewScope::Search;
                    } else if !matches!(st.scope, ViewScope::Folder(_)) {
                        let visible_paths: Vec<PathBuf> = (0..st.library.image_count())
                            .filter_map(|i| st.library.entry_at(i).map(|e| e.path()))
                            .filter(|image_path| {
                                !path_is_disabled(image_path, &st.disabled_folders)
                            })
                            .collect();
                        let _ = st.library.load_virtual(&visible_paths);
                    }
                    st.disabled_folders.clone()
                };

                sidebar_c.set_ignored_folders(&disabled_folders);
                viewer_c.clear();
                filmstrip_c.refresh_virtual();
                if let Some(win) = window_weak.upgrade() {
                    let _ = win.bump_thumbnail_generation("folder.ignored_changed");
                    win.complete_thumbnail_ops();
                    win.library_scope_changed();
                }
                let label = if ignored {
                    "Folder disabled"
                } else {
                    "Folder enabled"
                };
                toast_overlay_c.add_toast(libadwaita::Toast::new(label));
            });
        }

        {
            let weak = self.downgrade();
            filmstrip.set_sort_order_changed_cb(move |order| {
                if let Some(win) = weak.upgrade() {
                    win.imp().state.borrow_mut().sort_order = order;
                    let query = win
                        .imp()
                        .inline_search_entry
                        .borrow()
                        .as_ref()
                        .map(|e| e.text().to_string())
                        .unwrap_or_default();
                    win.run_browse_query(query, true);
                }
            });
        }

        // Find Copies… (Phase 7): exact, resized, and converted copies.
        {
            let window_weak = self.downgrade();
            let copies = gio::SimpleAction::new("find-duplicates", None);
            copies.connect_activate(move |_, _| {
                if let Some(win) = window_weak.upgrade() {
                    crate::ui::copies_dialog::present(&win);
                }
            });
            self.add_action(&copies);
        }

        {
            let window_weak = self.downgrade();
            let cleanup = gio::SimpleAction::new("remove-automatic-tags", None);
            cleanup.connect_activate(move |_, _| {
                if let Some(win) = window_weak.upgrade() {
                    win.start_automatic_tag_cleanup();
                }
            });
            self.add_action(&cleanup);
            let window_weak = self.downgrade();
            let overview = gio::SimpleAction::new("library-overview", None);
            overview.connect_activate(move |_, _| {
                if let Some(win) = window_weak.upgrade() {
                    crate::ui::library_overview::present(&win);
                }
            });
            self.add_action(&overview);
            let window_weak = self.downgrade();
            let offer = gio::SimpleAction::new("tags-to-buckets", None);
            offer.connect_activate(move |_, _| {
                if let Some(win) = window_weak.upgrade() {
                    win.offer_tags_as_buckets(false);
                }
            });
            self.add_action(&offer);
        }

        // Populate sidebar collections on startup.
        refresh_sidebar_collections();

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
                let (index, active_root, disabled) = {
                    let state = state_c.borrow();
                    (
                        state.library_index.clone(),
                        state.settings.active_library().map(|lib| lib.root.clone()),
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
                        crate::buckets::view(&index, id, active_root.as_deref(), &disabled)
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
                        Ok(view) => {
                            if view.missing > 0 {
                                // Missing members are kept; say so, never drop them.
                                toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                                    "{} image{} in this bucket can’t be found",
                                    view.missing,
                                    if view.missing == 1 { "" } else { "s" }
                                )));
                            }
                            view.paths
                        }
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
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
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
                    filmstrip_c.clone(),
                    viewer_c.clone(),
                    sidebar_c.clone(),
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
                                "Could not delete collection: {error}"
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
                        "No target collection is available for reparenting",
                    ));
                    return;
                }

                let dialog = libadwaita::AlertDialog::new(Some("Move into Bucket"), None);
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

        // Dragging to a bucket changes tags transactionally in a worker.
        {
            let weak = self.downgrade();
            sidebar.connect_drop_paths_to_collection(move |id, paths| {
                if let Some(win) = weak.upgrade() {
                    win.change_collection_tags(id, paths, false);
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
            let content_stack_c = content_stack.clone();
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
                    let current_page = content_stack_c.visible_child_name().unwrap_or_default();
                    if current_page == "welcome" {
                        content_stack_c.set_visible_child_name("viewer");
                    }
                    viewer_c.load_image(path);
                }
            });
        }

        // Start draining thumbnail results.
        self.start_thumbnail_poll(state.clone(), filmstrip.clone());
        self.start_hash_poll(state.clone());
        self.start_sharpness_poll(state.clone(), viewer.clone());

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
            ops_indicator,
            preview_search_revealer,
            preview_search_entry,
        ) = self.build_viewer_header(&viewer_menu_btn);

        *self.imp().inline_search_entry.borrow_mut() = Some(preview_search_entry.clone());
        *self.imp().inline_search_revealer.borrow_mut() = Some(preview_search_revealer.clone());

        *self.imp().presentation_inner_split.borrow_mut() = Some(inner_split.clone());
        *self.imp().presentation_header.borrow_mut() = Some(viewer_header.clone());

        self.setup_actions(&viewer, state.clone());
        filmstrip.attach_viewer_menu(&viewer);

        let viewer_toolbar = libadwaita::ToolbarView::new();
        viewer_toolbar.add_top_bar(&viewer_header);
        // Look prototype, pass 2: the top row follows the columns below it.
        // The filmstrip's header holds the sidebar toggle and one Sort and
        // Filter control; search sits centred over the viewer (D26 notes).
        viewer_header.remove(&sidebar_toggle);
        viewer_header.set_show_start_title_buttons(false);
        if let Some((strip_header, sort_menu, actions)) = filmstrip.take_header_buttons() {
            self.insert_action_group("filmstrip-sort", Some(&actions));
            strip_header.add_css_class("sharpr-top");
            strip_header.set_show_start_title_buttons(false);
            strip_header.set_title_widget(Some(&gtk4::Box::new(gtk4::Orientation::Horizontal, 0)));
            strip_header.pack_start(&sidebar_toggle);
            // No name here: the sidebar already shows what is open (user,
            // October 15).
            let combined = gtk4::MenuButton::new();
            combined.set_icon_name("view-sort-descending-symbolic");
            combined.set_tooltip_text(Some("Sort and filter"));
            combined.add_css_class("flat");
            combined.set_menu_model(Some(&sort_and_filter_menu(&sort_menu)));
            strip_header.pack_end(&combined);
        }
        sidebar.quiet_header();
        filmstrip.add_css_class("sharpr-filmstrip");
        {
            // Option A: a filter from the menu adds the same chip as typing it.
            let add_chip = gio::SimpleAction::new("add-filter-chip", Some(glib::VariantTy::STRING));
            let weak = self.downgrade();
            add_chip.connect_activate(move |_, arg| {
                let (Some(win), Some(word)) = (weak.upgrade(), arg.and_then(|a| a.get::<String>()))
                else {
                    return;
                };
                win.show_inline_search();
                let entry = win.imp().inline_search_entry.borrow().clone();
                if let Some(entry) = entry {
                    let text = entry.text().to_string();
                    let joined = if text.trim().is_empty() {
                        format!("{word} ")
                    } else {
                        format!("{} {word} ", text.trim_end())
                    };
                    entry.set_text(&joined);
                    entry.set_position(-1);
                }
            });
            self.add_action(&add_chip);
        }
        // Typed filter chips sit in their own bar under the header and wrap,
        // so they never widen the window past its minimum size.
        let search_chips = gtk4::FlowBox::new();
        search_chips.set_selection_mode(gtk4::SelectionMode::None);
        search_chips.set_halign(gtk4::Align::Start);
        search_chips.set_max_children_per_line(32);
        search_chips.set_column_spacing(6);
        search_chips.set_row_spacing(6);
        search_chips.set_margin_start(12);
        search_chips.set_margin_end(12);
        search_chips.set_margin_top(6);
        search_chips.set_margin_bottom(6);
        // Filter words that complete what is being typed (decision D16).
        let search_suggestions = gtk4::FlowBox::new();
        search_suggestions.set_selection_mode(gtk4::SelectionMode::None);
        search_suggestions.set_halign(gtk4::Align::Start);
        search_suggestions.set_max_children_per_line(32);
        search_suggestions.set_column_spacing(4);
        search_suggestions.set_margin_start(12);
        search_suggestions.set_margin_end(12);
        search_suggestions.set_margin_bottom(6);
        search_suggestions.set_visible(false);
        // How much of the scope visual search covered and how well it matched
        // (principle 5), with more results on request.
        let search_status = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        search_status.set_margin_start(12);
        search_status.set_margin_end(12);
        search_status.set_margin_bottom(6);
        search_status.set_visible(false);
        let search_status_label = gtk4::Label::new(None);
        search_status_label.add_css_class("dim-label");
        search_status_label.set_wrap(true);
        search_status_label.set_xalign(0.0);
        search_status_label.set_hexpand(true);
        let search_more = gtk4::Button::with_label("Show More");
        search_more.add_css_class("flat");
        search_more.set_valign(gtk4::Align::Center);
        search_more.set_tooltip_text(Some("Show the next closest images"));
        {
            let weak = self.downgrade();
            search_more.connect_clicked(move |_| {
                if let Some(win) = weak.upgrade() {
                    win.show_more_visual_results();
                }
            });
        }
        let search_selected = gtk4::Label::new(None);
        search_selected.add_css_class("dim-label");
        search_selected.add_css_class("numeric");
        search_selected.set_visible(false);
        let search_add = gtk4::Button::with_label("Add to Bucket…");
        search_add.add_css_class("flat");
        search_add.set_valign(gtk4::Align::Center);
        search_add.set_tooltip_text(Some("Add these results to a new or existing bucket"));
        search_add.set_visible(false);
        {
            let weak = self.downgrade();
            search_add.connect_clicked(move |_| {
                if let Some(win) = weak.upgrade() {
                    win.add_search_results_to_bucket();
                }
            });
        }
        search_status.append(&search_status_label);
        search_status.append(&search_selected);
        search_status.append(&search_add);
        search_status.append(&search_more);
        let chip_bar = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        chip_bar.append(&search_chips);
        chip_bar.append(&search_suggestions);
        chip_bar.append(&search_status);
        *self.imp().search_status.borrow_mut() = Some(SearchStatus {
            bar: search_status,
            label: search_status_label,
            selected: search_selected,
            add: search_add,
            more: search_more,
        });
        let search_chips_revealer = gtk4::Revealer::new();
        search_chips_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
        search_chips_revealer.set_child(Some(&chip_bar));
        viewer_toolbar.add_top_bar(&search_chips_revealer);
        *self.imp().search_chips.borrow_mut() = Some(search_chips);
        *self.imp().search_suggestions.borrow_mut() = Some(search_suggestions);
        *self.imp().mode_header.borrow_mut() = Some(mode_header::ModeHeader::new(
            &viewer_header,
            &search_chips_revealer,
        ));
        *self.imp().search_chip_bar.borrow_mut() = Some(search_chips_revealer);

        use crate::ui::compare_page::ComparePage;
        let compare_page = ComparePage::new();
        *self.imp().compare_page.borrow_mut() = Some(compare_page.clone());
        content_stack.add_named(&viewer, Some("viewer"));
        content_stack.add_named(&compare_page, Some("compare"));
        self.setup_survey(&content_stack);

        {
            let window_weak = self.downgrade();
            compare_page.set_close_cb(move |survivors| {
                if let Some(window) = window_weak.upgrade() {
                    window.exit_compare_mode(survivors);
                }
            });
        }

        // Mode keys also work while a header control has focus.
        {
            let keys = gtk4::EventControllerKey::new();
            keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
            keys.connect_key_pressed(move |controller, key, _, modifiers| {
                controller
                    .widget()
                    .and_downcast::<SharprWindow>()
                    .map_or(glib::Propagation::Proceed, |window| {
                        window.handle_mode_key(key, modifiers)
                    })
            });
            self.add_controller(keys);
        }
        // Anything that switches the content away from Compare (opening a
        // folder, searching) closes it properly, keeping the survivors.
        {
            let page = compare_page.clone();
            let window_weak = self.downgrade();
            content_stack.connect_visible_child_name_notify(move |stack| {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                if stack.visible_child_name().as_deref() != Some("compare")
                    && is_comparing(window.upcast_ref())
                {
                    let page = page.clone();
                    glib::idle_add_local_once(move || {
                        if !page.survivors().is_empty() {
                            page.close();
                        }
                    });
                }
            });
        }

        let compare_action = gio::SimpleAction::new("compare", None);
        compare_action.set_enabled(false);
        {
            let window_weak = self.downgrade();
            compare_action.connect_activate(move |_, _| {
                if let Some(window) = window_weak.upgrade() {
                    window.compare_selected();
                }
            });
        }
        self.add_action(&compare_action);
        self.install_compare_shortcut();
        self.setup_bucket_repeat();
        self.setup_library_root();
        {
            let compare_action = compare_action.clone();
            let weak = self.downgrade();
            filmstrip.connect_action_selection_changed(move |count| {
                compare_action.set_enabled(count >= 2);
                if let Some(win) = weak.upgrade() {
                    win.show_search_selection(count);
                    if let Some(action) = win
                        .lookup_action("survey")
                        .and_downcast::<gio::SimpleAction>()
                    {
                        action.set_enabled(count >= 2);
                    }
                }
            });
        }

        content_stack.set_visible_child_name("welcome");

        viewer_toolbar.set_content(Some(&content_stack));
        viewer_toolbar.set_top_bar_style(libadwaita::ToolbarStyle::Flat);

        let viewer_page = libadwaita::NavigationPage::builder()
            .title("View")
            .tag("viewer")
            .child(&viewer_toolbar)
            .build();
        inner_split.set_content(Some(&viewer_page));

        // Outer split: explorer sidebar | inner_split.
        let sidebar_overlay = gtk4::Overlay::new();
        sidebar_overlay.set_child(Some(&sidebar));

        let sidebar_page = libadwaita::NavigationPage::builder()
            .title("Library")
            .tag("sidebar")
            .child(&sidebar_overlay)
            .build();
        outer_split.set_sidebar(Some(&sidebar_page));

        let content_col = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        inner_split.set_vexpand(true);
        content_col.append(&inner_split);
        let content_page = libadwaita::NavigationPage::builder()
            .title("Image Library")
            .tag("content")
            .child(&content_col)
            .build();
        outer_split.set_content(Some(&content_page));

        sidebar_toggle.set_active(true);
        sidebar_toggle
            .bind_property("active", &outer_split, "show-sidebar")
            .flags(glib::BindingFlags::SYNC_CREATE | glib::BindingFlags::BIDIRECTIONAL)
            .build();

        self.setup_mode_panel_guards(&outer_split, &inner_split);
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

        // Look prototype: a narrower sidebar leaves more room for the
        // filmstrip and the viewer.
        outer_split.set_min_sidebar_width(170.0);
        outer_split.set_max_sidebar_width(210.0);
        toast_overlay.set_child(Some(&outer_split));
        crate::ui::look::install(self);
        self.add_css_class("sharpr");
        // The user chose the readout face everywhere (pass 2);
        // SHARPR_LOOK_MONO=numbers shows the numbers-only version.
        self.set_mono_everywhere(std::env::var("SHARPR_LOOK_MONO").as_deref() != Ok("numbers"));
        {
            let motion = gtk4::EventControllerMotion::new();
            motion.set_propagation_phase(gtk4::PropagationPhase::Capture);
            let weak = self.downgrade();
            motion.connect_motion(move |_, x, y| {
                if let Some(win) = weak.upgrade() {
                    win.imp().last_pointer.set(Some((x, y)));
                }
            });
            self.add_controller(motion);
        }
        // The always-visible search box would otherwise take the first focus,
        // sending Delete and the arrow keys to it instead of the filmstrip.
        {
            let list = filmstrip.imp().list_view.clone();
            self.connect_map(move |_| {
                let list = list.clone();
                glib::idle_add_local_once(move || {
                    list.grab_focus();
                });
            });
        }

        self.set_content(Some(&toast_overlay));

        // Drive the indicator from the ops event channel.
        {
            let indicator = ops_indicator.clone();
            glib::MainContext::default().spawn_local(async move {
                use crate::ops::queue::OpEvent;
                while let Ok(event) = ops_rx.recv().await {
                    match event {
                        OpEvent::Added { id, title } => {
                            indicator.push_op(id, &title);
                        }
                        OpEvent::Progress { id, fraction } => {
                            indicator.update_op(id, fraction);
                        }
                        OpEvent::Completed(id) => {
                            indicator.complete_op(id);
                        }
                        OpEvent::Failed { id, msg } => {
                            indicator.fail_op(id, &msg);
                        }
                        OpEvent::Dismissed(id) => {
                            indicator.remove_op(id);
                        }
                    }
                }
            });
        }

        {
            let weak = self.downgrade();
            preview_search_entry.set_placeholder_text(Some("Describe images, or 4k png 16:9…"));
            preview_search_entry.set_search_delay(150);
            preview_search_entry.connect_search_changed(move |entry| {
                let Some(win) = weak.upgrade() else { return };
                if win.imp().inline_search_suppressed.get() {
                    return;
                }
                win.take_completed_search_chips(entry, false);
                win.run_scoped_search(entry.text().to_string());
            });
            let weak = self.downgrade();
            preview_search_entry.connect_activate(move |entry| {
                if let Some(win) = weak.upgrade() {
                    win.take_completed_search_chips(entry, true);
                }
            });
            let weak = self.downgrade();
            preview_search_entry.connect_changed(move |entry| {
                if let Some(win) = weak.upgrade() {
                    win.rebuild_search_suggestions(entry.text().as_str());
                }
            });
            let weak = self.downgrade();
            let key = gtk4::EventControllerKey::new();
            key.connect_key_pressed(move |_, key, _, _| {
                if key != gtk4::gdk::Key::Escape {
                    return glib::Propagation::Proceed;
                }
                if let Some(win) = weak.upgrade() {
                    win.close_browse_search();
                }
                glib::Propagation::Stop
            });
            preview_search_entry.add_controller(key);
        }

        {
            let content_stack_c = content_stack.clone();
            let window_weak = self.downgrade();
            let type_to_search = gtk4::EventControllerKey::new();
            type_to_search.connect_key_pressed(move |controller, key, _, state| {
                let Some(widget) = controller.widget() else {
                    return glib::Propagation::Proceed;
                };
                let Some(window) = widget.downcast_ref::<gtk4::Window>() else {
                    return glib::Propagation::Proceed;
                };
                if let Some(focus) = gtk4::prelude::GtkWindowExt::focus(window) {
                    if focus.is::<gtk4::Text>() || focus.is::<gtk4::SearchEntry>() {
                        return glib::Propagation::Proceed;
                    }
                }
                if !(state.is_empty() || state == gtk4::gdk::ModifierType::SHIFT_MASK) {
                    return glib::Propagation::Proceed;
                }
                let Some(ch) = key.to_unicode() else {
                    return glib::Propagation::Proceed;
                };
                // Space stays free for viewing and Compare (decision D7);
                // Compare's own keys include printable ones.
                if ch.is_control()
                    || ch.is_whitespace()
                    || file_actions_blocked(window.upcast_ref())
                {
                    return glib::Propagation::Proceed;
                }
                let Some(win) = window_weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let Some(revealer) = win.imp().inline_search_revealer.borrow().clone() else {
                    return glib::Propagation::Proceed;
                };
                let Some(entry) = win.imp().inline_search_entry.borrow().clone() else {
                    return glib::Propagation::Proceed;
                };
                content_stack_c.set_visible_child_name("viewer");
                if !revealer.reveals_child() {
                    revealer.set_reveal_child(true);
                    win.imp().inline_search_suppressed.set(true);
                    entry.set_text("");
                    win.imp().inline_search_suppressed.set(false);
                }
                let mut text = entry.text().to_string();
                text.push(ch);
                entry.set_text(&text);
                entry.set_can_focus(true);
                entry.grab_focus();
                entry.set_position(-1);
                glib::Propagation::Stop
            });
            self.add_controller(type_to_search);
        }

        {
            let win_w = self.downgrade();
            let state_c = state.clone();
            let toast_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            filmstrip.connect_save_search_as_collection(move |query| {
                let Some(win) = win_w.upgrade() else {
                    return;
                };
                let name = query.trim().to_string();
                if name.is_empty() {
                    return;
                }
                show_new_collection_dialog(
                    win.upcast(),
                    name,
                    state_c.clone(),
                    toast_c.clone(),
                    refresh_c.clone(),
                );
            });
        }

        // -----------------------------------------------------------------------
        // Alt+Left / Alt+Right — navigate between images.
        // Scoped to the window so it fires regardless of focus position.
        // -----------------------------------------------------------------------
        // Right-click → Info: show the clicked image, then its Info panel (D18).
        {
            let state_c = state.clone();
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            filmstrip.connect_info_requested(move |path| {
                let index = state_c.borrow().library.index_of_path(&path);
                if let Some(index) = index {
                    filmstrip_c.navigate_to(index);
                }
                viewer_c.show_info();
            });
        }

        // Crop, WebP and Resize Copy from the right-click menu.
        {
            let window_weak = self.downgrade();
            filmstrip.connect_copy_requested(move |kind, sources| {
                if let Some(window) = window_weak.upgrade() {
                    window.present_copy_dialog(kind, sources);
                }
            });
        }

        // "Add to Bucket…" context menu → pick / create collection → add paths.
        {
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let window_weak = self.downgrade();
            filmstrip.connect_add_to_collection_requested(move |paths| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                let Some(_idx) = state_c.borrow().library_index.clone() else {
                    toast_overlay_c.add_toast(libadwaita::Toast::new("Library index unavailable"));
                    return;
                };
                let collections = {
                    let state = state_c.borrow();
                    collections_for_sidebar(&state)
                };

                // Clicking a bucket only ticks it; nothing is added until
                // "Add", so a stray click can't add to the wrong one.
                // Look prototype: a compact picker at the pointer (I7).
                let heading = gtk4::Label::new(Some(&format!(
                    "Add {} {} to",
                    paths.len(),
                    if paths.len() == 1 { "image" } else { "images" }
                )));
                heading.add_css_class("heading");
                heading.set_xalign(0.0);
                let add_btn = gtk4::Button::with_label("Add");
                add_btn.add_css_class("suggested-action");
                add_btn.set_sensitive(false);
                let cancel_btn = gtk4::Button::with_label("Cancel");
                let ticks: Rc<RefCell<Vec<(i64, gtk4::CheckButton)>>> = Rc::default();

                let list_box = gtk4::ListBox::new();
                list_box.add_css_class("boxed-list");
                list_box.set_selection_mode(gtk4::SelectionMode::None);

                // "New Bucket…" entry at top
                let new_row = gtk4::ListBoxRow::new();
                let new_label = gtk4::Label::new(Some("New Bucket\u{2026}"));
                new_label.set_halign(gtk4::Align::Start);
                new_label.set_margin_top(8);
                new_label.set_margin_bottom(8);
                new_label.set_margin_start(12);
                new_row.set_child(Some(&new_label));
                list_box.append(&new_row);

                for (collection_id, label) in flatten_collection_paths(&collections) {
                    let row = gtk4::ListBoxRow::new();
                    unsafe {
                        row.set_data("collection-id", collection_id);
                    }
                    let line = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                    line.set_margin_top(6);
                    line.set_margin_bottom(6);
                    line.set_margin_start(12);
                    line.set_margin_end(12);
                    let check = gtk4::CheckButton::new();
                    {
                        let add_btn = add_btn.downgrade();
                        let ticks = ticks.clone();
                        check.connect_toggled(move |_| {
                            if let Some(add_btn) = add_btn.upgrade() {
                                let any = ticks.borrow().iter().any(|(_, c)| c.is_active());
                                add_btn.set_sensitive(any);
                            }
                        });
                    }
                    ticks.borrow_mut().push((collection_id, check.clone()));
                    let lbl = gtk4::Label::new(Some(&label));
                    lbl.set_halign(gtk4::Align::Start);
                    line.append(&check);
                    line.append(&lbl);
                    row.set_child(Some(&line));
                    list_box.append(&row);
                }

                let scroll = gtk4::ScrolledWindow::new();
                scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
                scroll.set_max_content_height(300);
                scroll.set_propagate_natural_height(true);
                scroll.set_child(Some(&list_box));

                scroll.set_min_content_width(220);
                let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
                buttons.set_homogeneous(true);
                buttons.append(&cancel_btn);
                buttons.append(&add_btn);
                let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                content.append(&heading);
                content.append(&scroll);
                content.append(&buttons);
                let popover = win.popover_at_pointer(&content);
                popover.set_default_widget(Some(&add_btn));
                {
                    let popover = popover.downgrade();
                    cancel_btn.connect_clicked(move |_| {
                        if let Some(popover) = popover.upgrade() {
                            popover.popdown();
                        }
                    });
                }
                {
                    let ticks = ticks.clone();
                    let paths = paths.clone();
                    let win_weak = win.downgrade();
                    let popover = popover.downgrade();
                    add_btn.connect_clicked(move |_| {
                        let Some(win) = win_weak.upgrade() else {
                            return;
                        };
                        for (id, check) in ticks.borrow().iter() {
                            if check.is_active() {
                                win.change_collection_tags(*id, paths.clone(), false);
                            }
                        }
                        if let Some(popover) = popover.upgrade() {
                            popover.popdown();
                        }
                    });
                }

                let paths_c = paths.clone();
                let win_weak2 = win.downgrade();
                let popover_weak = popover.downgrade();
                list_box.connect_row_activated(move |_, row| {
                    let coll_id: Option<i64> =
                        unsafe { row.data("collection-id").map(|p| *p.as_ref()) };
                    if coll_id.is_none() {
                        // New collection
                        let Some(win2) = win_weak2.upgrade() else {
                            return;
                        };
                        if let Some(popover) = popover_weak.upgrade() {
                            popover.popdown();
                        }
                        let new_dialog = libadwaita::AlertDialog::new(Some("New Bucket"), None);
                        new_dialog.add_response("cancel", "Cancel");
                        new_dialog.add_response("create", "Create");
                        new_dialog.set_default_response(Some("create"));
                        new_dialog.set_close_response("cancel");
                        new_dialog.set_response_appearance(
                            "create",
                            libadwaita::ResponseAppearance::Suggested,
                        );
                        let entry = gtk4::Entry::new();
                        entry.set_placeholder_text(Some("Bucket name"));
                        let eb = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
                        eb.set_margin_top(6);
                        eb.append(&entry);
                        new_dialog.set_extra_child(Some(&eb));
                        let paths_cc = paths_c.clone();
                        let window_weak = win2.downgrade();
                        let create_collection = Rc::new(move |name: String| {
                            if let Some(window) = window_weak.upgrade() {
                                window.create_collection_with_paths(
                                    name,
                                    Vec::new(),
                                    paths_cc.clone(),
                                );
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
                    // A bucket row only toggles its tick.
                    if let Some(id) = coll_id {
                        if let Some((_, check)) = ticks.borrow().iter().find(|(i, _)| *i == id) {
                            check.set_active(!check.is_active());
                        }
                    }
                });
            });
        }

        // Removing bucket membership uses the same transactional worker.
        {
            let weak = self.downgrade();
            filmstrip.connect_remove_from_collection_requested(move |paths| {
                if let Some(win) = weak.upgrade() {
                    let scope = win.imp().state.borrow().scope.clone();
                    if let ViewScope::Collection(id) = scope {
                        win.change_collection_tags(id, paths, true);
                    }
                }
            });
        }

        self.setup_nav_shortcuts(
            state.clone(),
            filmstrip.clone(),
            viewer.clone(),
            outer_split.clone(),
            content_stack.clone(),
        );
        sidebar.refresh_active_library(state.clone());

        // -----------------------------------------------------------------------
        // Restore last folder
        // -----------------------------------------------------------------------
        let last_folder = state.borrow().settings.last_folder.clone();
        let start_folder = last_folder
            .filter(|p| p.is_dir())
            .or_else(|| sidebar.first_folder_path());

        // Defer by one idle cycle so widgets are realized before we call
        // open_folder() and apply its initial sidebar selection.
        //
        // Guard: if the sidebar's folder-tree rebuild callback already fired and
        // opened this same folder (via emit_folder_selected), skip the duplicate
        // open to avoid bumping the thumbnail generation twice and discarding the
        // in-progress folder scan.
        if let Some(folder) = start_folder {
            let state_startup = state.clone();
            glib::idle_add_local_once(move || {
                if matches!(&state_startup.borrow().scope, ViewScope::Folder(f) if f == &folder) {
                    return;
                }
                open_folder(folder);
            });
        }
    }

    pub fn enter_presentation_mode(&self) {
        let imp = self.imp();
        if imp.presentation_mode.get() {
            return;
        }
        let outer = imp.presentation_outer_split.borrow();
        let inner = imp.presentation_inner_split.borrow();
        let hdr = imp.presentation_header.borrow();
        if let (Some(o), Some(i), Some(h)) = (outer.as_ref(), inner.as_ref(), hdr.as_ref()) {
            imp.presentation_sidebar_was_visible.set(o.shows_sidebar());
            imp.presentation_filmstrip_was_visible
                .set(i.shows_sidebar());
            o.set_show_sidebar(false);
            i.set_show_sidebar(false);
            h.set_visible(
                imp.mode_header
                    .borrow()
                    .as_ref()
                    .is_some_and(mode_header::ModeHeader::active),
            );
        }
        imp.presentation_mode.set(true);
        self.fullscreen();
    }

    pub fn exit_presentation_mode(&self) {
        let imp = self.imp();
        if !imp.presentation_mode.get() {
            return;
        }
        let outer = imp.presentation_outer_split.borrow();
        let inner = imp.presentation_inner_split.borrow();
        let hdr = imp.presentation_header.borrow();
        if let (Some(o), Some(i), Some(h)) = (outer.as_ref(), inner.as_ref(), hdr.as_ref()) {
            o.set_show_sidebar(imp.presentation_sidebar_was_visible.get());
            i.set_show_sidebar(imp.presentation_filmstrip_was_visible.get());
            h.set_visible(true);
        }
        imp.presentation_mode.set(false);
        self.unfullscreen();

        if let Some(viewer) = imp.viewer.borrow().as_ref() {
            let viewer = viewer.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
                viewer.reset_zoom();
            });
        }
    }

    pub fn toggle_presentation_mode(&self) {
        if self.imp().presentation_mode.get() {
            self.exit_presentation_mode();
        } else {
            self.enter_presentation_mode();
        }
    }

    /// Wire Alt+Left / Alt+Right to advance the image selection.
    fn setup_nav_shortcuts(
        &self,
        state: Rc<RefCell<AppState>>,
        filmstrip: FilmstripPane,
        viewer: ViewerPane,
        outer_split: libadwaita::OverlaySplitView,
        content_stack: gtk4::Stack,
    ) {
        let shortcuts = gtk4::ShortcutController::new();
        // Managed scope means the window itself handles these even when a child
        // widget has focus.
        shortcuts.set_scope(gtk4::ShortcutScope::Managed);

        let make_action = |state: Rc<RefCell<AppState>>,
                           filmstrip: FilmstripPane,
                           _viewer: ViewerPane,
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
                if let Some(win) = widget.downcast_ref::<SharprWindow>() {
                    if win.imp().presentation_mode.get() {
                        win.exit_presentation_mode();
                    } else if win.is_fullscreen() {
                        win.unfullscreen();
                    } else {
                        win.fullscreen();
                    }
                }
                glib::Propagation::Stop
            })),
        ));

        // Escape — Exit presentation mode.
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("Escape").unwrap()),
            Some(gtk4::CallbackAction::new(move |widget, _| {
                if let Some(win) = widget.downcast_ref::<SharprWindow>() {
                    if win
                        .imp()
                        .viewer
                        .borrow()
                        .as_ref()
                        .is_some_and(|viewer| viewer.hide_info())
                    {
                        return glib::Propagation::Stop;
                    }
                    if win.imp().presentation_mode.get() {
                        win.exit_presentation_mode();
                        return glib::Propagation::Stop;
                    }
                }
                glib::Propagation::Proceed
            })),
        ));

        // Presentation mode navigation (Global scope to override ScrolledWindow arrow consumption).
        let presentation_shortcuts = gtk4::ShortcutController::new();
        presentation_shortcuts.set_scope(gtk4::ShortcutScope::Global);
        presentation_shortcuts.set_propagation_phase(gtk4::PropagationPhase::Capture);

        let make_presentation_nav =
            |state: Rc<RefCell<AppState>>, filmstrip: FilmstripPane, delta: i32| {
                gtk4::CallbackAction::new(move |widget, _| {
                    if let Some(win) = widget.downcast_ref::<SharprWindow>() {
                        if !win.imp().presentation_mode.get() || file_actions_blocked(widget) {
                            return glib::Propagation::Proceed;
                        }
                        // Avoid triggering if we are in a text entry (e.g. search)
                        if let Some(focus) = gtk4::prelude::GtkWindowExt::focus(win) {
                            if focus.is::<gtk4::Text>() || focus.is::<gtk4::SearchEntry>() {
                                return glib::Propagation::Proceed;
                            }
                        }
                    } else {
                        return glib::Propagation::Proceed;
                    }

                    let new_index = {
                        let Ok(mut st) = state.try_borrow_mut() else {
                            return glib::Propagation::Proceed;
                        };
                        if delta == -999999 {
                            st.library.selected_index = Some(0);
                            Some(0)
                        } else if delta == 999999 {
                            let last = st.library.image_count().saturating_sub(1);
                            st.library.selected_index = Some(last);
                            Some(last)
                        } else {
                            st.library.navigate(delta)
                        }
                    };
                    if let Some(index) = new_index {
                        filmstrip.navigate_to(index);
                    }
                    glib::Propagation::Stop
                })
            };

        for (trigger, delta) in [
            ("Up", -1),
            ("Down", 1),
            ("Left", -1),
            ("Right", 1),
            ("Page_Up", -5),
            ("Page_Down", 5),
            ("Home", -999999),
            ("End", 999999),
        ] {
            presentation_shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string(trigger).unwrap()),
                Some(make_presentation_nav(
                    state.clone(),
                    filmstrip.clone(),
                    delta,
                )),
            ));
        }
        self.add_controller(presentation_shortcuts);

        // F9 — Toggle Library sidebar.
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("F9").unwrap()),
            Some(gtk4::CallbackAction::new(move |widget, _| {
                if file_actions_blocked(widget) {
                    return glib::Propagation::Stop;
                }
                outer_split.set_show_sidebar(!outer_split.shows_sidebar());
                glib::Propagation::Stop
            })),
        ));

        // Ctrl+F — reveal and focus inline search.
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Control>F").unwrap()),
            Some(gtk4::CallbackAction::new(move |widget, _| {
                if let Some(window) = widget.downcast_ref::<SharprWindow>() {
                    if file_actions_blocked(window.upcast_ref()) {
                        return glib::Propagation::Stop;
                    }
                    content_stack.set_visible_child_name("viewer");
                    window.show_inline_search();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            })),
        ));

        // Del — Move selected image(s) to trash (respects multi-selection).
        {
            let state_d = state.clone();
            let filmstrip_d = filmstrip.clone();
            let viewer_d = viewer.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("Delete").unwrap()),
                Some(gtk4::CallbackAction::new(move |widget, _| {
                    if text_input_has_focus(widget) || file_actions_blocked(widget) {
                        return glib::Propagation::Proceed;
                    }
                    let paths = filmstrip_d.selected_paths_for_action();
                    if paths.is_empty() {
                        return glib::Propagation::Proceed;
                    }
                    trash_paths(
                        widget.downcast_ref::<SharprWindow>(),
                        &state_d,
                        &filmstrip_d,
                        &viewer_d,
                        &paths,
                    );
                    glib::Propagation::Stop
                })),
            ));
        }

        // Shift+F10 / Menu — the current image's right-click menu (decision D21).
        {
            let filmstrip_m = filmstrip.clone();
            let viewer_m = viewer.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("<Shift>F10|Menu").unwrap()),
                Some(gtk4::CallbackAction::new(move |widget, _| {
                    if !text_input_has_focus(widget)
                        && !file_actions_blocked(widget)
                        && (filmstrip_m.open_context_menu_for_current()
                            || filmstrip_m.open_viewer_menu(
                                &viewer_m,
                                f64::from(viewer_m.width()) / 2.0,
                                f64::from(viewer_m.height()) / 2.0,
                            ))
                    {
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                })),
            ));
        }

        // Ctrl+I — the current image's Info panel (decision D18).
        {
            let viewer_i = viewer.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("<Control>i").unwrap()),
                Some(gtk4::CallbackAction::new(move |_, _| {
                    viewer_i.show_info();
                    glib::Propagation::Stop
                })),
            ));
        }

        // Ctrl+B — add the selected images to a bucket (decision D5).
        {
            let filmstrip_b = filmstrip.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("<Control>b").unwrap()),
                Some(gtk4::CallbackAction::new(move |_, _| {
                    if filmstrip_b.request_add_selection_to_bucket() {
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                })),
            ));
        }

        // Ctrl+E — export the selection as WebP.
        {
            let filmstrip_e = filmstrip.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("<Control>e").unwrap()),
                Some(gtk4::CallbackAction::new(move |widget, _| {
                    if text_input_has_focus(widget) {
                        return glib::Propagation::Proceed;
                    }
                    if filmstrip_e.request_export_selection() {
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                })),
            ));
        }

        // Ctrl+A — select every image shown (ignored while typing).
        {
            let filmstrip_a = filmstrip.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("<Control>a").unwrap()),
                Some(gtk4::CallbackAction::new(move |widget, _| {
                    if text_input_has_focus(widget) {
                        return glib::Propagation::Proceed;
                    }
                    filmstrip_a.select_all();
                    glib::Propagation::Stop
                })),
            ));
        }

        // F2 — Rename selected image (single-select only; ignored when text entry has focus).
        {
            let filmstrip_f2 = filmstrip.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("F2").unwrap()),
                Some(gtk4::CallbackAction::new(move |widget, _| {
                    if text_input_has_focus(widget) || file_actions_blocked(widget) {
                        return glib::Propagation::Proceed;
                    }
                    if let Some(path) = filmstrip_f2.selected_single_path() {
                        filmstrip_f2.emit_rename_requested(path);
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                })),
            ));
        }

        // Ctrl+D — Duplicate selected image (single-select only).
        {
            let filmstrip_dup = filmstrip.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("<Control>d").unwrap()),
                Some(gtk4::CallbackAction::new(move |widget, _| {
                    if file_actions_blocked(widget) {
                        return glib::Propagation::Proceed;
                    }
                    if let Some(path) = filmstrip_dup.selected_single_path() {
                        filmstrip_dup.emit_duplicate_requested(path);
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                })),
            ));
        }

        // Context-menu "Move to Trash" from filmstrip.
        {
            let state_tr = state.clone();
            let filmstrip_tr = filmstrip.clone();
            let viewer_tr = viewer.clone();
            let window_weak = self.downgrade();
            filmstrip.connect_trash_requested(move |paths| {
                let window = window_weak.upgrade();
                trash_paths(
                    window.as_ref(),
                    &state_tr,
                    &filmstrip_tr,
                    &viewer_tr,
                    &paths,
                );
            });
        }

        // "Copy to Folder…" — pick a destination folder and copy selected files there.
        {
            let window_weak = self.downgrade();
            filmstrip.connect_copy_to_folder_requested(move |paths| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                let window_ptr = win.clone().upcast::<gtk4::Window>();
                let win_weak = win.downgrade();
                let dialog = gtk4::FileDialog::new();
                dialog.set_title("Copy to Folder");
                dialog.select_folder(
                    Some(&window_ptr),
                    None::<&gio::Cancellable>,
                    move |result| {
                        let Ok(file) = result else { return };
                        let Some(dest_dir) = file.path() else { return };
                        let mut copied = 0usize;
                        let mut errors = 0usize;
                        for src in &paths {
                            match crate::file_ops::copy_to_folder(src, &dest_dir) {
                                Ok(_) => copied += 1,
                                Err(_) => errors += 1,
                            }
                        }
                        let msg = if errors == 0 {
                            format!(
                                "Copied {} file{}",
                                copied,
                                if copied == 1 { "" } else { "s" }
                            )
                        } else {
                            format!(
                                "Copied {} file{}, {} failed",
                                copied,
                                if copied == 1 { "" } else { "s" },
                                errors
                            )
                        };
                        if let Some(w) = win_weak.upgrade() {
                            w.add_toast(libadwaita::Toast::new(&msg));
                        }
                    },
                );
            });
        }

        // "Move to Folder…" — pick a destination folder and move selected files there.
        {
            let state_mv = state.clone();
            let window_weak = self.downgrade();
            let filmstrip_mv = filmstrip.clone();
            let viewer_mv = viewer.clone();
            filmstrip.connect_move_to_folder_requested(move |paths| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                let window_ptr = win.clone().upcast::<gtk4::Window>();
                let state_c = state_mv.clone();
                let filmstrip_c = filmstrip_mv.clone();
                let viewer_c = viewer_mv.clone();
                let win_weak = win.downgrade();
                let dialog = gtk4::FileDialog::new();
                dialog.set_title("Move to Folder");
                dialog.select_folder(
                    Some(&window_ptr),
                    None::<&gio::Cancellable>,
                    move |result| {
                        let Ok(file) = result else { return };
                        let Some(dest_dir) = file.path() else { return };
                        let mut moved = 0usize;
                        let mut errors = 0usize;
                        let mut first_error = None;
                        let (Some(index), Some(tags_db)) = (
                            state_c.borrow().library_index.clone(),
                            state_c.borrow().tags.clone(),
                        ) else {
                            if let Some(w) = win_weak.upgrade() {
                                w.add_toast(libadwaita::Toast::new(
                                    "Cannot move files: bucket or tag database is unavailable",
                                ));
                            }
                            return;
                        };
                        // A name whose missing file still has tags counts as taken.
                        let taken = |p: &std::path::Path| {
                            p.symlink_metadata().is_ok()
                                || !tags_db.tags_for_path(p).is_empty()
                                || index.retains_bucket_path(p).unwrap_or(false)
                        };
                        for src in &paths {
                            let stem = src
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or_default()
                                .to_string();
                            let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
                            // Re-pick a free name if one appears between choosing and moving.
                            let mut result = Err("No unused name available".to_string());
                            for _ in 0..5 {
                                let dest = crate::file_ops::next_available_path(
                                    &dest_dir, &stem, ext, taken,
                                );
                                result = index.relocate_file(src, &dest, &tags_db);
                                if result.is_ok() || !taken(&dest) {
                                    break;
                                }
                            }
                            match result {
                                Ok(()) => {
                                    {
                                        let mut state = state_c.borrow_mut();
                                        state.library.remove_path(src);
                                        remove_path_from_action_selection(&mut state, src);
                                    }
                                    moved += 1;
                                }
                                Err(error) => {
                                    errors += 1;
                                    first_error.get_or_insert(error);
                                }
                            }
                        }
                        if moved > 0 {
                            let new_count = state_c.borrow().library.image_count();
                            if new_count == 0 {
                                viewer_c.clear();
                                filmstrip_c.refresh_multi_selection_visuals();
                            } else {
                                let index = state_c.borrow().library.selected_index.unwrap_or(0);
                                let new_index = index.min(new_count - 1);
                                filmstrip_c.navigate_to(new_index);
                                let next_path = state_c
                                    .borrow()
                                    .library
                                    .entry_at(new_index)
                                    .map(|e: ImageEntry| e.path());
                                if let Some(p) = next_path {
                                    filmstrip_c.set_action_selection_to_path(&p);
                                }
                            }
                        }
                        let mut msg = if errors == 0 {
                            format!("Moved {} file{}", moved, if moved == 1 { "" } else { "s" })
                        } else {
                            format!(
                                "Moved {} file{}, {} failed",
                                moved,
                                if moved == 1 { "" } else { "s" },
                                errors
                            )
                        };
                        if let Some(error) = first_error {
                            msg.push_str(&format!(": {error}"));
                        }
                        if let Some(w) = win_weak.upgrade() {
                            w.add_toast(libadwaita::Toast::new(&msg));
                        }
                    },
                );
            });
        }

        // "Rename…" — show a dialog pre-filled with the current filename, then rename on confirm.
        {
            let state_rn = state.clone();
            let window_weak = self.downgrade();
            let filmstrip_rn = filmstrip.clone();
            let viewer_rn = viewer.clone();
            filmstrip.connect_rename_requested(move |path| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                let current_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let dialog = libadwaita::AlertDialog::new(Some("Rename"), None);
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("rename", "Rename");
                dialog.set_default_response(Some("rename"));
                dialog.set_close_response("cancel");
                dialog.set_response_appearance("rename", libadwaita::ResponseAppearance::Suggested);
                let entry = gtk4::Entry::new();
                entry.set_text(&current_name);
                entry.select_region(0, -1);
                dialog.set_extra_child(Some(&entry));

                let state_d = state_rn.clone();
                let filmstrip_d = filmstrip_rn.clone();
                let viewer_d = viewer_rn.clone();
                let win_weak = win.downgrade();
                let entry_clone = entry.clone();
                dialog.connect_response(None, move |_, response| {
                    if response != "rename" {
                        return;
                    }
                    let new_name = entry_clone.text().to_string();
                    if new_name == current_name {
                        return;
                    }
                    let result = (|| -> Result<PathBuf, String> {
                        let dest = crate::file_ops::rename_destination(&path, &new_name)?;
                        let (index, tags) = {
                            let state = state_d.borrow();
                            (state.library_index.clone(), state.tags.clone())
                        };
                        let (Some(index), Some(tags)) = (index, tags) else {
                            return Err(
                                "Cannot rename: bucket or tag database is unavailable".into()
                            );
                        };
                        index.relocate_file(&path, &dest, &tags)?;
                        Ok(dest)
                    })();
                    let msg = match result {
                        Ok(_new_path) => {
                            {
                                let mut state = state_d.borrow_mut();
                                state.library.remove_path(&path);
                                remove_path_from_action_selection(&mut state, &path);
                            }
                            let new_count = state_d.borrow().library.image_count();
                            if new_count == 0 {
                                viewer_d.clear();
                                filmstrip_d.refresh_multi_selection_visuals();
                            } else {
                                let index = state_d.borrow().library.selected_index.unwrap_or(0);
                                let new_index = index.min(new_count - 1);
                                filmstrip_d.navigate_to(new_index);
                                let next_path = state_d
                                    .borrow()
                                    .library
                                    .entry_at(new_index)
                                    .map(|e: ImageEntry| e.path());
                                if let Some(p) = next_path {
                                    filmstrip_d.set_action_selection_to_path(&p);
                                }
                            }
                            "File renamed".to_string()
                        }
                        Err(error) => error,
                    };
                    if let Some(w) = win_weak.upgrade() {
                        w.add_toast(libadwaita::Toast::new(&msg));
                    }
                });
                dialog.present(Some(&win));
            });
        }

        // "Duplicate…" — copy file in-place with a non-colliding name, no dialog.
        {
            let window_weak = self.downgrade();
            filmstrip.connect_duplicate_requested(move |path| {
                let msg = match crate::file_ops::duplicate_file(&path) {
                    Ok(dest) => {
                        if let Some(win) = window_weak.upgrade() {
                            win.add_edit_copies(
                                crate::buckets::EditKind::Duplicated,
                                vec![(path.clone(), dest.clone())],
                            );
                        }
                        let name = dest
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("copy")
                            .to_string();
                        format!("Duplicated as \"{name}\"")
                    }
                    Err(e) => format!("Duplicate failed: {e}"),
                };
                if let Some(win) = window_weak.upgrade() {
                    win.add_toast(libadwaita::Toast::new(&msg));
                }
            });
        }

        self.add_controller(shortcuts);
    }

    fn show_inline_search(&self) {
        if file_actions_blocked(self.upcast_ref()) {
            return;
        }
        if let Some(revealer) = self.imp().inline_search_revealer.borrow().as_ref() {
            revealer.set_reveal_child(true);
        }
        if let Some(entry) = self.imp().inline_search_entry.borrow().as_ref() {
            entry.set_can_focus(true);
            entry.grab_focus();
            entry.set_position(-1);
        }
    }

    pub(super) fn clear_browse_filters(&self) {
        self.clear_inline_search(true);
    }

    pub(super) fn close_browse_search(&self) {
        let snapshot = self.imp().browse_snapshot.borrow_mut().take();
        self.clear_inline_search(true);
        *self.imp().browse_snapshot.borrow_mut() = snapshot;
        self.restore_browse_view();
    }

    fn clear_inline_search(&self, hide: bool) {
        self.imp().inline_search_suppressed.set(true);
        if let Some(button) = self.imp().search_whole_library.borrow().as_ref() {
            button.set_active(false);
        }
        self.imp().browse_snapshot.borrow_mut().take();
        *self.imp().browse_options.borrow_mut() = Default::default();
        self.show_search_status(None);
        self.rebuild_search_chips();
        let next_gen = self.imp().inline_search_generation.get().wrapping_add(1);
        self.imp().inline_search_generation.set(next_gen);
        self.imp()
            .browse_generation
            .store(next_gen, std::sync::atomic::Ordering::Relaxed);
        self.imp().inline_search_suppressed.set(true);
        if let Some(entry) = self.imp().inline_search_entry.borrow().as_ref() {
            entry.set_text("");
            entry.set_placeholder_text(Some("Describe images, or 4k png 16:9…"));
        }
        self.imp().inline_search_suppressed.set(false);
        if hide {
            if let Some(revealer) = self.imp().inline_search_revealer.borrow().as_ref() {
                revealer.set_reveal_child(false);
            }
            // The box stays on screen but gives the keys back to the filmstrip.
            if let Some(entry) = self.imp().inline_search_entry.borrow().as_ref() {
                entry.set_can_focus(false);
            }
            if let Some(filmstrip) = self.imp().filmstrip.borrow().as_ref() {
                filmstrip.imp().list_view.grab_focus();
            }
        }
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

    fn complete_thumbnail_ops(&self) {
        if let Some(op) = self.imp().global_thumbnail_op.borrow_mut().take() {
            op.complete();
        }
    }

    fn start_thumbnail_poll(&self, state: Rc<RefCell<AppState>>, filmstrip: FilmstripPane) {
        let result_rx = self.imp().result_rx.borrow().clone();
        let Some(rx) = result_rx else { return };
        let window_weak = self.downgrade();

        glib::MainContext::default().spawn_local(async move {
            while let Ok(response) = rx.recv().await {
                use gdk4::{MemoryFormat, MemoryTexture};
                use glib::Bytes;

                let (result_path, result_data) = match response {
                    ThumbnailWorkerResponse::Success(res) => (res.path.clone(), Some(res)),
                    ThumbnailWorkerResponse::Failed { path } => (path, None),
                };

                if let Some(result) = result_data {
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

                    // Virtual views can contain duplicate paths. Update every matching
                    // row there; normal folders keep the O(1) path lookup.
                    let mut applied_indices = Vec::new();
                    {
                        let st = state.borrow();
                        let entries = st.library.entries_for_path(&result_path);
                        if entries.is_empty() {
                            crate::bench_event!(
                                "thumbnail.apply_skipped",
                                serde_json::json!({
                                    "path": result_path.display().to_string(),
                                    "source": source,
                                    "reason": "not_in_active_view",
                                }),
                            );
                        } else {
                            for (idx, entry) in entries {
                                entry
                                    .set_thumbnail(Some(texture.clone().upcast::<gdk4::Texture>()));
                                applied_indices.push(idx);
                            }
                        }
                    }

                    // Cache the texture in LibraryManager (needs mut borrow).
                    {
                        let mut st = state.borrow_mut();
                        st.library.insert_thumbnail(
                            result_path.clone(),
                            texture.upcast(),
                            !applied_indices.is_empty(),
                        );
                        if !applied_indices.is_empty() {
                            let (active_cache_len, global_cache_len, global_cache_max) =
                                st.library.thumbnail_cache_stats();
                            crate::bench_event!(
                                "thumbnail.apply",
                                serde_json::json!({
                                    "path": result_path.display().to_string(),
                                    "indices": applied_indices,
                                    "source": source,
                                    "worker_ms": worker_ms,
                                    "active_cache_len": active_cache_len,
                                    "global_cache_len": global_cache_len,
                                    "global_cache_max": global_cache_max,
                                }),
                            );
                        }
                    }
                } else {
                    // Handle failure by marking the entry as failed in the library.
                    let st = state.borrow();
                    for (_, entry) in st.library.entries_for_path(&result_path) {
                        entry.set_thumbnail_failed(true);
                    }
                }

                filmstrip.mark_thumbnail_ready(&result_path);
            }

            if let Some(window) = window_weak.upgrade() {
                if let Some(op) = window.imp().global_thumbnail_op.borrow_mut().take() {
                    op.complete();
                }
            }
        });
    }

    fn start_hash_poll(&self, state: Rc<RefCell<AppState>>) {
        let hash_rx = self.imp().hash_result_rx.borrow().clone();
        let Some(rx) = hash_rx else { return };
        let persist = state.borrow().library_index.clone().map(spawn_phash_writer);

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
                if let Some(persist) = persist.as_ref() {
                    let _ = persist.try_send((path, hash));
                }
            }
        });
    }

    fn start_sharpness_poll(&self, state: Rc<RefCell<AppState>>, viewer: ViewerPane) {
        let rx = self.imp().sharpness_result_rx.borrow().clone();
        let Some(rx) = rx else { return };

        glib::MainContext::default().spawn_local(async move {
            while let Ok(result) = rx.recv().await {
                let norm = crate::quality::blur::normalize_sharpness(result.score);

                // Update the ImageEntry in the library model.
                if let Some(entry) = state.borrow().library.entry_for_path(&result.path) {
                    entry.set_sharpness_score(result.score);
                }

                // Update the viewer chip if this image is currently displayed.
                viewer.apply_sharpness(result.path.as_path(), norm);
            }
        });
    }

    fn build_primary_menu() -> gio::Menu {
        let menu = gio::Menu::new();

        let library_section = gio::Menu::new();
        library_section.append(Some("Open Library Root…"), Some("win.choose-library-root"));
        let copies = gio::MenuItem::new(Some("Find Copies…"), Some("win.find-duplicates"));
        copies.set_icon(&gio::ThemedIcon::new("sharpr-copies-symbolic"));
        library_section.append_item(&copies);
        library_section.append(Some("Library Overview…"), Some("win.library-overview"));
        menu.append_section(Some("Library"), &library_section);

        let app_section = gio::Menu::new();
        app_section.append(Some("Preferences"), Some("win.show-preferences"));
        app_section.append(Some("Help"), Some("win.show-help"));
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

    fn setup_actions(&self, viewer: &ViewerPane, state: Rc<RefCell<AppState>>) {
        let w = self.downgrade();
        let toggle_presentation_action = gio::SimpleAction::new("toggle-presentation", None);
        toggle_presentation_action.connect_activate(move |_, _| {
            if let Some(win) = w.upgrade() {
                win.toggle_presentation_mode();
            }
        });
        self.add_action(&toggle_presentation_action);

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

        let help_action = gio::SimpleAction::new("show-help", None);
        let weak = self.downgrade();
        help_action.connect_activate(move |_, _| {
            if let Some(window) = weak.upgrade() {
                crate::ui::help_window::show_help(&window);
            }
        });
        self.add_action(&help_action);

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

        // Keep the existing menu action and GTK's help key binding, but
        // present a sheet in this window so it inherits Sharpr's look.
        let shortcuts_action = gio::SimpleAction::new("show-help-overlay", None);
        let window_weak = self.downgrade();
        shortcuts_action.connect_activate(move |_, _| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let builder =
                gtk4::Builder::from_resource("/io/github/hebbihebb/Sharpr/help-overlay.ui");
            let dialog: libadwaita::ShortcutsDialog = builder.object("help_overlay").unwrap();
            dialog.present(Some(&window));
        });
        self.add_action(&shortcuts_action);

        let pref_action = gio::SimpleAction::new("show-preferences", None);
        {
            let window_weak = self.downgrade();
            let state_c = state.clone();
            pref_action.connect_activate(move |_, _| {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                let settings = state_c.borrow().settings.clone();
                let prefs = build_preferences_dialog(&settings, &window);
                prefs.present(Some(&window));
            });
        }
        self.add_action(&pref_action);
    }

    /// Build the viewer header bar.
    /// Returns `(header, sidebar_toggle, ops_indicator, preview_search_revealer, preview_search_entry)`.
    fn build_viewer_header(
        &self,
        menu_btn: &gtk4::MenuButton,
    ) -> (
        libadwaita::HeaderBar,
        gtk4::ToggleButton,
        OpsIndicator,
        gtk4::Revealer,    // preview_search_revealer
        gtk4::SearchEntry, // preview_search_entry
    ) {
        let header = libadwaita::HeaderBar::new();

        let sidebar_toggle = gtk4::ToggleButton::new();
        sidebar_toggle.set_icon_name("sidebar-show-symbolic");
        sidebar_toggle.set_tooltip_text(Some("Toggle Library"));
        sidebar_toggle.add_css_class("flat");
        header.pack_start(&sidebar_toggle);
        // Look prototype: search always in the middle of the viewer's bar.
        header.add_css_class("sharpr-top");

        let preview_search_entry = gtk4::SearchEntry::new();
        preview_search_entry.set_placeholder_text(Some("Describe images, or 4k png 16:9…"));
        preview_search_entry.set_width_request(360);
        // Always visible, so it must not take the window's first focus:
        // focusable only once search is wanted (a click, Ctrl+F, typing).
        preview_search_entry.set_can_focus(false);
        {
            let click = gtk4::GestureClick::new();
            click.set_propagation_phase(gtk4::PropagationPhase::Capture);
            let entry = preview_search_entry.downgrade();
            click.connect_pressed(move |_, _, _, _| {
                if let Some(entry) = entry.upgrade() {
                    entry.set_can_focus(true);
                    entry.grab_focus();
                }
            });
            preview_search_entry.add_controller(click);
        }

        let preview_search_revealer = gtk4::Revealer::new();
        preview_search_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideRight);
        let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let title_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        title_row.append(&preview_search_entry);
        title_row.append(&preview_search_revealer);
        header.set_title_widget(Some(&title_row));
        let all_library = gtk4::CheckButton::with_label("Whole library");
        *self.imp().search_whole_library.borrow_mut() = Some(all_library.clone());
        let weak = self.downgrade();
        all_library.connect_toggled(move |button| {
            if let Some(win) = weak.upgrade() {
                if win.imp().inline_search_suppressed.get() {
                    return;
                }
                win.imp().browse_options.borrow_mut().whole_library = button.is_active();
                let query = win
                    .imp()
                    .inline_search_entry
                    .borrow()
                    .as_ref()
                    .map(|e| e.text().to_string())
                    .unwrap_or_default();
                win.run_scoped_search(query);
            }
        });
        search_row.append(&all_library);
        preview_search_revealer.set_child(Some(&search_row));
        preview_search_revealer.set_reveal_child(false);

        header.pack_end(menu_btn);

        let ops_indicator = OpsIndicator::new();
        header.pack_end(&ops_indicator);

        (
            header,
            sidebar_toggle,
            ops_indicator,
            preview_search_revealer,
            preview_search_entry,
        )
    }
}
