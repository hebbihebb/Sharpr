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

use std::path::{Path, PathBuf};

use crate::config::{AppSettings, FolderMode};
use crate::duplicates::phash;
use crate::library_index::{normalize_collection_tag, BasicImageInfo, Collection, LibraryIndex};
use crate::model::library::{CachedImageData, RawImageEntry, SortOrder};
use crate::model::{ImageEntry, LibraryManager};
use crate::tags::smart::SmartModel;
use crate::thumbnails::worker::WorkerRequest;
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
    /// The single content scope currently loaded into the filmstrip.
    pub scope: ViewScope,
    /// Folders disabled by the user. Images under these paths must not be indexed or shown.
    pub disabled_folders: Vec<PathBuf>,
}

/// The single content scope currently loaded into the filmstrip.
/// Replaces the implicit encoding previously spread across
/// `library.current_folder` and `AppState::active_collection`.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ViewScope {
    Folder(PathBuf),
    Collection(i64),
    Duplicates,
    #[default]
    Search,
    Quality(crate::quality::QualityClass),
}

fn apply_scope_to_sidebar(scope: &ViewScope, sidebar: &SidebarPane) {
    match scope {
        ViewScope::Folder(path) => {
            sidebar.select_folder(path);
        }
        ViewScope::Collection(id) => {
            sidebar.set_collection_selected(*id);
        }
        ViewScope::Duplicates | ViewScope::Search | ViewScope::Quality(_) => {
            sidebar.clear_collection_selection();
        }
    }
}

fn parse_collection_tags_input(input: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for tag in input.split(',') {
        let tag = normalize_collection_tag(tag);
        if !tag.is_empty() && !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

fn effective_selected_paths(state: &AppState) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = state
        .selected_paths
        .iter()
        .filter(|path| state.library.entry_for_path(path).is_some())
        .cloned()
        .collect();
    if paths.is_empty() {
        if let Some(entry) = state.library.selected_entry() {
            paths.push(entry.path());
        }
    }
    paths.sort();
    paths
}

fn remove_path_from_action_selection(state: &mut AppState, path: &Path) {
    state.selected_paths.remove(path);
    state
        .selected_paths
        .retain(|selected| state.library.entry_for_path(selected).is_some());
}

const SWATCH_PALETTE: &[&str] = &[
    "#57e389", "#62a0ea", "#ff7800", "#f5c211", "#dc8add", "#5bc8af", "#e01b24", "#9141ac",
];

fn parse_hex_color(color: &str) -> Option<(f64, f64, f64)> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    ))
}

fn append_rounded_rect(
    cr: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    let right = x + width;
    let bottom = y + height;
    let degrees = std::f64::consts::PI / 180.0;

    cr.new_sub_path();
    cr.arc(
        right - radius,
        y + radius,
        radius,
        -90.0 * degrees,
        0.0 * degrees,
    );
    cr.arc(
        right - radius,
        bottom - radius,
        radius,
        0.0 * degrees,
        90.0 * degrees,
    );
    cr.arc(
        x + radius,
        bottom - radius,
        radius,
        90.0 * degrees,
        180.0 * degrees,
    );
    cr.arc(
        x + radius,
        y + radius,
        radius,
        180.0 * degrees,
        270.0 * degrees,
    );
    cr.close_path();
}

fn build_color_swatch_row(selected: Option<&str>) -> (gtk4::Widget, Rc<RefCell<Option<String>>>) {
    let selected_color = Rc::new(RefCell::new(selected.map(str::to_string)));
    let flowbox = gtk4::FlowBox::new();
    flowbox.set_max_children_per_line(8);
    flowbox.set_row_spacing(4);
    flowbox.set_column_spacing(4);
    flowbox.set_selection_mode(gtk4::SelectionMode::None);
    flowbox.set_halign(gtk4::Align::Start);

    for color in SWATCH_PALETTE {
        let button = gtk4::Button::new();
        button.add_css_class("flat");
        let swatch = gtk4::DrawingArea::new();
        swatch.set_content_width(20);
        swatch.set_content_height(20);

        let color_string = (*color).to_string();
        let selected_for_draw = selected_color.clone();
        swatch.set_draw_func(move |_, cr, _, _| {
            let (r, g, b) = parse_hex_color(&color_string)
                .unwrap_or_else(|| parse_hex_color("#57e389").unwrap());
            cr.set_source_rgb(r, g, b);
            append_rounded_rect(cr, 1.0, 1.0, 18.0, 18.0, 4.0);
            let _ = cr.fill();

            if selected_for_draw.borrow().as_deref() == Some(color_string.as_str()) {
                cr.set_source_rgb(1.0, 1.0, 1.0);
                cr.set_line_width(2.0);
                append_rounded_rect(cr, 3.0, 3.0, 14.0, 14.0, 3.0);
                let _ = cr.stroke();
            }
        });

        button.set_child(Some(&swatch));
        let selected_for_click = selected_color.clone();
        let flowbox_for_click = flowbox.clone();
        let color_for_click = (*color).to_string();
        button.connect_clicked(move |_| {
            *selected_for_click.borrow_mut() = Some(color_for_click.clone());
            flowbox_for_click.queue_draw();
        });
        flowbox.insert(&button, -1);
    }

    (flowbox.upcast(), selected_color)
}

fn search_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|term| term.trim().to_lowercase())
        .filter(|term| !term.is_empty())
        .collect()
}

fn show_new_collection_dialog<F>(
    window: gtk4::Window,
    initial_name: String,
    initial_extra_tags: String,
    state: Rc<RefCell<AppState>>,
    toast_overlay: libadwaita::ToastOverlay,
    refresh: F,
) where
    F: Fn() + Clone + 'static,
{
    let dialog = libadwaita::AlertDialog::new(Some("New Collection"), None);
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("create", libadwaita::ResponseAppearance::Suggested);
    let name_entry = gtk4::Entry::new();
    name_entry.set_placeholder_text(Some("Collection name"));
    name_entry.set_text(&initial_name);
    let (color_swatch_row, selected_color) = build_color_swatch_row(None);
    let tags_entry = gtk4::Entry::new();
    tags_entry.set_placeholder_text(Some("Extra tags, comma separated"));
    tags_entry.set_text(&initial_extra_tags);
    let info = gtk4::Label::new(Some("The collection name is also used as a tag."));
    info.add_css_class("dim-label");
    info.set_wrap(true);
    info.set_halign(gtk4::Align::Start);
    let entry_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    entry_box.set_margin_top(6);
    entry_box.append(&name_entry);
    entry_box.append(&color_swatch_row);
    entry_box.append(&info);
    entry_box.append(&tags_entry);
    dialog.set_extra_child(Some(&entry_box));
    let state_d = state.clone();
    let toast_d = toast_overlay.clone();
    let refresh_d = refresh.clone();
    let name_clone = name_entry.clone();
    let tags_clone = tags_entry.clone();
    let selected_color_clone = selected_color.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "create" {
            return;
        }
        let name = name_clone.text().to_string();
        let extra_tags = parse_collection_tags_input(tags_clone.text().as_str());
        let selected_color = selected_color_clone.borrow().clone();
        if let Some(idx) = state_d.borrow().library_index.clone() {
            let started = std::time::Instant::now();
            let library_id = state_d
                .borrow()
                .settings
                .active_library()
                .map(|l| l.id.clone())
                .unwrap_or_default();
            match idx.create_collection(
                &library_id,
                None,
                &name,
                &extra_tags,
                selected_color.as_deref(),
                None,
            ) {
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
    dialog.present(Some(&window));
}

fn show_new_library_dialog(
    window: gtk4::Window,
    state: Rc<RefCell<AppState>>,
    sidebar: SidebarPane,
    toast_overlay: libadwaita::ToastOverlay,
) {
    let dialog = libadwaita::AlertDialog::new(Some("Create Library"), None);
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("create", libadwaita::ResponseAppearance::Suggested);

    let name_entry = gtk4::Entry::new();
    name_entry.set_placeholder_text(Some("Library name"));
    let root_entry = gtk4::Entry::new();
    root_entry.set_editable(false);
    root_entry.set_hexpand(true);
    let choose_button = gtk4::Button::with_label("Choose…");
    let top_level = gtk4::CheckButton::with_label("Top level only");
    let drill_down = gtk4::CheckButton::with_label("Drill into subfolders");
    drill_down.set_group(Some(&top_level));
    top_level.set_active(true);
    let root_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    root_row.append(&root_entry);
    root_row.append(&choose_button);

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    box_.set_margin_top(6);
    box_.append(&name_entry);
    box_.append(&root_row);
    box_.append(&top_level);
    box_.append(&drill_down);
    dialog.set_extra_child(Some(&box_));

    {
        let root_entry_c = root_entry.clone();
        let window_c = window.clone();
        choose_button.connect_clicked(move |_| {
            let chooser = gtk4::FileDialog::new();
            chooser.set_title("Choose Library Root");
            let root_entry_inner = root_entry_c.clone();
            chooser.select_folder(Some(&window_c), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        root_entry_inner.set_text(&path.to_string_lossy());
                    }
                }
            });
        });
    }

    dialog.connect_response(None, move |_, response| {
        if response != "create" {
            return;
        }
        let root = PathBuf::from(root_entry.text().as_str());
        let folder_mode = if drill_down.is_active() {
            FolderMode::DrillDown
        } else {
            FolderMode::TopLevel
        };
        let created = {
            let mut st = state.borrow_mut();
            let result =
                st.settings
                    .add_library(name_entry.text().as_str(), root.clone(), folder_mode);
            match result {
                Ok(id) => {
                    st.settings.set_active_library(&id);
                    st.disabled_folders = st
                        .settings
                        .active_library()
                        .map(|library| library.ignored_folders.clone())
                        .unwrap_or_default();
                    Ok(())
                }
                Err(err) => Err(err),
            }
        };
        match created {
            Ok(()) => {
                sidebar.refresh_active_library(state.clone());
                toast_overlay.add_toast(libadwaita::Toast::new("Library created"));
            }
            Err(err) => toast_overlay.add_toast(libadwaita::Toast::new(&err)),
        }
    });
    dialog.present(Some(&window));
}

fn switch_active_library(library_id: &str, state: &Rc<RefCell<AppState>>, sidebar: &SidebarPane) {
    {
        let mut st = state.borrow_mut();
        st.settings.set_active_library(library_id);
        st.disabled_folders = st
            .settings
            .active_library()
            .map(|library| library.ignored_folders.clone())
            .unwrap_or_default();
        st.selected_paths.clear();
        st.scope = ViewScope::Search;
        st.library.load_virtual(&[]);
    }
    sidebar.imp().collapsed_folder_paths.borrow_mut().clear();
    sidebar.refresh_active_library(state.clone());
}

fn collection_paths_from_services(
    index: &LibraryIndex,
    tags: &crate::tags::TagDatabase,
    collection_id: i64,
    active_root: Option<&Path>,
    disabled_folders: &[PathBuf],
) -> Vec<PathBuf> {
    let effective_tags = index
        .collection_effective_tags(collection_id)
        .unwrap_or_default();
    if effective_tags.is_empty() {
        return Vec::new();
    }
    filter_paths_for_library(
        tags.paths_for_all_tags(&effective_tags),
        active_root,
        disabled_folders,
    )
}

fn collections_for_sidebar(state: &AppState) -> Vec<Collection> {
    let Some(index) = state.library_index.as_ref() else {
        return Vec::new();
    };
    let library_id = state
        .settings
        .active_library()
        .map(|lib| lib.id.clone())
        .unwrap_or_default();
    let active_root = state
        .settings
        .active_library()
        .map(|library| library.root.clone());
    let Some(tags) = state.tags.as_ref() else {
        return index
            .list_collections_for_library(Some(&library_id))
            .unwrap_or_default();
    };
    let mut collections = index
        .list_collections_for_library(Some(&library_id))
        .unwrap_or_default();
    for collection in &mut collections {
        collection.item_count = collection_paths_from_services(
            index,
            tags,
            collection.id,
            active_root.as_deref(),
            &state.disabled_folders,
        )
        .len();
    }
    collections
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

fn collection_local_tags(collection: &Collection) -> Vec<String> {
    let mut tags = vec![collection.primary_tag.clone()];
    for tag in &collection.extra_tags {
        if !tags.contains(tag) {
            tags.push(tag.clone());
        }
    }
    tags
}

fn apply_exact_tag_filter(
    state: &Rc<RefCell<AppState>>,
    filmstrip: &FilmstripPane,
    viewer: &ViewerPane,
    sidebar: &SidebarPane,
    selected_tags: &[String],
) {
    let paths = {
        let state_ref = state.borrow();
        let Some(tags_db) = state_ref.tags.as_ref() else {
            return;
        };
        let active_root = state_ref
            .settings
            .active_library()
            .map(|library| library.root.as_path());
        let paths = match selected_tags {
            [] => return,
            [single] => tags_db.paths_for_tag(single),
            many => tags_db.paths_for_all_tags(many),
        };
        filter_paths_for_library(paths, active_root, &state_ref.disabled_folders)
    };

    {
        let mut state_mut = state.borrow_mut();
        state_mut.selected_paths.clear();
        state_mut.scope = ViewScope::Search;
    }

    load_virtual_async(state, &paths);
    let scope = state.borrow().scope.clone();
    apply_scope_to_sidebar(&scope, sidebar);
    filmstrip.refresh_virtual();

    let first_path = state
        .borrow()
        .library
        .entry_at(0)
        .map(|entry: ImageEntry| entry.path());
    if let Some(path) = first_path {
        state.borrow_mut().library.selected_index = Some(0);
        filmstrip.navigate_to(0);
        viewer.load_image(path);
    } else {
        viewer.clear();
    }
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
    image: crate::image_pipeline::PreviewImage,
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
            let PrefetchResult { request, image } = result;
            {
                state_drain.borrow_mut().library.insert_prefetch(
                    request.path.clone(),
                    image.rgba,
                    image.width,
                    image.height,
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

fn path_in_active_library(path: &Path, active_root: Option<&Path>) -> bool {
    active_root
        .map(|root| path.starts_with(root))
        .unwrap_or(true)
}

fn filter_paths_for_library(
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
        if let Ok(image) = crate::image_pipeline::decode_preview(
            &request.path,
            crate::image_pipeline::PreviewDecodeMode::Prefetch,
        ) {
            let _ = tx.send_blocking(PrefetchResult { request, image });
        }
    });
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
            if let Ok((width, height)) = image::image_dimensions(&path) {
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

fn load_virtual_async(state: &Rc<RefCell<AppState>>, paths: &[PathBuf]) {
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

fn install_smart_tagger_model(
    model: SmartModel,
    model_path: &Path,
) -> Result<Arc<crate::tags::smart::LocalTagger>, String> {
    if model_path.exists() {
        return Ok(Arc::new(crate::tags::smart::LocalTagger::new(
            model_path.to_path_buf(),
        )));
    }

    let Some(dir) = model_path.parent() else {
        return Err("model path has no parent directory".into());
    };
    std::fs::create_dir_all(dir).map_err(|err| format!("create model directory failed: {err}"))?;
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

        let downloaded =
            std::fs::read(&tmp).map_err(|err| format!("read downloaded model failed: {err}"))?;
        let actual_hash = format!("{:x}", Sha256::digest(&downloaded));
        let expected_hash = model.sha256();
        if actual_hash != expected_hash {
            return Err(format!(
                "downloaded model hash mismatch: expected {expected_hash}, got {actual_hash}"
            ));
        }

        std::fs::rename(&tmp, model_path)
            .map_err(|err| format!("install downloaded model failed: {err}"))?;
        Ok(())
    })();

    if let Err(err) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }

    Ok(Arc::new(crate::tags::smart::LocalTagger::new(
        model_path.to_path_buf(),
    )))
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
        let disabled_folders = settings
            .active_library()
            .map(|library| library.ignored_folders.clone())
            .unwrap_or_default();
        let tags = crate::tags::TagDatabase::open().ok().map(Arc::new);
        if let (Some(index), Some(tags)) = (&library_index, &tags) {
            let _ = index.migrate_legacy_collections_to_tags(tags);
        }
        if let (Some(index), Some(lib)) = (&library_index, settings.active_library()) {
            let _ = index.assign_orphan_collections(&lib.id);
        }
        Self {
            library,
            settings,
            sort_order: SortOrder::default(),
            library_index,
            library_index_error,
            tags,
            smart_tagger,
            upscale_binary,
            ops,
            selected_paths: HashSet::new(),
            scope: ViewScope::default(),
            disabled_folders,
        }
    }
}

// ---------------------------------------------------------------------------
// GObject subclass
// ---------------------------------------------------------------------------

mod imp {
    use super::*;
    use crate::thumbnails::worker::{HashResult, SharpnessResult, ThumbnailResult};
    use async_channel::Receiver;

    pub struct SharprWindow {
        pub state: Rc<RefCell<AppState>>,
        pub viewer: RefCell<Option<ViewerPane>>,
        pub thumbnail_worker: RefCell<Option<ThumbnailWorker>>,
        pub preview_worker: RefCell<Option<crate::image_pipeline::worker::PreviewWorker>>,
        pub metadata_worker: RefCell<Option<crate::image_pipeline::worker::MetadataWorker>>,
        // Cloned receiver so the async task can hold it.
        pub result_rx: RefCell<Option<Receiver<ThumbnailResult>>>,
        pub hash_result_rx: RefCell<Option<Receiver<HashResult>>>,
        pub sharpness_result_rx: RefCell<Option<Receiver<SharpnessResult>>>,
        pub sharpness_backfill: RefCell<Option<crate::quality::backfill::SharpnessBackfill>>,
        pub(super) thumbnail_ops: RefCell<HashMap<PathBuf, ThumbnailOpState>>,
        pub toast_overlay: RefCell<Option<libadwaita::ToastOverlay>>,
        pub inline_search_entry: RefCell<Option<gtk4::SearchEntry>>,
        pub inline_search_revealer: RefCell<Option<gtk4::Revealer>>,
        pub inline_search_pending: RefCell<Option<glib::SourceId>>,
        pub inline_search_generation: Cell<u64>,
        pub inline_search_suppressed: Cell<bool>,
    }

    impl Default for SharprWindow {
        fn default() -> Self {
            let (ops_queue, _ops_rx) = crate::ops::queue::new_queue();
            Self {
                state: Rc::new(RefCell::new(AppState::new(ops_queue))),
                viewer: RefCell::new(None),
                thumbnail_worker: RefCell::new(None),
                preview_worker: RefCell::new(None),
                metadata_worker: RefCell::new(None),
                result_rx: RefCell::new(None),
                hash_result_rx: RefCell::new(None),
                sharpness_result_rx: RefCell::new(None),
                sharpness_backfill: RefCell::new(None),
                thumbnail_ops: RefCell::new(HashMap::new()),
                toast_overlay: RefCell::new(None),
                inline_search_entry: RefCell::new(None),
                inline_search_revealer: RefCell::new(None),
                inline_search_pending: RefCell::new(None),
                inline_search_generation: Cell::new(0),
                inline_search_suppressed: Cell::new(false),
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

        rayon::spawn(
            move || match install_smart_tagger_model(model, &model_path) {
                Ok(tagger) => {
                    let _ = tx.send_blocking(tagger);
                }
                Err(err) => {
                    eprintln!("Smart tagger model download aborted: {err}");
                }
            },
        );

        glib::MainContext::default().spawn_local(async move {
            if let Ok(tagger) = rx.recv().await {
                if let Some(window) = window_weak.upgrade() {
                    let configured_model = {
                        let state = window.app_state();
                        let state = state.borrow();
                        SmartModel::from_id(&state.settings.smart_tagger_model)
                    };
                    if configured_model != model {
                        return;
                    }
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
        let tags = self.imp().state.borrow().tags.clone();
        let (worker, result_rx, hash_result_rx, sharpness_result_rx) =
            ThumbnailWorker::spawn(thread_count, tags.clone());

        if let Some(db) = tags {
            let backfill =
                crate::quality::backfill::SharpnessBackfill::spawn(db, worker.sharpness_sender());
            *self.imp().sharpness_backfill.borrow_mut() = Some(backfill);
        }

        *self.imp().thumbnail_worker.borrow_mut() = Some(worker);
        *self.imp().result_rx.borrow_mut() = Some(result_rx);
        *self.imp().hash_result_rx.borrow_mut() = Some(hash_result_rx);
        *self.imp().sharpness_result_rx.borrow_mut() = Some(sharpness_result_rx);

        let (preview_worker, preview_result_rx) =
            crate::image_pipeline::worker::PreviewWorker::spawn();
        *self.imp().preview_worker.borrow_mut() = Some(preview_worker);

        let (metadata_worker, metadata_result_rx) =
            crate::image_pipeline::worker::MetadataWorker::spawn();
        *self.imp().metadata_worker.borrow_mut() = Some(metadata_worker);

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
                for dir_name in ["exported", "upscaled"] {
                    let output_dir = folder.join(dir_name);
                    if let Ok(entries) = std::fs::read_dir(&output_dir) {
                        for entry in entries.flatten() {
                            let name = entry.file_name().to_string_lossy().into_owned();
                            if name.contains(".pending-") || name.contains(".ncnn-intermediate") {
                                let _ = std::fs::remove_file(entry.path());
                            }
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
        } else {
            let model = SmartModel::from_id(&state.borrow().settings.smart_tagger_model);
            self.reload_smart_tagger_model(model);
        }

        if let Some(worker) = self.imp().thumbnail_worker.borrow().as_ref() {
            filmstrip.set_thumbnail_sender(
                worker.visible_sender(),
                worker.preload_sender(),
                worker.generation_arc(),
                worker.pending_set(),
            );
        }

        if let Some(tags) = state.borrow().tags.clone() {
            filmstrip.set_cached_tags(tags);
        }

        if let Some(worker) = self.imp().preview_worker.borrow().as_ref() {
            viewer.set_preview_worker(worker.handle(), preview_result_rx);
        }

        if let Some(worker) = self.imp().metadata_worker.borrow().as_ref() {
            viewer.set_metadata_worker(worker.handle(), metadata_result_rx);
        }

        {
            let filmstrip_ps = filmstrip.clone();
            viewer.set_post_save_callback(move || {
                filmstrip_ps.schedule_visible_thumbnails();
            });
        }

        let content_stack = gtk4::Stack::new();

        {
            let content_stack_c = content_stack.clone();
            viewer.connect_manage_tags(move || {
                content_stack_c.set_visible_child_name("tags");
            });
        }

        let toast_overlay = libadwaita::ToastOverlay::new();
        *self.imp().toast_overlay.borrow_mut() = Some(toast_overlay.clone());
        let tag_browser = state.borrow().tags.clone().map(TagBrowser::new);
        if let Some(tag_browser) = tag_browser.as_ref() {
            let collections = collections_for_sidebar(&state.borrow());
            tag_browser.set_collections(collections);
            if let Some(worker) = self.imp().thumbnail_worker.borrow().as_ref() {
                let state_c = state.clone();
                let visible_tx = worker.visible_sender();
                let pending_set = worker.pending_set();
                let generation = worker.generation_arc();
                tag_browser.set_preview_hooks(
                    move |path| state_c.borrow().library.cached_thumbnail(path),
                    move |path| {
                        if let Ok(pending) = pending_set.lock() {
                            if pending.contains(path) {
                                return;
                            }
                        }
                        let path = path.to_path_buf();
                        let should_enqueue = {
                            let Ok(mut pending) = pending_set.lock() else {
                                return;
                            };
                            pending.insert(path.clone())
                        };
                        if !should_enqueue {
                            return;
                        }
                        if visible_tx
                            .try_send(WorkerRequest::Thumbnail {
                                path: path.clone(),
                                gen: generation.load(std::sync::atomic::Ordering::Relaxed),
                            })
                            .is_err()
                        {
                            if let Ok(mut pending) = pending_set.lock() {
                                pending.remove(&path);
                            }
                        }
                    },
                );
            }
        }
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
                    win.clear_inline_search(true);
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
                    st.settings
                        .set_active_library_last_folder(Some(path.clone()));
                    st.selected_paths.clear();
                    st.scope = ViewScope::Folder(path.clone());
                }
                filmstrip_c.reset_quality_filter();

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
                    let mut raw_hydration_paths: Vec<PathBuf> = Vec::new();
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
                                    if let Some(texture) = st.library.cached_thumbnail(&raw.path) {
                                        entry.set_thumbnail(Some(texture));
                                    }
                                    st.library
                                        .path_to_index
                                        .insert(raw.path.clone(), index as u32);
                                    raw_hydration_paths.push(raw.path.clone());
                                    st.library.all_known_paths.insert(raw.path);
                                    new_entries.push(entry);
                                }
                                let entry_count = new_entries.len();
                                st.library.store.splice(0, 0, &new_entries);
                                crate::bench_event!(
                                    "folder.raw_scan.fast_populate",
                                    serde_json::json!({
                                        "path": path_rx.display().to_string(),
                                        "entry_count": entry_count,
                                    }),
                                );
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

                    let scope = state_rx.borrow().scope.clone();
                    apply_scope_to_sidebar(&scope, &sidebar_rx);

                    viewer_rx.clear();
                    filmstrip_rx.refresh();

                    // Queue unscored images into the idle sharpness backfill.
                    if let Some(win) = window_weak_rx.upgrade() {
                        if let Some(backfill) = win.imp().sharpness_backfill.borrow().as_ref() {
                            let paths: Vec<_> = {
                                let st = state_rx.borrow();
                                (0..st.library.image_count())
                                    .filter_map(|i| st.library.entry_at(i).map(|e| e.path()))
                                    .collect()
                            };
                            backfill.enqueue(paths);
                        }
                    }

                    let thumb_total = state_rx.borrow().library.image_count();
                    crate::bench_event!(
                        "folder.open.ready",
                        serde_json::json!({
                            "path": path_rx.display().to_string(),
                            "image_count": thumb_total,
                        }),
                    );
                    let (preview_bytes, prefetch_bytes) = state_rx.borrow().library.cache_stats();
                    crate::bench_event!(
                        "viewer.cache.stats",
                        serde_json::json!({
                            "preview_bytes": preview_bytes,
                            "prefetch_bytes": prefetch_bytes,
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

                    start_raw_dimension_hydrator(
                        raw_hydration_paths,
                        path_rx.clone(),
                        state_rx.clone(),
                    );
                });
            })
        };

        // Helper: refresh the sidebar collection list from the DB.
        let refresh_sidebar_collections = {
            let sidebar_c = sidebar.clone();
            let filmstrip_c = filmstrip.clone();
            let state_c = state.clone();
            let tag_browser_c = tag_browser.clone();
            move || {
                let (collections, scope) = {
                    let state = state_c.borrow();
                    (collections_for_sidebar(&state), state.scope.clone())
                };
                filmstrip_c.refresh_collection_colors(&collections);
                sidebar_c.refresh_collections(&collections);
                if let Some(tag_browser) = tag_browser_c.as_ref() {
                    tag_browser.set_collections(collections.clone());
                }
                apply_scope_to_sidebar(&scope, &sidebar_c);
            }
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
            let refresh_sidebar_collections_c = refresh_sidebar_collections.clone();
            sidebar.connect_library_selected(move |library_id| {
                switch_active_library(&library_id, &state_c, &sidebar_c);
                refresh_sidebar_collections_c();
            });
        }

        {
            let state_c = state.clone();
            let sidebar_c = sidebar.clone();
            let toast_overlay_c = toast_overlay.clone();
            let window = self.clone().upcast::<gtk4::Window>();
            sidebar.connect_library_create_requested(move || {
                show_new_library_dialog(
                    window.clone(),
                    state_c.clone(),
                    sidebar_c.clone(),
                    toast_overlay_c.clone(),
                );
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
                        st.library.load_virtual(&[]);
                        st.scope = ViewScope::Search;
                    } else if !matches!(st.scope, ViewScope::Folder(_)) {
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
            let content_stack = content_stack.clone();
            let toast_overlay_c = toast_overlay.clone();
            let window_weak = self.downgrade();
            let dupe_action = gio::SimpleAction::new("find-duplicates", None);
            dupe_action.connect_activate(move |_, _| {
                let expected_gen = window_weak.upgrade().and_then(|win| {
                    let gen = win.bump_thumbnail_generation("smart.duplicates");
                    win.complete_thumbnail_ops();
                    win.clear_inline_search(true);
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
                    filmstrip_rx.reset_quality_filter();
                    state_rx.borrow_mut().scope = ViewScope::Duplicates;
                    load_virtual_async(&state_rx, &paths);
                    crate::bench_event!(
                        "virtual_view.load",
                        serde_json::json!({
                            "source": "duplicates",
                            "image_count": result_count,
                        }),
                    );
                    let scope = state_rx.borrow().scope.clone();
                    apply_scope_to_sidebar(&scope, &sidebar_rx);
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
                    op.complete();
                });
            });
            self.add_action(&dupe_action);
        }

        if let Some(tag_browser) = tag_browser.clone() {
            let content_stack_c = content_stack.clone();
            let tags_action = gio::SimpleAction::new("show-tags", None);
            tags_action.connect_activate(move |_, _| {
                content_stack_c.set_visible_child_name("tags");
                tag_browser.refresh();
            });
            self.add_action(&tags_action);
        }

        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let content_stack = content_stack.clone();
            let window_weak = self.downgrade();
            let quality_action =
                gio::SimpleAction::new("scan-quality", Some(glib::VariantTy::STRING));
            quality_action.connect_activate(move |_, param| {
                let class = match param.and_then(|p| p.str()) {
                    Some("Excellent") => crate::quality::QualityClass::Excellent,
                    Some("Good") => crate::quality::QualityClass::Good,
                    Some("Fair") => crate::quality::QualityClass::Fair,
                    Some("Poor") => crate::quality::QualityClass::Poor,
                    Some("Needs Upscale") => crate::quality::QualityClass::NeedsUpscale,
                    _ => return,
                };
                let expected_gen = window_weak.upgrade().and_then(|win| {
                    let gen = win.bump_thumbnail_generation("smart.quality");
                    win.complete_thumbnail_ops();
                    win.clear_inline_search(true);
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
                let active_library = state_c.borrow().settings.active_library().cloned();
                let library_root = active_library.as_ref().map(|library| library.root.clone());
                let folder_mode = active_library
                    .as_ref()
                    .map(|library| library.folder_mode)
                    .unwrap_or(FolderMode::TopLevel);
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
                                let paths = filter_paths_for_library(
                                    paths,
                                    library_root.as_deref(),
                                    &disabled_folders,
                                );
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
                            folder_mode,
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
                    filmstrip_rx.reset_quality_filter();
                    state_rx.borrow_mut().scope = ViewScope::Quality(class);
                    load_virtual_async(&state_rx, &paths);
                    crate::bench_event!(
                        "virtual_view.load",
                        serde_json::json!({
                            "source": "quality",
                            "class": class.label(),
                            "image_count": result_count,
                        }),
                    );
                    let scope = state_rx.borrow().scope.clone();
                    apply_scope_to_sidebar(&scope, &sidebar_rx);
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
                    op.complete();
                });
            });
            self.add_action(&quality_action);
        }

        // Populate sidebar collections on startup.
        refresh_sidebar_collections();

        // "New Collection" + button → create root collection metadata.
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
                    String::new(),
                    state_c.clone(),
                    toast_overlay_c.clone(),
                    refresh_c.clone(),
                );
            });
        }

        if let Some(tag_browser) = tag_browser.as_ref() {
            let win_w = self.downgrade();
            let state_c = state.clone();
            let toast_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            tag_browser.connect_tag_create_collection_requested(move |tag| {
                let Some(win) = win_w.upgrade() else {
                    return;
                };
                show_new_collection_dialog(
                    win.upcast(),
                    tag.to_string(),
                    String::new(),
                    state_c.clone(),
                    toast_c.clone(),
                    refresh_c.clone(),
                );
            });
        }

        {
            let state_c = state.clone();
            let toast_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            sidebar.connect_tag_promoted_to_collection(move |tag| {
                let Some(idx) = state_c.borrow().library_index.clone() else {
                    return;
                };
                let library_id = state_c
                    .borrow()
                    .settings
                    .active_library()
                    .map(|l| l.id.clone())
                    .unwrap_or_default();
                match idx.create_collection(&library_id, None, &tag, &[], None, None) {
                    Ok(coll) => {
                        refresh_c();
                        toast_c.add_toast(libadwaita::Toast::new(&format!(
                            "Collection \u{201c}{}\u{201d} created",
                            coll.name
                        )));
                    }
                    Err(e) => {
                        toast_c.add_toast(libadwaita::Toast::new(&format!(
                            "Could not create collection: {e}"
                        )));
                    }
                }
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
                    win.clear_inline_search(true);
                }
                content_stack.set_visible_child_name("viewer");
                let paths = {
                    let state = state_c.borrow();
                    let active_root = state.settings.active_library().map(|lib| lib.root.clone());
                    match (state.library_index.as_ref(), state.tags.as_ref()) {
                        (Some(idx), Some(tags)) => collection_paths_from_services(
                            idx,
                            tags,
                            id,
                            active_root.as_deref(),
                            &state.disabled_folders,
                        ),
                        _ => Vec::new(),
                    }
                };
                let started = std::time::Instant::now();
                {
                    let mut s = state_c.borrow_mut();
                    s.selected_paths.clear();
                }
                filmstrip_c.reset_quality_filter();
                state_c.borrow_mut().scope = ViewScope::Collection(id);
                load_virtual_async(&state_c, &paths);
                crate::bench_event!(
                    "collection.load",
                    serde_json::json!({
                        "collection_id": id,
                        "path_count": paths.len(),
                        "duration_ms": crate::bench::duration_ms(started),
                    }),
                );
                let scope = state_c.borrow().scope.clone();
                apply_scope_to_sidebar(&scope, &sidebar_c);
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

        // Right-click → create child collection.
        {
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            let window_weak = self.downgrade();
            sidebar.connect_collection_child_add_requested(move |parent_id| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                let dialog = libadwaita::AlertDialog::new(Some("New Child Collection"), None);
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("create", "Create");
                dialog.set_default_response(Some("create"));
                dialog.set_close_response("cancel");
                dialog.set_response_appearance("create", libadwaita::ResponseAppearance::Suggested);
                let name_entry = gtk4::Entry::new();
                name_entry.set_placeholder_text(Some("Collection name"));
                let tags_entry = gtk4::Entry::new();
                tags_entry.set_placeholder_text(Some("Extra tags, comma separated"));
                let info = gtk4::Label::new(Some("The collection name is also used as a tag."));
                info.add_css_class("dim-label");
                info.set_wrap(true);
                info.set_halign(gtk4::Align::Start);
                let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
                box_.set_margin_top(6);
                box_.append(&name_entry);
                box_.append(&info);
                box_.append(&tags_entry);
                dialog.set_extra_child(Some(&box_));
                let state_d = state_c.clone();
                let toast_d = toast_overlay_c.clone();
                let refresh_d = refresh_c.clone();
                let name_clone = name_entry.clone();
                let tags_clone = tags_entry.clone();
                dialog.connect_response(None, move |_, response| {
                    if response != "create" {
                        return;
                    }
                    if let Some(idx) = state_d.borrow().library_index.clone() {
                        let started = std::time::Instant::now();
                        let library_id = state_d
                            .borrow()
                            .settings
                            .active_library()
                            .map(|l| l.id.clone())
                            .unwrap_or_default();
                        match idx.create_collection(
                            &library_id,
                            Some(parent_id),
                            name_clone.text().as_str(),
                            &parse_collection_tags_input(tags_clone.text().as_str()),
                            None,
                            None,
                        ) {
                            Ok(coll) => {
                                crate::bench_event!(
                                    "collection.create",
                                    serde_json::json!({
                                        "collection_id": coll.id,
                                        "parent_id": parent_id,
                                        "name": coll.name,
                                        "duration_ms": crate::bench::duration_ms(started),
                                    }),
                                );
                                refresh_d();
                                toast_d.add_toast(libadwaita::Toast::new(&format!(
                                    "Child collection \u{201c}{}\u{201d} created",
                                    coll.name
                                )));
                            }
                            Err(e) => {
                                toast_d.add_toast(libadwaita::Toast::new(&format!(
                                    "Could not create child collection: {e}"
                                )));
                            }
                        }
                    }
                });
                dialog.present(Some(&win));
            });
        }

        // Right-click → rename/edit collection metadata and retag current scope.
        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            let window_weak = self.downgrade();
            sidebar.connect_collection_edit_requested(move |id| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                let Some(idx) = state_c.borrow().library_index.clone() else {
                    return;
                };
                let Some(collection) = idx.collection(id).ok().flatten() else {
                    return;
                };
                let dialog = libadwaita::AlertDialog::new(Some("Edit Collection"), None);
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("save", "Save");
                dialog.set_default_response(Some("save"));
                dialog.set_close_response("cancel");
                dialog.set_response_appearance("save", libadwaita::ResponseAppearance::Suggested);
                let name_entry = gtk4::Entry::new();
                name_entry.set_text(&collection.name);
                name_entry.select_region(0, -1);
                let (color_swatch_row, selected_color) =
                    build_color_swatch_row(collection.color.as_deref());
                let tags_entry = gtk4::Entry::new();
                tags_entry.set_text(&collection.extra_tags.join(", "));
                let info = gtk4::Label::new(Some("The collection name is also used as a tag."));
                info.add_css_class("dim-label");
                info.set_wrap(true);
                info.set_halign(gtk4::Align::Start);
                let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
                box_.set_margin_top(6);
                box_.append(&name_entry);
                box_.append(&color_swatch_row);
                box_.append(&info);
                box_.append(&tags_entry);
                dialog.set_extra_child(Some(&box_));
                let state_d = state_c.clone();
                let toast_d = toast_overlay_c.clone();
                let refresh_d = refresh_c.clone();
                let filmstrip_d = filmstrip_c.clone();
                let viewer_d = viewer_c.clone();
                let sidebar_d = sidebar_c.clone();
                let name_clone = name_entry.clone();
                let tags_clone = tags_entry.clone();
                let selected_color_clone = selected_color.clone();
                let icon_name = collection.icon_name.clone();
                dialog.connect_response(None, move |_, response| {
                    if response != "save" {
                        return;
                    }
                    let (Some(idx), Some(tags_db)) = (
                        state_d.borrow().library_index.clone(),
                        state_d.borrow().tags.clone(),
                    ) else {
                        return;
                    };
                    let Some(before) = idx.collection(id).ok().flatten() else {
                        return;
                    };
                    let new_name = name_clone.text().to_string();
                    let new_extra_tags = parse_collection_tags_input(tags_clone.text().as_str());
                    let selected_color = selected_color_clone.borrow().clone();
                    let (active_root_buf, disabled) = {
                        let s = state_d.borrow();
                        (s.settings.active_library().map(|lib| lib.root.clone()), s.disabled_folders.clone())
                    };
                    let old_scope_paths = collection_paths_from_services(&idx, &tags_db, id, active_root_buf.as_deref(), &disabled);
                    let old_primary = before.primary_tag.clone();
                    let new_primary = normalize_collection_tag(&new_name);
                    let added_extra_tags: Vec<String> = new_extra_tags
                        .iter()
                        .filter(|tag| !before.extra_tags.contains(*tag))
                        .cloned()
                        .collect();
                    let started = std::time::Instant::now();
                    match idx.update_collection(
                        id,
                        &new_name,
                        &new_extra_tags,
                        selected_color.as_deref(),
                        icon_name.as_deref(),
                    ) {
                        Ok(()) => {
                            if !added_extra_tags.is_empty() {
                                tags_db.add_tags_to_paths(&old_scope_paths, &added_extra_tags);
                            }
                            if old_primary != new_primary {
                                tags_db.replace_tag_in_paths(&old_scope_paths, &old_primary, &new_primary);
                            }
                            crate::bench_event!(
                                "collection.update",
                                serde_json::json!({
                                    "collection_id": id,
                                    "duration_ms": crate::bench::duration_ms(started),
                                }),
                            );
                            refresh_d();
                            if matches!(state_d.borrow().scope, ViewScope::Collection(active) if active == id) {
                                let (active_root_buf, disabled) = {
                                    let s = state_d.borrow();
                                    (s.settings.active_library().map(|lib| lib.root.clone()), s.disabled_folders.clone())
                                };
                                let paths = collection_paths_from_services(&idx, &tags_db, id, active_root_buf.as_deref(), &disabled);
                                state_d.borrow_mut().scope = ViewScope::Collection(id);
                                load_virtual_async(&state_d, &paths);
                                filmstrip_d.refresh_virtual();
                                let scope = state_d.borrow().scope.clone();
                                apply_scope_to_sidebar(&scope, &sidebar_d);
                                let first = state_d
                                    .borrow()
                                    .library
                                    .entry_at(0)
                                    .map(|e: ImageEntry| e.path());
                                if let Some(path) = first {
                                    state_d.borrow_mut().library.selected_index = Some(0);
                                    filmstrip_d.navigate_to(0);
                                    viewer_d.load_image(path);
                                } else {
                                    viewer_d.clear();
                                }
                            }
                        }
                        Err(e) => {
                            toast_d.add_toast(libadwaita::Toast::new(&format!(
                                "Could not update collection: {e}"
                            )));
                        }
                    }
                });
                dialog.present(Some(&win));
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
                            let was_active = matches!(state_c.borrow().scope, ViewScope::Collection(active) if idx.collection(active).unwrap_or(None).is_none());
                            if was_active {
                                let mut s = state_c.borrow_mut();
                                s.scope = ViewScope::Search;
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

        // Reparent a leaf collection under another collection.
        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            let reparent_collection = Rc::new(move |source_id: i64, target_parent_id: i64| {
                let (Some(idx), Some(tags_db)) = (
                    state_c.borrow().library_index.clone(),
                    state_c.borrow().tags.clone(),
                ) else {
                    return;
                };
                let Some(before) = idx.collection(source_id).ok().flatten() else {
                    return;
                };
                let old_effective_tags =
                    idx.collection_effective_tags(source_id).unwrap_or_default();
                let local_tags = collection_local_tags(&before);
                let (active_root_buf, disabled) = {
                    let s = state_c.borrow();
                    (
                        s.settings.active_library().map(|lib| lib.root.clone()),
                        s.disabled_folders.clone(),
                    )
                };
                let old_paths = collection_paths_from_services(
                    &idx,
                    &tags_db,
                    source_id,
                    active_root_buf.as_deref(),
                    &disabled,
                );
                let started = std::time::Instant::now();
                match idx.reparent_collection(source_id, target_parent_id) {
                    Ok(()) => {
                        let new_effective_tags =
                            idx.collection_effective_tags(source_id).unwrap_or_default();
                        let old_ancestor_tags: Vec<String> = old_effective_tags
                            .iter()
                            .filter(|tag| !local_tags.contains(*tag))
                            .cloned()
                            .collect();
                        let new_ancestor_tags: Vec<String> = new_effective_tags
                            .iter()
                            .filter(|tag| !local_tags.contains(*tag))
                            .cloned()
                            .collect();
                        let tags_to_remove: Vec<String> = old_ancestor_tags
                            .iter()
                            .filter(|tag| !new_ancestor_tags.contains(*tag))
                            .cloned()
                            .collect();
                        let tags_to_add: Vec<String> = new_ancestor_tags
                            .iter()
                            .filter(|tag| !old_ancestor_tags.contains(*tag))
                            .cloned()
                            .collect();
                        if !tags_to_remove.is_empty() {
                            tags_db.remove_tags_from_paths(&old_paths, &tags_to_remove);
                        }
                        if !tags_to_add.is_empty() {
                            tags_db.add_tags_to_paths(&old_paths, &tags_to_add);
                        }
                        crate::bench_event!(
                            "collection.reparent",
                            serde_json::json!({
                                "collection_id": source_id,
                                "target_parent_id": target_parent_id,
                                "path_count": old_paths.len(),
                                "duration_ms": crate::bench::duration_ms(started),
                            }),
                        );
                        refresh_c();
                        if let ViewScope::Collection(active_id) = state_c.borrow().scope.clone() {
                            let (active_root_buf2, disabled2) = {
                                let s = state_c.borrow();
                                (
                                    s.settings.active_library().map(|lib| lib.root.clone()),
                                    s.disabled_folders.clone(),
                                )
                            };
                            let paths = collection_paths_from_services(
                                &idx,
                                &tags_db,
                                active_id,
                                active_root_buf2.as_deref(),
                                &disabled2,
                            );
                            state_c.borrow_mut().scope = ViewScope::Collection(active_id);
                            load_virtual_async(&state_c, &paths);
                            filmstrip_c.refresh_virtual();
                            let scope = state_c.borrow().scope.clone();
                            apply_scope_to_sidebar(&scope, &sidebar_c);
                            let first = state_c
                                .borrow()
                                .library
                                .entry_at(0)
                                .map(|e: ImageEntry| e.path());
                            if let Some(path) = first {
                                state_c.borrow_mut().library.selected_index = Some(0);
                                filmstrip_c.navigate_to(0);
                                viewer_c.load_image(path);
                            } else {
                                viewer_c.clear();
                            }
                        }
                        let moved_name = idx
                            .collection(source_id)
                            .ok()
                            .flatten()
                            .map(|c| c.name)
                            .unwrap_or_else(|| "collection".to_string());
                        let parent_name = idx
                            .collection(target_parent_id)
                            .ok()
                            .flatten()
                            .map(|c| c.name)
                            .unwrap_or_else(|| "collection".to_string());
                        toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                            "\u{201c}{}\u{201d} is now a child of \u{201c}{}\u{201d}",
                            moved_name, parent_name
                        )));
                    }
                    Err(e) => {
                        toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                            "Could not move collection: {e}"
                        )));
                    }
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
                let options: Vec<(i64, String)> = flatten_collection_paths(&collections)
                    .into_iter()
                    .filter(|(id, _)| *id != source_id)
                    .collect();
                if options.is_empty() {
                    toast_move.add_toast(libadwaita::Toast::new(
                        "No target collection is available for reparenting",
                    ));
                    return;
                }

                let dialog = libadwaita::AlertDialog::new(Some("Assign As Child Of"), None);
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
                    reparent(source_id, target_id);
                });
                dialog.present(Some(&win));
            });

            let reparent_from_drop = reparent_collection.clone();
            sidebar.connect_collection_reparent_requested(move |source_id, target_parent_id| {
                reparent_from_drop(source_id, target_parent_id);
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
                let (Some(idx), Some(tags_db)) = (
                    state_c.borrow().library_index.clone(),
                    state_c.borrow().tags.clone(),
                ) else {
                    return;
                };
                let started = std::time::Instant::now();
                match idx.collection_effective_tags(id) {
                    Ok(effective_tags) => {
                        let added = tags_db.add_tags_to_paths(&paths, &effective_tags);
                        let _ = idx.touch_collection(id);
                        let name = idx
                            .collection(id)
                            .ok()
                            .flatten()
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
                        if matches!(state_c.borrow().scope, ViewScope::Collection(a) if a == id) {
                            let (active_root_buf, disabled) = {
                                let s = state_c.borrow();
                                (
                                    s.settings.active_library().map(|lib| lib.root.clone()),
                                    s.disabled_folders.clone(),
                                )
                            };
                            let all_paths = collection_paths_from_services(
                                &idx,
                                &tags_db,
                                id,
                                active_root_buf.as_deref(),
                                &disabled,
                            );
                            state_c.borrow_mut().scope = ViewScope::Collection(id);
                            load_virtual_async(&state_c, &all_paths);
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
                            "Tagged {} image{} for \u{201c}{}\u{201d}",
                            paths.len(),
                            if paths.len() == 1 { "" } else { "s" },
                            name
                        )));
                    }
                    Err(e) => {
                        toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                            "Could not tag images for collection: {e}"
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
        self.start_thumbnail_poll(state.clone(), filmstrip.clone(), tag_browser.clone());
        self.start_hash_poll(state.clone());
        self.start_sharpness_poll(state.clone(), viewer.clone());

        // Quality filter in the filmstrip sort dropdown: re-derive paths from the
        // current scope intersected with the chosen quality class.
        {
            let state_c = state.clone();
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let window_weak = self.downgrade();
            filmstrip.connect_quality_filter_changed(move |quality_class| {
                let (scope, library_index, sort_order, indexed_paths, metadata_cache) = {
                    let s = state_c.borrow();
                    let (indexed_paths, _, metadata_cache) = s.library.bg_scan_quality_prep();
                    (
                        s.scope.clone(),
                        s.library_index.clone(),
                        s.sort_order,
                        indexed_paths,
                        metadata_cache,
                    )
                };

                let base_paths: Vec<PathBuf> = match &scope {
                    ViewScope::Folder(path) => library_index
                        .as_ref()
                        .and_then(|idx| idx.images_in_folder(path, sort_order).ok())
                        .map(|rows| rows.into_iter().map(|r| r.path).collect())
                        .unwrap_or_default(),
                    ViewScope::Collection(id) => library_index
                        .as_ref()
                        .and_then(|idx| {
                            let state_ref = state_c.borrow();
                            let active_root = state_ref
                                .settings
                                .active_library()
                                .map(|library| library.root.as_path());
                            state_ref.tags.as_ref().map(|tags| {
                                collection_paths_from_services(
                                    idx,
                                    tags,
                                    *id,
                                    active_root,
                                    &state_ref.disabled_folders,
                                )
                            })
                        })
                        .unwrap_or_default(),
                    _ => {
                        let s = state_c.borrow();
                        (0..s.library.image_count())
                            .filter_map(|i| s.library.entry_at(i).map(|e: ImageEntry| e.path()))
                            .collect()
                    }
                };

                if base_paths.is_empty() {
                    return;
                }

                type ScanState = (
                    Vec<PathBuf>,
                    rustc_hash::FxHashMap<PathBuf, crate::model::library::CachedImageData>,
                );
                let (tx, rx) = async_channel::bounded::<(Vec<PathBuf>, Option<ScanState>)>(1);
                rayon::spawn(move || {
                    let result = if let Some(class) = quality_class {
                        let (new_indexed, new_cache, filtered) =
                            crate::model::library::LibraryManager::filter_paths_by_quality_class(
                                class,
                                base_paths,
                                indexed_paths,
                                metadata_cache,
                            );
                        (filtered, Some((new_indexed, new_cache)))
                    } else {
                        (base_paths, None)
                    };
                    let _ = tx.send_blocking(result);
                });

                let filmstrip_rx = filmstrip_c.clone();
                let viewer_rx = viewer_c.clone();
                let state_rx = state_c.clone();
                let window_weak_rx = window_weak.clone();
                glib::MainContext::default().spawn_local(async move {
                    let Ok((paths, scan_state)) = rx.recv().await else {
                        return;
                    };
                    if let Some(win) = window_weak_rx.upgrade() {
                        let _ = win.bump_thumbnail_generation("filter.apply");
                        win.complete_thumbnail_ops();
                    }
                    if let Some((new_indexed, new_cache)) = scan_state {
                        state_rx
                            .borrow_mut()
                            .library
                            .bg_scan_quality_finish(new_indexed, new_cache);
                    }
                    load_virtual_async(&state_rx, &paths);
                    filmstrip_rx.refresh_virtual();
                    let first = state_rx
                        .borrow()
                        .library
                        .entry_at(0)
                        .map(|e: ImageEntry| e.path());
                    if let Some(p) = first {
                        state_rx.borrow_mut().library.selected_index = Some(0);
                        filmstrip_rx.navigate_to(0);
                        viewer_rx.load_image(p);
                    } else {
                        viewer_rx.clear();
                    }
                });
            });
        }

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
            commit_menu_btn,
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

        let preview_search_entry = gtk4::SearchEntry::new();
        preview_search_entry.set_hexpand(true);
        preview_search_entry.set_placeholder_text(Some("Search filenames and tags"));
        preview_search_entry.set_margin_start(12);
        preview_search_entry.set_margin_end(12);
        preview_search_entry.set_margin_top(6);
        preview_search_entry.set_margin_bottom(6);

        let preview_search_revealer = gtk4::Revealer::new();
        preview_search_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
        preview_search_revealer.set_reveal_child(false);
        preview_search_revealer.set_child(Some(&preview_search_entry));
        viewer_toolbar.add_top_bar(&preview_search_revealer);
        *self.imp().inline_search_entry.borrow_mut() = Some(preview_search_entry.clone());
        *self.imp().inline_search_revealer.borrow_mut() = Some(preview_search_revealer.clone());

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
        viewer.set_comparison_buttons(
            commit_btn.clone(),
            commit_menu_btn.clone(),
            discard_btn.clone(),
        );
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
                viewer_c.commit_convert_default();
            });
        }
        {
            let viewer_c = viewer.clone();
            discard_btn.connect_clicked(move |_| {
                viewer_c.discard_convert();
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
            let preview_search_revealer_c = preview_search_revealer.clone();
            content_stack.connect_visible_child_notify(move |stack| {
                let name = stack.visible_child_name().unwrap_or_default();
                let is_tags = name == "tags";
                viewer_page_c.set_title(if is_tags { "Tags" } else { "Preview" });
                preview_title_btn_c.set_label(if is_tags { "Tags" } else { "Preview" });
                preview_title_btn_c.set_sensitive(!is_tags);
                if is_tags {
                    preview_search_revealer_c.set_reveal_child(false);
                }
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
        viewer.set_ops_indicator(ops_indicator.clone());

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

        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            let sidebar_c = sidebar.clone();
            let content_stack_c = content_stack.clone();
            let window_weak = self.downgrade();
            preview_search_entry.connect_search_changed(move |entry| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                if win.imp().inline_search_suppressed.get() {
                    return;
                }
                if let Some(source_id) = win.imp().inline_search_pending.borrow_mut().take() {
                    source_id.remove();
                }
                let search_gen = win.imp().inline_search_generation.get().wrapping_add(1);
                win.imp().inline_search_generation.set(search_gen);

                let query = entry.text().trim().to_string();
                content_stack_c.set_visible_child_name("viewer");
                if query.is_empty() {
                    let Ok(mut state) = state_c.try_borrow_mut() else {
                        return;
                    };
                    state.scope = ViewScope::Search;
                    drop(state);
                    load_virtual_async(&state_c, &[]);
                    let Ok(state) = state_c.try_borrow() else {
                        return;
                    };
                    let scope = state.scope.clone();
                    drop(state);
                    apply_scope_to_sidebar(&scope, &sidebar_c);
                    viewer_c.clear();
                    filmstrip_c.refresh_virtual();
                    return;
                }

                if query.len() < 2 {
                    return;
                }

                let filmstrip_rx = filmstrip_c.clone();
                let viewer_rx = viewer_c.clone();
                let state_rx = state_c.clone();
                let sidebar_rx = sidebar_c.clone();
                let content_stack_rx = content_stack_c.clone();
                let window_weak_rx = window_weak.clone();
                let source_id =
                    glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
                        if let Some(win) = window_weak_rx.upgrade() {
                            win.imp().inline_search_pending.borrow_mut().take();
                        }
                        let db_query = query.clone();
                        let ui_query = query.clone();
                        let (tx, rx) = async_channel::bounded::<Vec<PathBuf>>(1);
                        let tags = state_rx
                            .try_borrow()
                            .ok()
                            .and_then(|state| state.tags.clone());
                        rayon::spawn(move || {
                            let paths = tags
                                .as_ref()
                                .map(|db| db.search_paths(&db_query))
                                .unwrap_or_default();
                            let _ = tx.send_blocking(paths);
                        });

                        let filmstrip_ui = filmstrip_rx.clone();
                        let viewer_ui = viewer_rx.clone();
                        let state_ui = state_rx.clone();
                        let sidebar_ui = sidebar_rx.clone();
                        let content_stack_ui = content_stack_rx.clone();
                        let window_weak_ui = window_weak_rx.clone();
                        glib::MainContext::default().spawn_local(async move {
                            if let Ok(db_paths) = rx.recv().await {
                                let Some(win) = window_weak_ui.upgrade() else {
                                    return;
                                };
                                if win.imp().inline_search_generation.get() != search_gen {
                                    return;
                                }
                                let terms = search_terms(&ui_query);
                                let library_paths: Vec<PathBuf> = {
                                    let Ok(state) = state_ui.try_borrow() else {
                                        return;
                                    };
                                    (0..state.library.image_count())
                                        .filter_map(|i| state.library.entry_at(i))
                                        .map(|e| e.path())
                                        .filter(|p| {
                                            p.file_name()
                                                .and_then(|n| n.to_str())
                                                .map(|n| {
                                                    let name = n.to_lowercase();
                                                    terms.iter().all(|term| name.contains(term))
                                                })
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

                                let Ok(mut state) = state_ui.try_borrow_mut() else {
                                    return;
                                };
                                state.scope = ViewScope::Search;
                                drop(state);
                                load_virtual_async(&state_ui, &merged);
                                content_stack_ui.set_visible_child_name("viewer");
                                let Ok(state) = state_ui.try_borrow() else {
                                    return;
                                };
                                let scope = state.scope.clone();
                                drop(state);
                                apply_scope_to_sidebar(&scope, &sidebar_ui);
                                viewer_ui.clear();
                                filmstrip_ui.refresh_virtual();
                                let first_path = state_ui.try_borrow().ok().and_then(|state| {
                                    state.library.entry_at(0).map(|e: ImageEntry| e.path())
                                });
                                if let Some(path) = first_path {
                                    if let Ok(mut state) = state_ui.try_borrow_mut() {
                                        state.library.selected_index = Some(0);
                                    } else {
                                        return;
                                    }
                                    filmstrip_ui.navigate_to(0);
                                    viewer_ui.load_image(path);
                                }
                            }
                        });

                        glib::ControlFlow::Break
                    });
                *win.imp().inline_search_pending.borrow_mut() = Some(source_id);
            });
        }

        {
            let state_c = state.clone();
            let open_folder_c = open_folder.clone();
            let window_weak = self.downgrade();
            let search_key = gtk4::EventControllerKey::new();
            search_key.connect_key_pressed(move |_, key, _, _| {
                if key != gtk4::gdk::Key::Escape {
                    return glib::Propagation::Proceed;
                }
                let Some(win) = window_weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let Some(revealer) = win.imp().inline_search_revealer.borrow().clone() else {
                    return glib::Propagation::Proceed;
                };
                if !revealer.reveals_child() {
                    return glib::Propagation::Proceed;
                }
                win.clear_inline_search(true);
                if !matches!(state_c.borrow().scope, ViewScope::Folder(_)) {
                    let last_folder = state_c.borrow().settings.last_folder.clone();
                    if let Some(folder) = last_folder {
                        if folder.is_dir() {
                            open_folder_c(folder);
                        }
                    }
                }
                glib::Propagation::Stop
            });
            preview_search_entry.add_controller(search_key);
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
                if ch.is_control() {
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
                entry.grab_focus();
                entry.set_position(-1);
                glib::Propagation::Stop
            });
            self.add_controller(type_to_search);
        }

        if let Some(tag_browser) = tag_browser.as_ref() {
            let content_stack_c = content_stack.clone();
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let window_weak = self.downgrade();
            tag_browser.connect_tags_activated(move |activation| {
                if activation.tags.is_empty() {
                    return;
                }
                let Some(win) = window_weak.upgrade() else {
                    return;
                };
                win.clear_inline_search(true);
                apply_exact_tag_filter(
                    &state_c,
                    &filmstrip_c,
                    &viewer_c,
                    &sidebar_c,
                    &activation.tags,
                );
                if activation.focus_viewer {
                    content_stack_c.set_visible_child_name("viewer");
                }
            });
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
                let mut words: Vec<&str> = query.split_whitespace().collect();
                if words.is_empty() {
                    return;
                }
                let name = words.remove(0).to_string();
                let extra = words.join(", ");
                show_new_collection_dialog(
                    win.upcast(),
                    name,
                    extra,
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
                let Some(_idx) = state_c.borrow().library_index.clone() else {
                    toast_overlay_c.add_toast(libadwaita::Toast::new("Library index unavailable"));
                    return;
                };
                let collections = {
                    let state = state_c.borrow();
                    collections_for_sidebar(&state)
                };

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

                for (collection_id, label) in flatten_collection_paths(&collections) {
                    let row = gtk4::ListBoxRow::new();
                    unsafe {
                        row.set_data("collection-id", collection_id);
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
                        let tags_entry = gtk4::Entry::new();
                        tags_entry.set_placeholder_text(Some("Extra tags, comma separated"));
                        let info =
                            gtk4::Label::new(Some("The collection name is also used as a tag."));
                        info.add_css_class("dim-label");
                        info.set_wrap(true);
                        info.set_halign(gtk4::Align::Start);
                        let eb = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
                        eb.set_margin_top(6);
                        eb.append(&entry);
                        eb.append(&info);
                        eb.append(&tags_entry);
                        new_dialog.set_extra_child(Some(&eb));
                        let paths_cc = paths_c.clone();
                        let state_dd = state_d.clone();
                        let toast_dd = toast_d.clone();
                        let refresh_dd = refresh_d.clone();
                        let tags_entry_c = tags_entry.clone();
                        let create_collection = Rc::new(move |name: String| {
                            let (Some(idx), Some(tags_db)) = (
                                state_dd.borrow().library_index.clone(),
                                state_dd.borrow().tags.clone(),
                            ) else {
                                return;
                            };
                            let started = std::time::Instant::now();
                            let extra_tags =
                                parse_collection_tags_input(tags_entry_c.text().as_str());
                            let library_id = state_dd
                                .borrow()
                                .settings
                                .active_library()
                                .map(|l| l.id.clone())
                                .unwrap_or_default();
                            match idx.create_collection(
                                &library_id,
                                None,
                                &name,
                                &extra_tags,
                                None,
                                None,
                            ) {
                                Ok(coll) => {
                                    let effective_tags =
                                        idx.collection_effective_tags(coll.id).unwrap_or_default();
                                    let added =
                                        tags_db.add_tags_to_paths(&paths_cc, &effective_tags);
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
                                        "Tagged {} image{} for \u{201c}{}\u{201d}",
                                        paths_cc.len(),
                                        if paths_cc.len() == 1 { "" } else { "s" },
                                        coll.name
                                    )));
                                }
                                Err(e) => {
                                    toast_dd.add_toast(libadwaita::Toast::new(&format!(
                                        "Could not create collection: {e}"
                                    )));
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
                    if let (Some(idx), Some(tags_db)) = (
                        state_d.borrow().library_index.clone(),
                        state_d.borrow().tags.clone(),
                    ) {
                        let started = std::time::Instant::now();
                        match idx.collection_effective_tags(id) {
                            Ok(effective_tags) => {
                                let added = tags_db.add_tags_to_paths(&paths_c, &effective_tags);
                                let name = idx
                                    .collection(id)
                                    .ok()
                                    .flatten()
                                    .map(|c| c.name)
                                    .unwrap_or_else(|| "collection".to_string());
                                let _ = idx.touch_collection(id);
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
                                    "Tagged {} image{} for \u{201c}{}\u{201d}",
                                    paths_c.len(),
                                    if paths_c.len() == 1 { "" } else { "s" },
                                    name
                                )));
                            }
                            Err(e) => {
                                toast_d.add_toast(libadwaita::Toast::new(&format!(
                                    "Could not tag images for collection: {e}"
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
            let state_c = state.clone();
            let toast_overlay_c = toast_overlay.clone();
            let refresh_c = refresh_sidebar_collections.clone();
            filmstrip.connect_remove_from_collection_requested(move |paths| {
                let (Some(idx), Some(tags_db)) = (
                    state_c.borrow().library_index.clone(),
                    state_c.borrow().tags.clone(),
                ) else {
                    return;
                };
                let id = match state_c.borrow().scope {
                    ViewScope::Collection(id) => id,
                    _ => return,
                };
                let started = std::time::Instant::now();
                match idx.collection_effective_tags(id) {
                    Ok(effective_tags) => {
                        let removed = tags_db.remove_tags_from_paths(&paths, &effective_tags);
                        let _ = idx.touch_collection(id);
                        crate::bench_event!(
                            "collection.remove_paths",
                            serde_json::json!({
                                "collection_id": id,
                                "path_count": paths.len(),
                                "removed_count": removed,
                                "duration_ms": crate::bench::duration_ms(started),
                            })
                        );
                        let (active_root_buf, disabled) = {
                            let s = state_c.borrow();
                            (
                                s.settings.active_library().map(|lib| lib.root.clone()),
                                s.disabled_folders.clone(),
                            )
                        };
                        let remaining = collection_paths_from_services(
                            &idx,
                            &tags_db,
                            id,
                            active_root_buf.as_deref(),
                            &disabled,
                        );
                        state_c.borrow_mut().scope = ViewScope::Collection(id);
                        load_virtual_async(&state_c, &remaining);
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
                        toast_overlay_c.add_toast(libadwaita::Toast::new(&format!(
                            "Removed {} image{} from collection",
                            paths.len(),
                            if paths.len() == 1 { "" } else { "s" }
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
        content_stack: gtk4::Stack,
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

        // Ctrl+F — reveal and focus inline search.
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Control>F").unwrap()),
            Some(gtk4::CallbackAction::new(move |widget, _| {
                if let Some(window) = widget.downcast_ref::<SharprWindow>() {
                    content_stack.set_visible_child_name("viewer");
                    window.show_inline_search();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
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
                            let new_index = index.min(new_count - 1);
                            filmstrip_d.navigate_to(new_index);
                            let next_path = state_d
                                .borrow()
                                .library
                                .entry_at(new_index)
                                .map(|e: ImageEntry| e.path());
                            if let Some(next_path) = next_path {
                                viewer_d.load_image(next_path.clone());
                                filmstrip_d.set_action_selection_to_path(&next_path);
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
                    {
                        let mut state = state_tr.borrow_mut();
                        state.library.remove_path(&path);
                        remove_path_from_action_selection(&mut state, &path);
                    }
                    let new_count = state_tr.borrow().library.image_count();
                    if new_count == 0 {
                        viewer_tr.clear();
                        filmstrip_tr.refresh_multi_selection_visuals();
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
                            viewer_tr.load_image(p.clone());
                            filmstrip_tr.set_action_selection_to_path(&p);
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

    fn show_inline_search(&self) {
        if let Some(revealer) = self.imp().inline_search_revealer.borrow().as_ref() {
            revealer.set_reveal_child(true);
        }
        if let Some(entry) = self.imp().inline_search_entry.borrow().as_ref() {
            entry.grab_focus();
            entry.set_position(-1);
        }
    }

    fn clear_inline_search(&self, hide: bool) {
        if let Some(source_id) = self.imp().inline_search_pending.borrow_mut().take() {
            source_id.remove();
        }
        let next_gen = self.imp().inline_search_generation.get().wrapping_add(1);
        self.imp().inline_search_generation.set(next_gen);
        self.imp().inline_search_suppressed.set(true);
        if let Some(entry) = self.imp().inline_search_entry.borrow().as_ref() {
            entry.set_text("");
        }
        self.imp().inline_search_suppressed.set(false);
        if hide {
            if let Some(revealer) = self.imp().inline_search_revealer.borrow().as_ref() {
                revealer.set_reveal_child(false);
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

    fn start_thumbnail_poll(
        &self,
        state: Rc<RefCell<AppState>>,
        filmstrip: FilmstripPane,
        tag_browser: Option<TagBrowser>,
    ) {
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
                if let Some(tag_browser) = tag_browser.as_ref() {
                    tag_browser.refresh_preview_for_path(&result_path);
                }

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

        let convert_section = gio::Menu::new();
        convert_section.append(Some("Convert…"), Some("win.convert"));
        menu.append_section(Some("Convert"), &convert_section);

        let library_section = gio::Menu::new();
        library_section.append(Some("Find Duplicates"), Some("win.find-duplicates"));
        library_section.append(Some("Browse Tags"), Some("win.show-tags"));
        menu.append_section(Some("Library"), &library_section);

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
        upscale_action.set_enabled(state.borrow().settings.show_upscale_ui);
        {
            let state_c = state.clone();
            let banner_c = upscale_banner.clone();
            let viewer_weak = viewer.downgrade();
            let action_weak = upscale_action.downgrade();
            upscale_action.connect_activate(move |_action, _| {
                if !state_c.borrow().settings.show_upscale_ui {
                    return;
                }
                let Some(viewer) = viewer_weak.upgrade() else { return };

                // For CLI, lazily detect the binary once.
                let (
                    saved_backend,
                    saved_cli_model,
                    saved_onnx_model,
                    saved_comfyui_workflow,
                    comfyui_enabled,
                    ops_queue,
                ) = {
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
                        st.settings.comfyui_workflow.clone(),
                        st.settings.comfyui_enabled,
                        st.ops.clone(),
                    )
                };

                let backend_kind = UpscaleBackendKind::from_settings(&saved_backend);
                let comfyui_workflow =
                    crate::upscale::ComfyUiWorkflow::from_settings(&saved_comfyui_workflow);
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

                    let advanced_expander = gtk4::Expander::new(Some("Advanced"));
                    advanced_expander.set_expanded(false);
                    let advanced_box = gtk4::Box::new(gtk4::Orientation::Vertical, 18);

                    // ── Backend selection ────────────────────────────────────
                    let backend_label = gtk4::Label::new(Some("Backend"));
                    backend_label.set_halign(gtk4::Align::Start);
                    advanced_box.append(&backend_label);

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
                    advanced_box.append(&cli_btn);
                    advanced_box.append(&onnx_btn);
                    advanced_box.append(&comfyui_btn);

                    // ── Per-backend model sections (stack) ───────────────────
                    let backend_stack = gtk4::Stack::new();
                    backend_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
                    backend_stack.set_transition_duration(120);

                    // CLI page
                    let cli_page = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                    let cli_model_label = gtk4::Label::new(Some("Model"));
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
                    let comfyui_note = gtk4::Label::new(Some(&format!(
                        "{} uses a fixed server-side workflow and ignores Sharpr's model toggle.",
                        comfyui_workflow.display_name()
                    )));
                    comfyui_note.set_halign(gtk4::Align::Start);
                    comfyui_note.add_css_class("dim-label");
                    comfyui_note.set_wrap(true);
                    comfyui_note.set_visible(false);
                    cli_page.append(&cli_model_label);
                    cli_page.append(&standard_btn);
                    cli_page.append(&anime_btn);
                    cli_page.append(&comfyui_note);
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

                    let refresh_cli_model_ui = {
                        let standard_btn = standard_btn.clone();
                        let anime_btn = anime_btn.clone();
                        let comfyui_btn = comfyui_btn.clone();
                        let comfyui_note = comfyui_note.clone();
                        move || {
                            let fixed_model = comfyui_btn.is_active()
                                && !comfyui_workflow.uses_sharpr_model_picker();
                            standard_btn.set_sensitive(!fixed_model);
                            anime_btn.set_sensitive(!fixed_model);
                            comfyui_note.set_visible(fixed_model);
                        }
                    };
                    let sync_backend_stack = {
                        let stack_c = backend_stack.clone();
                        let onnx_btn = onnx_btn.clone();
                        move || {
                            stack_c.set_visible_child_name(if onnx_btn.is_active() {
                                "onnx"
                            } else {
                                "cli"
                            });
                        }
                    };
                    refresh_cli_model_ui();
                    sync_backend_stack();
                    {
                        let refresh = refresh_cli_model_ui.clone();
                        let sync = sync_backend_stack.clone();
                        onnx_btn.connect_toggled(move |_| {
                            sync();
                            refresh();
                        });
                    }
                    {
                        let refresh = refresh_cli_model_ui.clone();
                        let sync = sync_backend_stack.clone();
                        cli_btn.connect_toggled(move |_| {
                            sync();
                            refresh();
                        });
                    }
                    {
                        let refresh = refresh_cli_model_ui.clone();
                        let sync = sync_backend_stack.clone();
                        comfyui_btn.connect_toggled(move |_| {
                            sync();
                            refresh();
                        });
                    }
                    advanced_box.append(&backend_stack);

                    advanced_expander.set_child(Some(&advanced_box));
                    content.append(&advanced_expander);

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

        let export_action = gio::SimpleAction::new("export", None);
        {
            let state_c = state.clone();
            let window_weak = self.downgrade();
            export_action.connect_activate(move |_, _| {
                let Some(win) = window_weak.upgrade() else {
                    return;
                };

                let sources: Vec<PathBuf> = {
                    let st = state_c.borrow();
                    let sources = effective_selected_paths(&st);
                    if sources.is_empty() {
                        return;
                    };
                    sources
                };

                let dialog = libadwaita::AlertDialog::new(Some("Export Images"), None);
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("export", "Export");
                dialog.set_default_response(Some("export"));
                dialog.set_close_response("cancel");
                dialog.set_response_appearance("export", libadwaita::ResponseAppearance::Suggested);

                let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
                content.set_margin_top(6);
                content.set_margin_bottom(6);

                let dest_path_label = gtk4::Label::new(None);
                dest_path_label.set_halign(gtk4::Align::Start);
                dest_path_label.set_hexpand(true);
                let dest_label = gtk4::Label::new(Some("Saved to"));
                dest_label.set_halign(gtk4::Align::Start);
                dest_label.add_css_class("dim-label");
                let dest_row = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
                dest_row.set_margin_bottom(4);
                dest_row.append(&dest_label);
                dest_row.append(&dest_path_label);
                content.append(&dest_row);

                let edge_drop = gtk4::DropDown::from_strings(&["Original", "1920px", "2560px"]);
                let fmt_drop = gtk4::DropDown::from_strings(&["JPEG", "WebP", "PNG"]);

                let edge_label = gtk4::Label::new(Some("Max size"));
                edge_label.set_halign(gtk4::Align::Start);
                content.append(&edge_label);
                content.append(&edge_drop);

                let fmt_label = gtk4::Label::new(Some("Format"));
                fmt_label.set_halign(gtk4::Align::Start);
                content.append(&fmt_label);
                content.append(&fmt_drop);

                let quality_label = gtk4::Label::new(Some("Quality (1–100)"));
                quality_label.set_halign(gtk4::Align::Start);
                let quality_spin = gtk4::SpinButton::with_range(1.0, 100.0, 1.0);
                quality_spin.set_value(85.0);
                content.append(&quality_label);
                content.append(&quality_spin);

                {
                    let ql = quality_label.clone();
                    let qs = quality_spin.clone();
                    fmt_drop.connect_selected_notify(move |drop| {
                        let visible = drop.selected() != 2;
                        ql.set_visible(visible);
                        qs.set_visible(visible);
                    });
                }

                let default_dest = {
                    let st = state_c.borrow();
                    crate::export::resolve_output_dir(
                        st.settings.export_output_dir.as_ref(),
                        crate::export::OutputFolderKind::Export,
                    )
                };
                dest_path_label.set_label(default_dest.to_string_lossy().as_ref());

                dialog.set_extra_child(Some(&content));

                let state_cc = state_c.clone();
                let win_weak3 = win.downgrade();
                dialog.connect_response(None, move |_, response| {
                    if response != "export" {
                        return;
                    }
                    let dest = {
                        let st = state_cc.borrow();
                        crate::export::resolve_output_dir(
                            st.settings.export_output_dir.as_ref(),
                            crate::export::OutputFolderKind::Export,
                        )
                    };

                    let max_edge = match edge_drop.selected() {
                        1 => Some(1920u32),
                        2 => Some(2560u32),
                        _ => None,
                    };
                    let format = match fmt_drop.selected() {
                        1 => crate::export::ExportFormat::Webp,
                        2 => crate::export::ExportFormat::Png,
                        _ => crate::export::ExportFormat::Jpeg,
                    };
                    let quality = quality_spin.value() as u8;

                    let config = crate::export::ExportConfig {
                        destination: dest,
                        max_edge,
                        format,
                        quality,
                        filename_suffix: Some(crate::export::export_filename_suffix(
                            max_edge, format,
                        )),
                    };

                    let total = sources.len();
                    let op = state_cc
                        .borrow()
                        .ops
                        .add(format!("Exporting {total} image(s)"));
                    let win_export = win_weak3.clone();
                    let sources_export = sources.clone();

                    let (tx, rx) = async_channel::unbounded::<Result<(), String>>();
                    rayon::spawn(move || {
                        for source in &sources_export {
                            let result = crate::export::export_image(source, &config)
                                .map(|_| ())
                                .map_err(|e| format!("{}: {e}", source.display()));
                            let _ = tx.send_blocking(result);
                        }
                    });

                    glib::MainContext::default().spawn_local(async move {
                        let mut ok = 0usize;
                        let mut failed = 0usize;
                        while let Ok(result) = rx.recv().await {
                            match result {
                                Ok(()) => ok += 1,
                                Err(e) => {
                                    failed += 1;
                                    eprintln!("export error: {e}");
                                }
                            }
                            let done = ok + failed;
                            op.progress(Some(done as f32 / total as f32));
                        }
                        if failed == 0 {
                            op.complete();
                            let msg = if ok == 1 {
                                "Image exported".to_string()
                            } else {
                                format!("{ok} images exported")
                            };
                            if let Some(w) = win_export.upgrade() {
                                w.add_toast(libadwaita::Toast::new(&msg));
                            }
                        } else {
                            let msg = format!("{ok}/{total} exported, {failed} failed");
                            op.fail(msg.clone());
                            if let Some(w) = win_export.upgrade() {
                                w.add_toast(libadwaita::Toast::new(&msg));
                            }
                        }
                    });
                });

                dialog.present(Some(&win));
            });
        }
        self.add_action(&export_action);

        let convert_action = gio::SimpleAction::new("convert", None);
        {
            let state_c = state.clone();
            let banner_c = upscale_banner.clone();
            let viewer_weak = viewer.downgrade();
            let window_weak = self.downgrade();
            let action_weak = convert_action.downgrade();

            convert_action.connect_activate(move |_, _| {
                let Some(win) = window_weak.upgrade() else { return };
                let Some(viewer) = viewer_weak.upgrade() else { return };

                let (
                    sources,
                    show_upscale,
                    saved_backend,
                    saved_cli_model,
                    saved_onnx_model,
                    saved_comfyui_workflow,
                    comfyui_enabled,
                    ops_queue,
                ) = {
                    let mut st = state_c.borrow_mut();
                    if st.upscale_binary.is_none() {
                        st.upscale_binary = AppSettings::load()
                            .upscaler_binary_path
                            .filter(|p| p.is_file());
                    }
                    if st.upscale_binary.is_none() {
                        st.upscale_binary = UpscaleDetector::find_realesrgan();
                    }
                    let srcs = if !st.selected_paths.is_empty() {
                        let selected = effective_selected_paths(&st);
                        if selected.is_empty() {
                            return;
                        }
                        selected
                    } else if let Some(entry) = st.library.selected_entry() {
                        vec![entry.path()]
                    } else {
                        return;
                    };
                    (
                        srcs,
                        st.settings.show_upscale_ui,
                        st.settings.upscale_backend.clone(),
                        st.settings.upscaler_default_model.clone(),
                        st.settings.onnx_upscale_model.clone(),
                        st.settings.comfyui_workflow.clone(),
                        st.settings.comfyui_enabled,
                        st.ops.clone(),
                    )
                };

                let backend_kind = UpscaleBackendKind::from_settings(&saved_backend);
                let comfyui_workflow =
                    crate::upscale::ComfyUiWorkflow::from_settings(&saved_comfyui_workflow);

                // ── Dialog ──────────────────────────────────────────────────
                let dialog = libadwaita::AlertDialog::new(Some("Convert"), None);
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("convert", "Convert");
                dialog.set_default_response(Some("convert"));
                dialog.set_close_response("cancel");
                dialog.set_response_appearance(
                    "convert",
                    libadwaita::ResponseAppearance::Suggested,
                );

                let content = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
                content.set_margin_top(6);
                content.set_margin_bottom(6);

                // ── Mode toggle ─────────────────────────────────────────────
                let downscale_radio = gtk4::CheckButton::with_label("Downscale");
                downscale_radio.set_active(true);
                let upscale_radio = gtk4::CheckButton::with_label("AI Upscale");
                upscale_radio.set_group(Some(&downscale_radio));

                let mode_stack = gtk4::Stack::new();
                mode_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
                mode_stack.set_transition_duration(120);

                // ── Upscale widgets (created unconditionally so they are in
                //    scope for the connect_response closure regardless of
                //    show_upscale; they are only *added to the UI* when
                //    show_upscale = true) ────────────────────────────────────
                let scale_drop =
                    gtk4::DropDown::from_strings(&["Smart (auto)", "2×", "3×", "4×"]);
                scale_drop.set_selected(0);

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
                match backend_kind {
                    UpscaleBackendKind::Onnx => onnx_btn.set_active(true),
                    UpscaleBackendKind::ComfyUi if comfyui_enabled => {
                        comfyui_btn.set_active(true)
                    }
                    _ => cli_btn.set_active(true),
                }

                let standard_btn = gtk4::CheckButton::with_label("Standard – best for photos");
                let anime_btn =
                    gtk4::CheckButton::with_label("Anime / Art – best for illustration");
                anime_btn.set_group(Some(&standard_btn));
                if saved_cli_model == "anime" {
                    anime_btn.set_active(true);
                } else {
                    standard_btn.set_active(true);
                }

                let onnx_variants = [
                    OnnxUpscaleModel::Swin2srLightweightX2,
                    OnnxUpscaleModel::Swin2srCompressedX4,
                    OnnxUpscaleModel::Swin2srRealX4,
                ];
                let onnx_labels: Vec<&str> =
                    onnx_variants.iter().map(|m| m.info().display_name).collect();
                let onnx_drop = gtk4::DropDown::from_strings(&onnx_labels);
                let saved_onnx = OnnxUpscaleModel::from_settings(&saved_onnx_model);
                let onnx_sel = onnx_variants
                    .iter()
                    .position(|&m| m == saved_onnx)
                    .unwrap_or(0) as u32;
                onnx_drop.set_selected(onnx_sel);

                // ── Downscale pane ───────────────────────────────────────────
                let ds_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

                let dest_label = gtk4::Label::new(Some("No folder selected"));
                dest_label.set_halign(gtk4::Align::Start);
                dest_label.set_hexpand(true);
                let choose_btn = gtk4::Button::with_label("Choose Folder…");
                let dest_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                dest_row.append(&dest_label);
                dest_row.append(&choose_btn);
                ds_box.append(&dest_row);

                let edge_lbl = gtk4::Label::new(Some("Max size"));
                edge_lbl.set_halign(gtk4::Align::Start);
                let edge_drop =
                    gtk4::DropDown::from_strings(&["Original", "1920px", "2560px", "1280px"]);
                ds_box.append(&edge_lbl);
                ds_box.append(&edge_drop);

                let fmt_lbl = gtk4::Label::new(Some("Format"));
                fmt_lbl.set_halign(gtk4::Align::Start);
                let fmt_drop = gtk4::DropDown::from_strings(&["JPEG", "WebP", "PNG"]);
                ds_box.append(&fmt_lbl);
                ds_box.append(&fmt_drop);

                let quality_lbl = gtk4::Label::new(Some("Quality (1–100)"));
                quality_lbl.set_halign(gtk4::Align::Start);
                let quality_spin = gtk4::SpinButton::with_range(1.0, 100.0, 1.0);
                quality_spin.set_value(85.0);
                ds_box.append(&quality_lbl);
                ds_box.append(&quality_spin);

                {
                    let ql = quality_lbl.clone();
                    let qs = quality_spin.clone();
                    fmt_drop.connect_selected_notify(move |drop| {
                        let visible = drop.selected() != 2;
                        ql.set_visible(visible);
                        qs.set_visible(visible);
                    });
                }

                let default_dest = {
                    let st = state_c.borrow();
                    crate::export::resolve_output_dir(
                        st.settings.export_output_dir.as_ref(),
                        crate::export::OutputFolderKind::Export,
                    )
                };
                dest_label.set_label(default_dest.to_string_lossy().as_ref());
                choose_btn.set_visible(false);

                mode_stack.add_named(&ds_box, Some("downscale"));

                // ── Upscale pane (only added to UI when show_upscale) ────────
                if show_upscale {
                    let mode_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
                    mode_box.append(&downscale_radio);
                    mode_box.append(&upscale_radio);
                    content.append(&mode_box);
                    {
                        let stack_c = mode_stack.clone();
                        upscale_radio.connect_toggled(move |btn| {
                            stack_c.set_visible_child_name(if btn.is_active() {
                                "upscale"
                            } else {
                                "downscale"
                            });
                        });
                    }

                    let us_box = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
                    let scale_lbl = gtk4::Label::new(Some("Scale"));
                    scale_lbl.set_halign(gtk4::Align::Start);
                    us_box.append(&scale_lbl);
                    us_box.append(&scale_drop);

                    let adv_expander = gtk4::Expander::new(Some("Advanced"));
                    adv_expander.set_expanded(false);
                    let adv_box = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
                    let be_lbl = gtk4::Label::new(Some("Backend"));
                    be_lbl.set_halign(gtk4::Align::Start);
                    adv_box.append(&be_lbl);
                    adv_box.append(&cli_btn);
                    adv_box.append(&onnx_btn);
                    adv_box.append(&comfyui_btn);

                    let be_stack = gtk4::Stack::new();
                    be_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
                    be_stack.set_transition_duration(120);

                    let cli_page = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                    let cli_model_lbl = gtk4::Label::new(Some("Model"));
                    cli_model_lbl.set_halign(gtk4::Align::Start);
                    let comfyui_note = gtk4::Label::new(Some(&format!(
                        "{} uses a fixed server-side workflow and ignores Sharpr's model toggle.",
                        comfyui_workflow.display_name()
                    )));
                    comfyui_note.set_halign(gtk4::Align::Start);
                    comfyui_note.add_css_class("dim-label");
                    comfyui_note.set_wrap(true);
                    comfyui_note.set_visible(false);
                    cli_page.append(&cli_model_lbl);
                    cli_page.append(&standard_btn);
                    cli_page.append(&anime_btn);
                    cli_page.append(&comfyui_note);
                    be_stack.add_named(&cli_page, Some("cli"));

                    let onnx_page = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                    let onnx_model_lbl = gtk4::Label::new(Some("ONNX Model"));
                    onnx_model_lbl.set_halign(gtk4::Align::Start);

                    let dl_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                    dl_row.set_halign(gtk4::Align::Start);
                    let dl_btn = gtk4::Button::new();
                    let dl_status = gtk4::Label::new(None);
                    dl_status.add_css_class("dim-label");
                    let refresh_dl = {
                        let dl_btn = dl_btn.clone();
                        let dl_status = dl_status.clone();
                        move |model: OnnxUpscaleModel| {
                            use crate::upscale::backends::onnx::OnnxBackend;
                            if OnnxBackend::model_path(model).exists() {
                                dl_btn.set_visible(false);
                                dl_status.set_text("Downloaded ✓");
                            } else {
                                let info = model.info();
                                dl_btn.set_label(&format!(
                                    "Download ({} MB)",
                                    info.download_size_mb
                                ));
                                dl_btn.set_visible(true);
                                dl_status.set_text("");
                            }
                        }
                    };
                    refresh_dl(onnx_variants[onnx_sel as usize]);
                    {
                        let refresh = refresh_dl.clone();
                        let onnx_drop_c = onnx_drop.clone();
                        onnx_drop_c.connect_selected_notify(move |dd| {
                            if let Some(&m) = onnx_variants.get(dd.selected() as usize) {
                                refresh(m);
                            }
                        });
                    }
                    {
                        let dl_btn_c = dl_btn.clone();
                        let dl_status_c = dl_status.clone();
                        let onnx_drop_c = onnx_drop.clone();
                        let ops_c = ops_queue.clone();
                        dl_btn.connect_clicked(move |btn| {
                            let idx = onnx_drop_c.selected() as usize;
                            let Some(&model) = onnx_variants.get(idx) else { return };
                            btn.set_sensitive(false);
                            dl_status_c.set_text("Downloading…");
                            let rx = downloader::download_model(model);
                            let op = ops_c.add(format!(
                                "Downloading {}",
                                model.info().display_name
                            ));
                            let btn_weak = dl_btn_c.downgrade();
                            let status_weak = dl_status_c.downgrade();
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
                    dl_row.append(&dl_btn);
                    dl_row.append(&dl_status);
                    onnx_page.append(&onnx_model_lbl);
                    onnx_page.append(&onnx_drop);
                    onnx_page.append(&dl_row);
                    be_stack.add_named(&onnx_page, Some("onnx"));

                    let refresh_cli_model_ui = {
                        let standard_btn = standard_btn.clone();
                        let anime_btn = anime_btn.clone();
                        let comfyui_btn = comfyui_btn.clone();
                        let comfyui_note = comfyui_note.clone();
                        move || {
                            let fixed_model = comfyui_btn.is_active()
                                && !comfyui_workflow.uses_sharpr_model_picker();
                            standard_btn.set_sensitive(!fixed_model);
                            anime_btn.set_sensitive(!fixed_model);
                            comfyui_note.set_visible(fixed_model);
                        }
                    };
                    let sync_backend_stack = {
                        let stack_c = be_stack.clone();
                        let onnx_btn = onnx_btn.clone();
                        move || {
                            stack_c.set_visible_child_name(if onnx_btn.is_active() {
                                "onnx"
                            } else {
                                "cli"
                            });
                        }
                    };
                    refresh_cli_model_ui();
                    sync_backend_stack();
                    {
                        let refresh = refresh_cli_model_ui.clone();
                        let sync = sync_backend_stack.clone();
                        onnx_btn.connect_toggled(move |_| {
                            sync();
                            refresh();
                        });
                    }
                    {
                        let refresh = refresh_cli_model_ui.clone();
                        let sync = sync_backend_stack.clone();
                        cli_btn.connect_toggled(move |_| {
                            sync();
                            refresh();
                        });
                    }
                    {
                        let refresh = refresh_cli_model_ui.clone();
                        let sync = sync_backend_stack.clone();
                        comfyui_btn.connect_toggled(move |_| {
                            sync();
                            refresh();
                        });
                    }
                    adv_box.append(&be_stack);
                    adv_expander.set_child(Some(&adv_box));
                    us_box.append(&adv_expander);
                    mode_stack.add_named(&us_box, Some("upscale"));
                }

                mode_stack.set_visible_child_name("downscale");
                content.append(&mode_stack);
                dialog.set_extra_child(Some(&content));

                // ── Response handler ─────────────────────────────────────────
                let state_cc = state_c.clone();
                let win_weak3 = window_weak.clone();
                let viewer_c = viewer.clone();
                let action_weak_c = action_weak.clone();
                let banner_cc = banner_c.clone();
                dialog.connect_response(None, move |_, response| {
                    if response != "convert" {
                        return;
                    }

                    let is_upscale = show_upscale && !downscale_radio.is_active();

                    if is_upscale {
                        // ── AI Upscale path ──────────────────────────────────
                        let backend_kind2 = if onnx_btn.is_active() {
                            UpscaleBackendKind::Onnx
                        } else if comfyui_btn.is_active() {
                            UpscaleBackendKind::ComfyUi
                        } else {
                            UpscaleBackendKind::Cli
                        };
                        if backend_kind2 == UpscaleBackendKind::Cli
                            && state_cc.borrow().upscale_binary.is_none()
                        {
                            banner_cc.set_title("AI upscaling requires a supported Vulkan backend such as upscayl-bin or realesrgan-ncnn-vulkan.");
                            banner_cc.set_revealed(true);
                            return;
                        }
                        banner_cc.set_revealed(false);

                        let chosen_onnx = onnx_variants
                            .get(onnx_drop.selected() as usize)
                            .copied()
                            .unwrap_or_default();
                        {
                            let mut st = state_cc.borrow_mut();
                            st.settings.set_upscale_backend(backend_kind2.settings_key());
                            st.settings.set_onnx_upscale_model(chosen_onnx.settings_key());
                            st.settings.set_upscaler_default_model(
                                if anime_btn.is_active() { "anime" } else { "standard" },
                            );
                        }

                        if let Some(action) = action_weak_c.upgrade() {
                            action.set_enabled(false);
                        }
                        let action_weak_cc = action_weak_c.clone();
                        let proxy_btn = gtk4::Button::new();
                        proxy_btn.connect_sensitive_notify(move |btn| {
                            if btn.is_sensitive() {
                                if let Some(a) = action_weak_cc.upgrade() {
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
                        let scale = match scale_drop.selected() {
                            1 => 2,
                            2 => 3,
                            3 => 4,
                            _ => 0,
                        };
                        if sources.len() == 1 {
                            viewer_c.start_upscale(sources[0].clone(), scale, cli_model, proxy_btn);
                        } else {
                            viewer_c.start_upscale_batch(
                                sources.clone(),
                                scale,
                                cli_model,
                                proxy_btn,
                            );
                        }
                    } else {
                        // ── Downscale path ───────────────────────────────────
                        let max_edge = match edge_drop.selected() {
                            1 => Some(1920u32),
                            2 => Some(2560u32),
                            3 => Some(1280u32),
                            _ => None,
                        };
                        let format = match fmt_drop.selected() {
                            1 => crate::export::ExportFormat::Webp,
                            2 => crate::export::ExportFormat::Png,
                            _ => crate::export::ExportFormat::Jpeg,
                        };
                        let quality = quality_spin.value() as u8;

                        if sources.len() == 1 {
                            // Single image → comparison preview
                            let source = sources[0].clone();
                            let destination = {
                                let st = state_cc.borrow();
                                crate::export::resolve_output_dir(
                                    st.settings.export_output_dir.as_ref(),
                                    crate::export::OutputFolderKind::Export,
                                )
                            };
                            let config = crate::export::ExportConfig {
                                destination,
                                max_edge,
                                format,
                                quality,
                                filename_suffix: Some(crate::export::export_filename_suffix(
                                    max_edge, format,
                                )),
                            };
                            viewer_c.start_downscale_preview(source, config);
                        } else {
                            let dest = {
                                let st = state_cc.borrow();
                                crate::export::resolve_output_dir(
                                    st.settings.export_output_dir.as_ref(),
                                    crate::export::OutputFolderKind::Export,
                                )
                            };
                            let config = crate::export::ExportConfig {
                                destination: dest.clone(),
                                max_edge,
                                format,
                                quality,
                                filename_suffix: Some(crate::export::export_filename_suffix(
                                    max_edge, format,
                                )),
                            };
                            // Batch → run directly with progress
                            let total = sources.len();
                            let op = state_cc
                                .borrow()
                                .ops
                                .add(format!("Exporting {total} image(s)"));
                            let win_weak_c = win_weak3.clone();
                            let srcs = sources.clone();
                            let (tx, rx) = async_channel::unbounded::<Result<(), String>>();
                            rayon::spawn(move || {
                                for src in &srcs {
                                    let result = crate::export::export_image(src, &config)
                                        .map(|_| ())
                                        .map_err(|e| format!("{}: {e}", src.display()));
                                    let _ = tx.send_blocking(result);
                                }
                            });
                            glib::MainContext::default().spawn_local(async move {
                                let mut ok = 0usize;
                                let mut failed = 0usize;
                                while let Ok(result) = rx.recv().await {
                                    match result {
                                        Ok(()) => ok += 1,
                                        Err(e) => {
                                            failed += 1;
                                            eprintln!("export error: {e}");
                                        }
                                    }
                                    let done = ok + failed;
                                    op.progress(Some(done as f32 / total as f32));
                                }
                                if failed == 0 {
                                    op.complete();
                                    let msg = if ok == 1 {
                                        "Image exported".to_string()
                                    } else {
                                        format!("{ok} images exported")
                                    };
                                    if let Some(w) = win_weak_c.upgrade() {
                                        w.add_toast(libadwaita::Toast::new(&msg));
                                    }
                                } else {
                                    let msg = format!("{ok}/{total} exported, {failed} failed");
                                    op.fail(msg.clone());
                                    if let Some(w) = win_weak_c.upgrade() {
                                        w.add_toast(libadwaita::Toast::new(&msg));
                                    }
                                }
                            });
                        }
                    }
                });

                dialog.present(Some(&win));
            });
        }
        self.add_action(&convert_action);
    }

    /// Build the viewer header bar.
    /// Returns `(header, sidebar_toggle, preview_title_btn, commit_btn, commit_menu_btn, discard_btn, edit_commit_btn, edit_discard_btn)`.
    /// Commit and Discard are initially hidden; the comparison view shows them.
    fn build_viewer_header(
        &self,
        menu_btn: &gtk4::MenuButton,
    ) -> (
        libadwaita::HeaderBar,
        gtk4::ToggleButton,
        gtk4::Button,
        gtk4::Button,
        gtk4::MenuButton,
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
        commit_btn.set_tooltip_text(Some("Save to the configured output folder"));
        commit_btn.add_css_class("suggested-action");
        commit_btn.set_visible(false);
        header.pack_end(&commit_btn);

        let commit_menu_btn = gtk4::MenuButton::new();
        commit_menu_btn.set_icon_name("pan-down-symbolic");
        commit_menu_btn.set_tooltip_text(Some("More save options"));
        commit_menu_btn.add_css_class("flat");
        commit_menu_btn.set_visible(false);
        header.pack_end(&commit_menu_btn);

        let discard_btn = gtk4::Button::with_label("Discard");
        discard_btn.set_tooltip_text(Some("Discard the pending converted image"));
        discard_btn.add_css_class("destructive-action");
        discard_btn.set_visible(false);
        header.pack_end(&discard_btn);

        let edit_commit_btn = gtk4::Button::with_label("Save");
        edit_commit_btn.set_tooltip_text(Some("Save rotated or flipped changes to disk"));
        edit_commit_btn.add_css_class("suggested-action");
        edit_commit_btn.set_visible(false);
        header.pack_end(&edit_commit_btn);

        let edit_discard_btn = gtk4::Button::with_label("Discard");
        edit_discard_btn.set_tooltip_text(Some("Discard rotated or flipped changes"));
        edit_discard_btn.add_css_class("destructive-action");
        edit_discard_btn.set_visible(false);
        header.pack_end(&edit_discard_btn);

        header.pack_end(menu_btn);

        (
            header,
            sidebar_toggle,
            preview_title_btn,
            commit_btn,
            commit_menu_btn,
            discard_btn,
            edit_commit_btn,
            edit_discard_btn,
        )
    }
}
