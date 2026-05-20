use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use glib::WeakRef;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;

use crate::export::{
    export_to_path, resolve_output_dir, unique_output_path, ExportFormat, OutputFolderKind,
};
use crate::library_index::{LibraryIndex, Pipeline, PipelineStatus, PipelineStep, StepType};
use crate::ui::compare_item::{
    build_compare_item_from_pipeline, CompareItem, ExportStepSettings, UpscaleStepSettings,
};
use crate::ui::window::AppState;
use crate::upscale::{
    backend::make_upscale_backend,
    runner::{preserved_png_temp_path, UpscaleRunner},
    UpscaleBackendKind, UpscaleCompressionMode, UpscaleJobConfig, UpscaleModel,
    UpscaleOutputFormat,
};

pub type CompareCallback = Box<dyn Fn(CompareItem)>;
pub type UserActivityCallback = Box<dyn Fn(bool)>;

fn inherit_generated_output_metadata(
    state: &AppState,
    source: &Path,
    output: &Path,
    step_type: StepType,
) {
    if let Some(tags) = state.tags.as_ref() {
        tags.copy_tags(source, output);
    }

    let Some(idx) = state.library_index.as_ref() else {
        return;
    };
    let _ = idx.copy_collection_memberships(source, output);
    let output_collection = match step_type {
        StepType::Upscale => "Upscaled",
        StepType::Export => "Exports",
    };
    if let Ok(collection_id) = idx.ensure_output_collection(output_collection) {
        let _ = idx.add_to_collection_by_id(output, collection_id);
    }
}

/// Per-item configuration held in memory until Start Queue is pressed.
#[derive(Clone)]
pub struct PendingConfig {
    pub upscale_on: bool,
    pub upscale: UpscaleStepSettings,
    pub export_on: bool,
    pub export: ExportStepSettings,
}

impl Default for PendingConfig {
    fn default() -> Self {
        Self {
            upscale_on: true,
            upscale: UpscaleStepSettings {
                backend: "onnx".to_string(),
                model: String::new(),
                onnx_model: None,
                scale: 0,
                compress: false,
                format: "png".to_string(),
                quality: 85,
                keep_png: false,
                destination: "default".to_string(),
                custom_path: None,
                comfyui_workflow: None,
            },
            export_on: false,
            export: ExportStepSettings {
                format: "jxl".to_string(),
                max_edge: None,
                quality: 90,
                destination: "default".to_string(),
                custom_path: None,
            },
        }
    }
}

pub(super) struct BackgroundTaskRow {
    row: gtk4::ListBoxRow,
    progress_bar: gtk4::ProgressBar,
    status_label: gtk4::Label,
    active: Cell<bool>,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct TasksPage {
        pub queue_list: RefCell<Option<gtk4::ListBox>>,
        pub start_btn: RefCell<Option<gtk4::Button>>,
        pub pause_btn: RefCell<Option<gtk4::Button>>,
        pub clear_btn: RefCell<Option<gtk4::Button>>,
        pub add_images_btn: RefCell<Option<gtk4::Button>>,
        pub remove_btn: RefCell<Option<gtk4::Button>>,
        pub queue_count_label: RefCell<Option<gtk4::Label>>,
        pub background_status_label: RefCell<Option<gtk4::Label>>,
        pub background_empty_label: RefCell<Option<gtk4::Label>>,
        pub background_list: RefCell<Option<gtk4::ListBox>>,
        pub crash_banner: RefCell<Option<libadwaita::Banner>>,
        pub queue_empty_status: RefCell<Option<libadwaita::StatusPage>>,

        // Upscale toggle + settings
        pub upscale_toggle: RefCell<Option<gtk4::Switch>>,
        pub upscale_settings_box: RefCell<Option<gtk4::Box>>,
        pub backend_onnx_btn: RefCell<Option<gtk4::ToggleButton>>,
        pub backend_comfyui_btn: RefCell<Option<gtk4::ToggleButton>>,
        pub backend_cli_btn: RefCell<Option<gtk4::ToggleButton>>,
        pub onnx_model_dropdown: RefCell<Option<libadwaita::ComboRow>>,
        pub onnx_model_row: RefCell<Option<libadwaita::ComboRow>>,
        pub scale_dropdown: RefCell<Option<libadwaita::ComboRow>>,
        pub comfyui_workflow_row: RefCell<Option<libadwaita::ComboRow>>,

        // Convert/Export toggle + settings
        pub export_toggle: RefCell<Option<gtk4::Switch>>,
        pub export_settings_box: RefCell<Option<gtk4::Box>>,
        pub export_format_dropdown: RefCell<Option<libadwaita::ComboRow>>,
        pub export_edge_dropdown: RefCell<Option<libadwaita::ComboRow>>,
        pub export_quality_spin: RefCell<Option<gtk4::SpinButton>>,
        pub export_dest_dropdown: RefCell<Option<libadwaita::ComboRow>>,
        pub export_custom_row: RefCell<Option<libadwaita::ActionRow>>,
        pub export_custom_label: RefCell<Option<gtk4::Label>>,
        pub history_cap_spin: RefCell<Option<gtk4::SpinButton>>,

        // Right panel state
        pub right_scroll: RefCell<Option<gtk4::ScrolledWindow>>,
        pub right_settings_box: RefCell<Option<gtk4::Box>>,
        pub no_selection_label: RefCell<Option<gtk4::Label>>,
        pub summary_action_list: RefCell<Option<gtk4::Box>>,
        pub summary_estimated_label: RefCell<Option<gtk4::Label>>,

        // History
        pub history_list: RefCell<Option<gtk4::ListBox>>,
        pub clear_history_btn: RefCell<Option<gtk4::Button>>,
        pub history_section: RefCell<Option<gtk4::Box>>,

        // State
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
        pub selected_pipeline_id: RefCell<Option<i64>>,
        pub compare_cb: RefCell<Option<CompareCallback>>,
        pub user_activity_cb: RefCell<Option<UserActivityCallback>>,
        pub parent_window: RefCell<WeakRef<gtk4::Window>>,
        pub runner_active: Rc<Cell<bool>>,
        pub paused: Rc<Cell<bool>>,
        pub selected_is_history: Cell<bool>,
        pub export_custom_path: RefCell<Option<PathBuf>>,
        pub queue_row_selected_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub history_row_selected_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub polling_timer: RefCell<Option<glib::SourceId>>,
        pub(super) background_rows: RefCell<HashMap<u64, BackgroundTaskRow>>,
        pub(super) background_active_count: Cell<u32>,

        // Per-item pending configurations (not yet committed to DB)
        pub pending_configs: RefCell<HashMap<i64, PendingConfig>>,
        pub queue_checked_ids: RefCell<HashSet<i64>>,
        pub queue_chip_suffixes: RefCell<HashMap<i64, gtk4::Box>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TasksPage {
        const NAME: &'static str = "SharprTasksPage";
        type Type = super::TasksPage;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for TasksPage {
        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();

            let main_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            main_box.set_parent(&*widget);

            // --- Left Column ---
            let left_col = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            left_col.set_hexpand(true);
            left_col.set_margin_top(12);
            left_col.set_margin_bottom(12);
            left_col.set_margin_start(12);
            left_col.set_margin_end(12);

            let crash_banner =
                libadwaita::Banner::new("Unfinished jobs from previous session detected.");
            crash_banner.set_button_label(Some("Resume All"));
            crash_banner.set_revealed(false);
            left_col.append(&crash_banner);

            let background_section = gtk4::Box::new(gtk4::Orientation::Vertical, 6);

            let background_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            background_header.set_hexpand(true);

            let background_title = gtk4::Label::new(Some("Background Activity"));
            background_title.add_css_class("heading");
            background_title.set_halign(gtk4::Align::Start);

            let background_status_label = gtk4::Label::new(Some("No background activity"));
            background_status_label.add_css_class("dim-label");
            background_status_label.add_css_class("caption");
            background_status_label.set_halign(gtk4::Align::Start);

            let background_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            background_spacer.set_hexpand(true);

            background_header.append(&background_title);
            background_header.append(&background_status_label);
            background_header.append(&background_spacer);

            let background_list = gtk4::ListBox::new();
            background_list.add_css_class("boxed-list");
            background_list.set_selection_mode(gtk4::SelectionMode::None);
            background_list.set_visible(false);

            let background_empty_label = gtk4::Label::new(Some("No background activity"));
            background_empty_label.add_css_class("dim-label");
            background_empty_label.add_css_class("caption");
            background_empty_label.set_halign(gtk4::Align::Start);

            background_section.append(&background_header);
            background_section.append(&background_list);
            background_section.append(&background_empty_label);
            left_col.append(&background_section);

            let queue_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            queue_header.set_hexpand(true);

            let queue_title = gtk4::Label::new(Some("Queue"));
            queue_title.add_css_class("heading");
            queue_title.set_halign(gtk4::Align::Start);

            let queue_count_label = gtk4::Label::new(None);
            queue_count_label.add_css_class("dim-label");
            queue_count_label.add_css_class("caption");
            queue_count_label.set_halign(gtk4::Align::Start);
            queue_count_label.set_visible(false);

            let header_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            header_spacer.set_hexpand(true);

            let add_images_btn = gtk4::Button::builder()
                .label("+ Add Selected")
                .icon_name("list-add-symbolic")
                .build();
            add_images_btn.add_css_class("flat");
            add_images_btn.set_tooltip_text(Some("Browse and add image files to the queue"));

            let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            toolbar.set_halign(gtk4::Align::End);
            toolbar.add_css_class("linked");

            let pause_btn = gtk4::Button::builder()
                .label("Pause")
                .icon_name("media-playback-pause-symbolic")
                .build();
            pause_btn.set_tooltip_text(Some("Pause the queue"));
            let clear_btn = gtk4::Button::builder()
                .label("Clear")
                .icon_name("edit-clear-all-symbolic")
                .build();
            clear_btn.add_css_class("destructive-action");
            clear_btn.set_tooltip_text(Some("Clear all queued tasks"));
            toolbar.append(&pause_btn);
            toolbar.append(&clear_btn);

            let remove_btn = gtk4::Button::builder()
                .label("Remove")
                .icon_name("list-remove-symbolic")
                .build();
            remove_btn.add_css_class("destructive-action");
            remove_btn.set_sensitive(false);
            remove_btn.set_tooltip_text(Some("Remove checked items from the queue"));
            toolbar.append(&remove_btn);

            let start_btn = gtk4::Button::builder()
                .label("Start Queue")
                .icon_name("media-playback-start-symbolic")
                .build();
            start_btn.add_css_class("suggested-action");
            start_btn.set_tooltip_text(Some("Start the queue"));

            queue_header.append(&queue_title);
            queue_header.append(&queue_count_label);
            queue_header.append(&header_spacer);
            queue_header.append(&add_images_btn);
            queue_header.append(&toolbar);
            queue_header.append(&start_btn);

            let queue_list = gtk4::ListBox::new();
            queue_list.add_css_class("boxed-list");
            queue_list.set_selection_mode(gtk4::SelectionMode::Single);

            let scrolled = gtk4::ScrolledWindow::new();
            scrolled.set_vexpand(true);
            scrolled.set_child(Some(&queue_list));

            let queue_empty_status = libadwaita::StatusPage::builder()
                .icon_name("document-open-recent-symbolic")
                .title("Queue is empty")
                .description("Drag images here, use the + button, or Add Selected from the viewer")
                .build();
            queue_empty_status.set_visible(false);

            let queue_overlay = gtk4::Overlay::new();
            queue_overlay.set_child(Some(&scrolled));
            queue_overlay.add_overlay(&queue_empty_status);

            let queue_section = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            queue_section.append(&queue_header);
            queue_section.append(&queue_overlay);

            {
                let widget_weak = widget.downgrade();
                let drop_target = gtk4::DropTarget::new(
                    gdk4::FileList::static_type(),
                    gdk4::DragAction::COPY | gdk4::DragAction::MOVE,
                );
                drop_target.connect_drop(move |_, value, _, _| {
                    let Ok(file_list) = value.get::<gdk4::FileList>() else {
                        return false;
                    };
                    let mut paths = Vec::new();
                    if let Some(w) = widget_weak.upgrade() {
                        for file in file_list.files() {
                            if let Some(path) = file.path() {
                                paths.push(path);
                            }
                        }
                        return w.add_paths_to_queue(paths);
                    }
                    false
                });
                queue_overlay.add_controller(drop_target);
            }
            {
                let widget_weak = widget.downgrade();
                let drop_target = gtk4::DropTarget::new(glib::Type::STRING, gdk4::DragAction::COPY);
                drop_target.connect_drop(move |_, value, _, _| {
                    let Ok(uri_list) = value.get::<String>() else {
                        return false;
                    };
                    let mut paths = Vec::new();
                    if let Some(w) = widget_weak.upgrade() {
                        for entry in uri_list.lines().map(str::trim) {
                            if entry.is_empty() || entry.starts_with('#') {
                                continue;
                            }
                            let path = if entry.contains("://") {
                                gio::File::for_uri(entry).path()
                            } else {
                                Some(PathBuf::from(entry))
                            };
                            if let Some(path) = path {
                                paths.push(path);
                            }
                        }
                        return w.add_paths_to_queue(paths);
                    }
                    false
                });
                queue_overlay.add_controller(drop_target);
            }

            // --- History section ---
            let history_section = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            history_section.set_margin_top(8);

            let history_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            history_header.set_hexpand(true);

            let history_title = gtk4::Label::new(Some("History"));
            history_title.add_css_class("heading");
            history_title.set_hexpand(true);
            history_title.set_halign(gtk4::Align::Start);

            let clear_history_btn = gtk4::Button::with_label("Clear");
            clear_history_btn.add_css_class("flat");
            clear_history_btn.set_tooltip_text(Some("Clear all history"));

            history_header.append(&history_title);
            history_header.append(&clear_history_btn);
            history_section.append(&history_header);

            let history_list = gtk4::ListBox::new();
            history_list.add_css_class("boxed-list");
            history_list.set_selection_mode(gtk4::SelectionMode::Single);

            let history_scroll = gtk4::ScrolledWindow::new();
            history_scroll.set_vexpand(true);
            history_scroll.set_child(Some(&history_list));
            history_section.append(&history_scroll);
            history_section.set_vexpand(true);

            // Hidden until there are history entries
            history_section.set_visible(false);

            let queue_history_paned = gtk4::Paned::new(gtk4::Orientation::Vertical);
            queue_history_paned.set_vexpand(true);
            queue_history_paned.set_wide_handle(false);
            queue_history_paned.set_shrink_start_child(false);
            queue_history_paned.set_shrink_end_child(false);
            queue_history_paned.set_start_child(Some(&queue_section));
            queue_history_paned.set_end_child(Some(&history_section));
            left_col.append(&queue_history_paned);

            // --- Right Column ---
            let right_col = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            right_col.set_width_request(340);
            right_col.set_hexpand(false);

            let right_header = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            right_header.set_margin_top(16);
            right_header.set_margin_bottom(8);
            right_header.set_margin_start(16);
            right_header.set_margin_end(16);

            let right_title = gtk4::Label::new(Some("Selected Item Settings"));
            right_title.add_css_class("title-4");
            right_title.set_halign(gtk4::Align::Start);
            right_header.append(&right_title);

            let right_subtitle =
                gtk4::Label::new(Some("Changes apply only to the selected queue item."));
            right_subtitle.add_css_class("dim-label");
            right_subtitle.add_css_class("caption");
            right_subtitle.set_halign(gtk4::Align::Start);
            right_header.append(&right_subtitle);

            right_col.append(&right_header);
            right_col.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

            let no_selection_label = gtk4::Label::new(Some("Select a queue item to configure it"));
            no_selection_label.add_css_class("dim-label");
            no_selection_label.set_margin_top(32);
            no_selection_label.set_halign(gtk4::Align::Center);
            right_col.append(&no_selection_label);

            let right_scroll = gtk4::ScrolledWindow::new();
            right_scroll.set_vexpand(true);
            right_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
            right_scroll.set_visible(false);

            let right_settings_box = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
            right_settings_box.set_margin_top(12);
            right_settings_box.set_margin_bottom(12);
            right_settings_box.set_margin_start(16);
            right_settings_box.set_margin_end(16);

            let upscale_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            upscale_header.set_hexpand(true);

            let upscale_icon = gtk4::Image::from_icon_name("applications-graphics-symbolic");
            upscale_icon.add_css_class("accent");
            upscale_header.append(&upscale_icon);

            let upscale_label = gtk4::Label::new(Some("Upscale"));
            upscale_label.add_css_class("heading");
            upscale_label.set_halign(gtk4::Align::Start);
            upscale_label.set_hexpand(true);
            upscale_header.append(&upscale_label);

            let upscale_toggle = gtk4::Switch::new();
            upscale_toggle.set_valign(gtk4::Align::Center);
            upscale_toggle.set_tooltip_text(Some("Enable AI upscaling for this item"));
            upscale_header.append(&upscale_toggle);
            right_settings_box.append(&upscale_header);

            let upscale_settings_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            upscale_settings_box.set_visible(false);

            let upscale_group = libadwaita::PreferencesGroup::new();

            let backend_row = libadwaita::ActionRow::new();
            backend_row.set_title("Backend");
            let backend_switcher = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            backend_switcher.add_css_class("linked");
            backend_switcher.set_valign(gtk4::Align::Center);
            let backend_onnx_btn = gtk4::ToggleButton::with_label("ONNX");
            backend_onnx_btn.set_tooltip_text(Some(
                "Run upscaling locally using ONNX models (no GPU server required)",
            ));
            let backend_comfyui_btn = gtk4::ToggleButton::with_label("ComfyUI");
            backend_comfyui_btn.set_tooltip_text(Some("Run upscaling via a local ComfyUI server"));
            let backend_cli_btn = gtk4::ToggleButton::with_label("External CLI");
            backend_cli_btn.set_tooltip_text(Some(
                "Run upscaling via the external realesrgan-ncnn-vulkan CLI tool",
            ));
            backend_switcher.append(&backend_onnx_btn);
            backend_switcher.append(&backend_comfyui_btn);
            backend_switcher.append(&backend_cli_btn);
            backend_row.add_suffix(&backend_switcher);
            upscale_group.add(&backend_row);

            let scale_row = libadwaita::ComboRow::new();
            scale_row.set_title("Scale");
            scale_row.set_subtitle("Uses AI to determine the best output size");
            scale_row.set_tooltip_text(Some(
                "Output resolution multiplier. Smart scale uses AI to pick the best size.",
            ));
            let scale_model = gtk4::StringList::new(&["Smart scale", "2×", "3×", "4×"]);
            scale_row.set_model(Some(&scale_model));
            let scale_dropdown = scale_row.clone();

            upscale_group.add(&scale_row);

            let onnx_model_row = libadwaita::ComboRow::new();
            onnx_model_row.set_title("Model");
            onnx_model_row.set_tooltip_text(Some(
                "ONNX model to use. Larger models are slower but may produce better results.",
            ));
            let onnx_model_list = gtk4::StringList::new(&[
                "Lightweight ×2 — 8 MB",
                "Compressed ×4 — 55 MB",
                "Realworld ×4 — 53 MB",
            ]);
            onnx_model_row.set_model(Some(&onnx_model_list));
            let onnx_model_dropdown = onnx_model_row.clone();
            upscale_group.add(&onnx_model_row);

            let comfyui_workflow_row = libadwaita::ComboRow::new();
            comfyui_workflow_row.set_title("Workflow");
            comfyui_workflow_row.set_tooltip_text(Some("ComfyUI workflow to use for upscaling"));
            comfyui_workflow_row.set_model(Some(&gtk4::StringList::new(&["ESRGAN", "SeedVR2"])));
            comfyui_workflow_row.set_visible(false);
            upscale_group.add(&comfyui_workflow_row);

            upscale_settings_box.append(&upscale_group);
            right_settings_box.append(&upscale_settings_box);

            right_settings_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

            let export_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            export_header.set_hexpand(true);
            export_header.set_margin_top(8);

            let export_icon = gtk4::Image::from_icon_name("document-save-as-symbolic");
            export_icon.add_css_class("success");
            export_header.append(&export_icon);

            let export_label = gtk4::Label::new(Some("Convert"));
            export_label.add_css_class("heading");
            export_label.set_halign(gtk4::Align::Start);
            export_label.set_hexpand(true);
            export_header.append(&export_label);

            let export_toggle = gtk4::Switch::new();
            export_toggle.set_valign(gtk4::Align::Center);
            export_toggle
                .set_tooltip_text(Some("Convert or compress the output to a different format"));
            export_header.append(&export_toggle);
            right_settings_box.append(&export_header);

            let export_settings_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            export_settings_box.set_visible(false);

            let export_group = libadwaita::PreferencesGroup::new();

            let export_dest_row = libadwaita::ComboRow::new();
            export_dest_row.set_title("Destination");
            let export_dest_model =
                gtk4::StringList::new(&["Default", "Same as source", "Custom folder"]);
            export_dest_row.set_model(Some(&export_dest_model));
            let export_dest_dropdown = export_dest_row.clone();
            export_group.add(&export_dest_row);

            let export_custom_row = libadwaita::ActionRow::new();
            export_custom_row.set_title("Custom folder");
            let export_custom_label = gtk4::Label::new(Some("Choose a folder"));
            export_custom_label.add_css_class("dim-label");
            let export_choose_btn = gtk4::Button::with_label("Choose…");
            export_custom_row.add_prefix(&export_custom_label);
            export_custom_row.add_suffix(&export_choose_btn);
            export_custom_row.set_visible(false);
            export_group.add(&export_custom_row);

            let export_format_row = libadwaita::ComboRow::new();
            export_format_row.set_title("Format");
            export_format_row.set_tooltip_text(Some("Output file format for converted images"));
            let export_format_model = gtk4::StringList::new(&["JXL", "WebP", "PNG", "JPEG"]);
            export_format_row.set_model(Some(&export_format_model));
            let export_format_dropdown = export_format_row.clone();
            export_group.add(&export_format_row);

            let export_edge_row = libadwaita::ComboRow::new();
            export_edge_row.set_title("Max Edge");
            export_edge_row.set_tooltip_text(Some("Limit the longest edge of the output image"));
            let export_edge_model =
                gtk4::StringList::new(&["Original", "1080px", "2160px", "4096px"]);
            export_edge_row.set_model(Some(&export_edge_model));
            let export_edge_dropdown = export_edge_row.clone();
            export_group.add(&export_edge_row);

            let export_quality_row = libadwaita::ActionRow::new();
            export_quality_row.set_title("Quality");
            export_quality_row
                .set_subtitle("Handles format conversion and compression in one step");
            let export_quality_adj = gtk4::Adjustment::new(90.0, 1.0, 100.0, 1.0, 10.0, 0.0);
            let export_quality_spin = gtk4::SpinButton::new(Some(&export_quality_adj), 1.0, 0);
            export_quality_spin.set_valign(gtk4::Align::Center);
            export_quality_spin.set_tooltip_text(Some(
                "Compression quality (1–100). Higher values preserve more detail.",
            ));
            export_quality_row.add_suffix(&export_quality_spin);
            export_quality_row.set_activatable_widget(Some(&export_quality_spin));
            export_group.add(&export_quality_row);

            export_settings_box.append(&export_group);
            right_settings_box.append(&export_settings_box);

            right_settings_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

            let summary_header = gtk4::Label::new(Some("This item will perform:"));
            summary_header.add_css_class("heading");
            summary_header.set_halign(gtk4::Align::Start);
            summary_header.set_margin_top(4);
            right_settings_box.append(&summary_header);

            let summary_action_list = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            right_settings_box.append(&summary_action_list);

            let summary_estimated_label = gtk4::Label::new(None);
            summary_estimated_label.add_css_class("dim-label");
            summary_estimated_label.add_css_class("caption");
            summary_estimated_label.set_halign(gtk4::Align::Start);
            right_settings_box.append(&summary_estimated_label);

            right_scroll.set_child(Some(&right_settings_box));
            right_col.append(&right_scroll);

            right_col.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
            let queue_defaults_group = libadwaita::PreferencesGroup::new();
            queue_defaults_group.set_title("Queue &amp; History");
            queue_defaults_group.set_margin_top(12);
            queue_defaults_group.set_margin_bottom(12);
            queue_defaults_group.set_margin_start(12);
            queue_defaults_group.set_margin_end(12);
            let history_cap_row = libadwaita::ActionRow::new();
            history_cap_row.set_title("History cap");
            history_cap_row.set_subtitle("Maximum completed and failed tasks to keep");
            let history_cap_spin = gtk4::SpinButton::with_range(10.0, 10000.0, 10.0);
            history_cap_spin.set_valign(gtk4::Align::Center);
            history_cap_row.add_suffix(&history_cap_spin);
            history_cap_row.set_activatable_widget(Some(&history_cap_spin));
            queue_defaults_group.add(&history_cap_row);
            right_col.append(&queue_defaults_group);

            main_box.append(&left_col);
            main_box.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
            main_box.append(&right_col);

            // Wire backend switcher
            {
                let comfy_btn = backend_comfyui_btn.clone();
                let cli_btn = backend_cli_btn.clone();
                let onnx_row = onnx_model_row.clone();
                let comfyui_wf_row = comfyui_workflow_row.clone();
                let widget_weak = widget.downgrade();
                backend_onnx_btn.connect_toggled(move |btn| {
                    if btn.is_active() {
                        if comfy_btn.is_active() {
                            comfy_btn.set_active(false);
                        }
                        if cli_btn.is_active() {
                            cli_btn.set_active(false);
                        }
                        onnx_row.set_visible(true);
                        comfyui_wf_row.set_visible(false);
                    } else if !comfy_btn.is_active() && !cli_btn.is_active() {
                        btn.set_active(true);
                    } else {
                        onnx_row.set_visible(false);
                    }
                    if let Some(w) = widget_weak.upgrade() {
                        w.on_pending_config_changed();
                    }
                });
            }
            {
                let onnx_btn = backend_onnx_btn.clone();
                let cli_btn = backend_cli_btn.clone();
                let onnx_row = onnx_model_row.clone();
                let comfyui_wf_row = comfyui_workflow_row.clone();
                let widget_weak = widget.downgrade();
                backend_comfyui_btn.connect_toggled(move |btn| {
                    if btn.is_active() {
                        if onnx_btn.is_active() {
                            onnx_btn.set_active(false);
                        }
                        if cli_btn.is_active() {
                            cli_btn.set_active(false);
                        }
                        onnx_row.set_visible(false);
                        comfyui_wf_row.set_visible(true);
                    } else if !onnx_btn.is_active() && !cli_btn.is_active() {
                        btn.set_active(true);
                    } else {
                        comfyui_wf_row.set_visible(false);
                    }
                    if let Some(w) = widget_weak.upgrade() {
                        w.on_pending_config_changed();
                    }
                });
            }
            {
                let onnx_btn = backend_onnx_btn.clone();
                let comfy_btn = backend_comfyui_btn.clone();
                let onnx_row = onnx_model_row.clone();
                let comfyui_wf_row = comfyui_workflow_row.clone();
                let widget_weak = widget.downgrade();
                backend_cli_btn.connect_toggled(move |btn| {
                    if btn.is_active() {
                        if onnx_btn.is_active() {
                            onnx_btn.set_active(false);
                        }
                        if comfy_btn.is_active() {
                            comfy_btn.set_active(false);
                        }
                        onnx_row.set_visible(false);
                        comfyui_wf_row.set_visible(false);
                    } else if !onnx_btn.is_active() && !comfy_btn.is_active() {
                        btn.set_active(true);
                    }
                    if let Some(w) = widget_weak.upgrade() {
                        w.on_pending_config_changed();
                    }
                });
            }

            {
                let settings_box = upscale_settings_box.clone();
                let widget_weak = widget.downgrade();
                upscale_toggle.connect_state_set(move |_sw, active| {
                    settings_box.set_visible(active);
                    if let Some(w) = widget_weak.upgrade() {
                        w.on_pending_config_changed();
                    }
                    glib::Propagation::Proceed
                });
            }

            {
                let settings_box = export_settings_box.clone();
                let widget_weak = widget.downgrade();
                export_toggle.connect_state_set(move |_sw, active| {
                    settings_box.set_visible(active);
                    if let Some(w) = widget_weak.upgrade() {
                        w.on_pending_config_changed();
                    }
                    glib::Propagation::Proceed
                });
            }

            {
                let custom_row = export_custom_row.clone();
                export_dest_dropdown.connect_selected_item_notify(move |row| {
                    custom_row.set_visible(row.selected() == 2);
                });
            }

            // Wire Start/Stop
            {
                let widget_weak = widget.downgrade();
                start_btn.connect_clicked(move |_| {
                    if let Some(w) = widget_weak.upgrade() {
                        w.commit_pending_configs_to_db();
                        w.imp().runner_active.set(true);
                        w.imp().paused.set(false);
                        w.try_start_runner();
                        w.run_next_pipeline();
                        w.refresh();
                    }
                });
            }
            {
                let widget_weak = widget.downgrade();
                pause_btn.connect_clicked(move |_| {
                    if let Some(w) = widget_weak.upgrade() {
                        w.imp().paused.set(true);
                        w.refresh();
                    }
                });
            }
            {
                let widget_weak = widget.downgrade();
                clear_btn.connect_clicked(move |_| {
                    let Some(w) = widget_weak.upgrade() else {
                        return;
                    };
                    if let Some(state_rc) = w.imp().state.borrow().as_ref() {
                        if let Some(idx) = state_rc.borrow().library_index.as_ref() {
                            for pipeline in idx
                                .pipelines_by_status(PipelineStatus::Queued)
                                .unwrap_or_default()
                            {
                                let _ = idx.delete_pipeline(pipeline.id);
                            }
                        }
                    }
                    w.refresh();
                });
            }
            {
                let widget_weak = widget.downgrade();
                remove_btn.connect_clicked(move |_| {
                    let Some(w) = widget_weak.upgrade() else {
                        return;
                    };
                    let checked: Vec<i64> =
                        w.imp().queue_checked_ids.borrow().iter().copied().collect();
                    if let Some(state_rc) = w.imp().state.borrow().as_ref() {
                        if let Some(idx) = state_rc.borrow().library_index.as_ref() {
                            for pid in &checked {
                                let _ = idx.delete_pipeline(*pid);
                                w.imp().pending_configs.borrow_mut().remove(pid);
                            }
                        }
                    }
                    w.imp().queue_checked_ids.borrow_mut().clear();
                    if let Some(btn) = w.imp().remove_btn.borrow().as_ref() {
                        btn.set_sensitive(false);
                    }
                    w.refresh();
                });
            }
            {
                let widget_weak = widget.downgrade();
                add_images_btn.connect_clicked(move |_| {
                    let Some(w) = widget_weak.upgrade() else {
                        return;
                    };
                    let parent_window = w.imp().parent_window.borrow().upgrade();
                    let dialog = gtk4::FileDialog::builder()
                        .title("Add Images to Queue")
                        .modal(true)
                        .build();

                    let filter = gtk4::FileFilter::new();
                    filter.set_name(Some("Images"));
                    for suffix in ["png", "jpg", "jpeg", "webp", "jxl", "tiff"] {
                        filter.add_suffix(suffix);
                    }
                    let filters = gio::ListStore::new::<gtk4::FileFilter>();
                    filters.append(&filter);
                    dialog.set_filters(Some(&filters));
                    dialog.set_default_filter(Some(&filter));

                    let widget_weak_inner = w.downgrade();
                    dialog.open_multiple(
                        parent_window.as_ref(),
                        None::<&gio::Cancellable>,
                        move |result| {
                            let Some(w) = widget_weak_inner.upgrade() else {
                                return;
                            };
                            if let Ok(files) = result {
                                let mut paths = Vec::new();
                                for i in 0..files.n_items() {
                                    if let Some(file) = files.item(i).and_downcast::<gio::File>() {
                                        if let Some(path) = file.path() {
                                            paths.push(path);
                                        }
                                    }
                                }
                                w.add_paths_to_queue(paths);
                            }
                        },
                    );
                });
            }

            {
                let widget_weak = widget.downgrade();
                export_choose_btn.connect_clicked(move |_| {
                    let Some(w) = widget_weak.upgrade() else {
                        return;
                    };
                    w.choose_custom_destination();
                });
            }

            {
                let widget_weak = widget.downgrade();
                scale_dropdown.connect_selected_item_notify(move |dd| {
                    dd.set_subtitle(if dd.selected() == 0 {
                        "Uses AI to determine the best output size."
                    } else {
                        ""
                    });
                    if let Some(w) = widget_weak.upgrade() {
                        w.on_pending_config_changed();
                    }
                });
            }
            {
                let widget_weak = widget.downgrade();
                onnx_model_dropdown.connect_selected_item_notify(move |_| {
                    if let Some(w) = widget_weak.upgrade() {
                        w.on_pending_config_changed();
                    }
                });
            }
            {
                let widget_weak = widget.downgrade();
                comfyui_workflow_row.connect_selected_item_notify(move |_| {
                    if let Some(w) = widget_weak.upgrade() {
                        w.on_pending_config_changed();
                    }
                });
            }
            {
                let widget_weak = widget.downgrade();
                export_format_dropdown.connect_selected_item_notify(move |_| {
                    if let Some(w) = widget_weak.upgrade() {
                        w.on_pending_config_changed();
                    }
                });
            }
            {
                let widget_weak = widget.downgrade();
                export_quality_spin.connect_value_changed(move |_| {
                    if let Some(w) = widget_weak.upgrade() {
                        w.on_pending_config_changed();
                    }
                });
            }
            {
                let widget_weak = widget.downgrade();
                export_dest_dropdown.connect_selected_item_notify(move |_| {
                    if let Some(w) = widget_weak.upgrade() {
                        w.on_pending_config_changed();
                    }
                });
            }

            {
                let widget_weak = widget.downgrade();
                clear_history_btn.connect_clicked(move |_| {
                    let Some(w) = widget_weak.upgrade() else {
                        return;
                    };
                    if let Some(state_rc) = w.imp().state.borrow().as_ref() {
                        if let Some(idx) = state_rc.borrow().library_index.as_ref() {
                            let _ = idx.clear_pipeline_history();
                        }
                    }
                    w.refresh();
                });
            }

            {
                let widget_weak = widget.downgrade();
                crash_banner.connect_button_clicked(move |_| {
                    if let Some(w) = widget_weak.upgrade() {
                        if let Some(b) = w.imp().crash_banner.borrow().as_ref() {
                            b.set_revealed(false);
                        }
                        w.imp().runner_active.set(true);
                        w.imp().paused.set(false);
                        w.try_start_runner();
                        w.run_next_pipeline();
                    }
                });
            }

            *self.queue_list.borrow_mut() = Some(queue_list);
            *self.queue_empty_status.borrow_mut() = Some(queue_empty_status);
            *self.start_btn.borrow_mut() = Some(start_btn);
            *self.pause_btn.borrow_mut() = Some(pause_btn);
            *self.clear_btn.borrow_mut() = Some(clear_btn);
            *self.add_images_btn.borrow_mut() = Some(add_images_btn);
            *self.remove_btn.borrow_mut() = Some(remove_btn);
            *self.queue_count_label.borrow_mut() = Some(queue_count_label);
            *self.background_status_label.borrow_mut() = Some(background_status_label);
            *self.background_empty_label.borrow_mut() = Some(background_empty_label);
            *self.background_list.borrow_mut() = Some(background_list);
            *self.crash_banner.borrow_mut() = Some(crash_banner);

            *self.upscale_toggle.borrow_mut() = Some(upscale_toggle);
            *self.upscale_settings_box.borrow_mut() = Some(upscale_settings_box);
            *self.backend_onnx_btn.borrow_mut() = Some(backend_onnx_btn);
            *self.backend_comfyui_btn.borrow_mut() = Some(backend_comfyui_btn);
            *self.backend_cli_btn.borrow_mut() = Some(backend_cli_btn);
            *self.onnx_model_dropdown.borrow_mut() = Some(onnx_model_dropdown);
            *self.onnx_model_row.borrow_mut() = Some(onnx_model_row);
            *self.comfyui_workflow_row.borrow_mut() = Some(comfyui_workflow_row);
            *self.scale_dropdown.borrow_mut() = Some(scale_dropdown);

            *self.export_toggle.borrow_mut() = Some(export_toggle);
            *self.export_settings_box.borrow_mut() = Some(export_settings_box);
            *self.export_format_dropdown.borrow_mut() = Some(export_format_dropdown);
            *self.export_edge_dropdown.borrow_mut() = Some(export_edge_dropdown);
            *self.export_quality_spin.borrow_mut() = Some(export_quality_spin);
            *self.export_dest_dropdown.borrow_mut() = Some(export_dest_dropdown);
            *self.export_custom_row.borrow_mut() = Some(export_custom_row);
            *self.export_custom_label.borrow_mut() = Some(export_custom_label);
            *self.history_cap_spin.borrow_mut() = Some(history_cap_spin);

            *self.right_scroll.borrow_mut() = Some(right_scroll);
            *self.right_settings_box.borrow_mut() = Some(right_settings_box);
            *self.no_selection_label.borrow_mut() = Some(no_selection_label);
            *self.summary_action_list.borrow_mut() = Some(summary_action_list);
            *self.summary_estimated_label.borrow_mut() = Some(summary_estimated_label);

            *self.history_list.borrow_mut() = Some(history_list);
            *self.clear_history_btn.borrow_mut() = Some(clear_history_btn);
            *self.history_section.borrow_mut() = Some(history_section);
        }

        fn dispose(&self) {
            if let Some(source_id) = self.polling_timer.borrow_mut().take() {
                source_id.remove();
            }
            let widget = self.obj();
            while let Some(child) = widget.first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for TasksPage {}
}

glib::wrapper! {
    pub struct TasksPage(ObjectSubclass<imp::TasksPage>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl TasksPage {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_parent_window(&self, window: &gtk4::Window) {
        self.imp().parent_window.borrow_mut().set(Some(window));
    }

    fn update_custom_destination_labels(&self) {
        let imp = self.imp();
        if let Some(label) = imp.export_custom_label.borrow().as_ref() {
            label.set_text(
                &imp.export_custom_path
                    .borrow()
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "Choose a folder".to_string()),
            );
        }
    }

    fn choose_custom_destination(&self) {
        let parent_window = self.imp().parent_window.borrow().upgrade();
        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Choose Export Output Folder");

        let widget_weak = self.downgrade();
        dialog.select_folder(
            parent_window.as_ref(),
            None::<&gio::Cancellable>,
            move |result| {
                let Some(widget) = widget_weak.upgrade() else {
                    return;
                };
                let Ok(file) = result else {
                    return;
                };
                let Some(path) = file.path() else {
                    return;
                };
                *widget.imp().export_custom_path.borrow_mut() = Some(path.clone());
                if let Some(state_rc) = widget.imp().state.borrow().as_ref() {
                    state_rc
                        .borrow_mut()
                        .settings
                        .set_export_output_dir(Some(path));
                }
                widget.update_custom_destination_labels();
                widget.on_pending_config_changed();
            },
        );
    }

    fn clear_summary(&self) {
        let imp = self.imp();
        if let Some(box_) = imp.summary_action_list.borrow().as_ref() {
            while let Some(child) = box_.first_child() {
                child.unparent();
            }
        }
        if let Some(lbl) = imp.summary_estimated_label.borrow().as_ref() {
            lbl.set_text("");
        }
    }

    fn update_summary_for_config(&self, config: &PendingConfig) {
        let imp = self.imp();
        let Some(action_list) = imp.summary_action_list.borrow().clone() else {
            return;
        };
        while let Some(child) = action_list.first_child() {
            child.unparent();
        }
        if !config.upscale_on && !config.export_on {
            let lbl = gtk4::Label::new(Some("No actions configured"));
            lbl.add_css_class("dim-label");
            lbl.add_css_class("caption");
            lbl.set_halign(gtk4::Align::Start);
            action_list.append(&lbl);
        }
        if config.upscale_on {
            let scale_str = match config.upscale.scale {
                2 => "2×",
                4 => "4×",
                _ => "Smart scale",
            };
            let backend_label = match config.upscale.backend.as_str() {
                "comfyui" => "ComfyUI",
                "cli" => "External CLI",
                _ => "ONNX",
            };
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            let dot = gtk4::Label::new(Some("●"));
            dot.add_css_class("accent");
            dot.add_css_class("caption");
            row.append(&dot);
            let text = gtk4::Label::new(Some(&format!(
                "Upscale using {} · {}",
                backend_label, scale_str
            )));
            text.add_css_class("caption");
            text.set_halign(gtk4::Align::Start);
            row.append(&text);
            action_list.append(&row);
        }
        if config.export_on {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            let dot = gtk4::Label::new(Some("●"));
            dot.add_css_class("success");
            dot.add_css_class("caption");
            row.append(&dot);
            let format_upper = config.export.format.to_uppercase();
            let text = gtk4::Label::new(Some(&format!(
                "Convert / Export as {} · Q{}",
                format_upper, config.export.quality
            )));
            text.add_css_class("caption");
            text.set_halign(gtk4::Align::Start);
            row.append(&text);
            action_list.append(&row);
        }
        if let Some(lbl) = imp.summary_estimated_label.borrow().as_ref() {
            lbl.set_text("");
        }
    }

    fn first_step(steps: &[PipelineStep]) -> Option<&PipelineStep> {
        steps.iter().min_by_key(|step| step.step_order)
    }

    fn active_step(steps: &[PipelineStep]) -> Option<&PipelineStep> {
        steps
            .iter()
            .find(|step| step.status == PipelineStatus::InProgress)
    }

    fn latest_completed_step(steps: &[PipelineStep]) -> Option<&PipelineStep> {
        steps
            .iter()
            .filter(|step| step.status == PipelineStatus::Completed)
            .max_by_key(|step| step.step_order)
    }

    fn latest_output_step(steps: &[PipelineStep]) -> Option<&PipelineStep> {
        steps
            .iter()
            .filter(|step| step.output_path.is_some())
            .max_by_key(|step| step.step_order)
    }

    fn failed_step(steps: &[PipelineStep]) -> Option<&PipelineStep> {
        steps
            .iter()
            .filter(|step| step.status == PipelineStatus::Failed)
            .max_by_key(|step| step.step_order)
    }

    pub fn set_interrupted_count(&self, n: usize) {
        if n == 0 {
            return;
        }
        let imp = self.imp();
        if let Some(banner) = imp.crash_banner.borrow().as_ref() {
            let msg = if n == 1 {
                "1 job was interrupted and re-queued".to_string()
            } else {
                format!("{} jobs were interrupted and re-queued", n)
            };
            banner.set_title(&msg);
            banner.set_revealed(true);
        }
    }

    fn selected_backend(&self) -> &'static str {
        let imp = self.imp();
        if imp
            .backend_cli_btn
            .borrow()
            .as_ref()
            .map(|btn| btn.is_active())
            .unwrap_or(false)
        {
            "cli"
        } else if imp
            .backend_comfyui_btn
            .borrow()
            .as_ref()
            .map(|btn| btn.is_active())
            .unwrap_or(false)
        {
            "comfyui"
        } else {
            "onnx"
        }
    }

    fn set_backend(&self, backend: &str) {
        let imp = self.imp();
        if let Some(btn) = imp.backend_onnx_btn.borrow().as_ref() {
            btn.set_active(backend == "onnx");
        }
        if let Some(btn) = imp.backend_comfyui_btn.borrow().as_ref() {
            btn.set_active(backend == "comfyui");
        }
        if let Some(btn) = imp.backend_cli_btn.borrow().as_ref() {
            btn.set_active(backend == "cli");
        }
        if let Some(row) = imp.onnx_model_row.borrow().as_ref() {
            row.set_visible(backend == "onnx");
        }
        if let Some(row) = imp.comfyui_workflow_row.borrow().as_ref() {
            row.set_visible(backend == "comfyui");
        }
    }

    fn load_settings_for_pipeline(&self, pipeline_id: i64) {
        *self.imp().selected_pipeline_id.borrow_mut() = Some(pipeline_id);
        self.imp().selected_is_history.set(false);
        if let Some(scroll) = self.imp().right_scroll.borrow().as_ref() {
            scroll.set_visible(true);
        }
        if let Some(label) = self.imp().no_selection_label.borrow().as_ref() {
            label.set_visible(false);
        }

        let config = self.get_or_init_pending_config(pipeline_id);
        self.populate_panel_from_config(&config);
        self.update_summary_for_config(&config);
    }

    fn get_or_init_pending_config(&self, pipeline_id: i64) -> PendingConfig {
        if let Some(c) = self
            .imp()
            .pending_configs
            .borrow()
            .get(&pipeline_id)
            .cloned()
        {
            return c;
        }
        let config = self.build_pending_config_from_db(pipeline_id);
        self.imp()
            .pending_configs
            .borrow_mut()
            .insert(pipeline_id, config.clone());
        config
    }

    fn build_pending_config_from_db(&self, pipeline_id: i64) -> PendingConfig {
        let Some(state_rc) = self.imp().state.borrow().clone() else {
            return PendingConfig::default();
        };
        let state = state_rc.borrow();
        let Some(idx) = state.library_index.as_ref() else {
            return PendingConfig::default();
        };
        let steps = idx.steps_for_pipeline(pipeline_id).unwrap_or_default();
        let mut config = PendingConfig {
            upscale_on: false,
            export_on: false,
            ..PendingConfig::default()
        };
        for step in &steps {
            match step.step_type {
                StepType::Upscale => {
                    if let Ok(s) = serde_json::from_str::<UpscaleStepSettings>(&step.settings_json)
                    {
                        config.upscale_on = true;
                        config.upscale = s;
                    }
                }
                StepType::Export => {
                    if let Ok(s) = serde_json::from_str::<ExportStepSettings>(&step.settings_json) {
                        config.export_on = true;
                        config.export = s;
                    }
                }
            }
        }
        if steps.is_empty() {
            config.upscale_on = true;
        }
        config
    }

    fn populate_panel_from_config(&self, config: &PendingConfig) {
        let imp = self.imp();

        if let Some(sw) = imp.upscale_toggle.borrow().as_ref() {
            sw.set_active(config.upscale_on);
        }
        if let Some(box_) = imp.upscale_settings_box.borrow().as_ref() {
            box_.set_visible(config.upscale_on);
        }
        self.set_backend(&config.upscale.backend);
        if let Some(d) = imp.scale_dropdown.borrow().as_ref() {
            let selected = match config.upscale.scale {
                2 => 1,
                3 => 2,
                4 => 3,
                _ => 0,
            };
            d.set_selected(selected);
            d.set_subtitle(if selected == 0 {
                "Uses AI to determine the best output size."
            } else {
                ""
            });
        }
        if let Some(d) = imp.onnx_model_dropdown.borrow().as_ref() {
            d.set_selected(match config.upscale.onnx_model.as_deref() {
                Some("swin2sr-compressed-x4") => 1,
                Some("swin2sr-real-x4") => 2,
                _ => 0,
            });
        }
        if let Some(row) = imp.comfyui_workflow_row.borrow().as_ref() {
            row.set_selected(match config.upscale.comfyui_workflow.as_deref() {
                Some("seedvr2") => 1,
                _ => 0,
            });
        }

        if let Some(sw) = imp.export_toggle.borrow().as_ref() {
            sw.set_active(config.export_on);
        }
        if let Some(box_) = imp.export_settings_box.borrow().as_ref() {
            box_.set_visible(config.export_on);
        }
        if let Some(d) = imp.export_format_dropdown.borrow().as_ref() {
            d.set_selected(match config.export.format.as_str() {
                "webp" => 1,
                "png" => 2,
                "jpeg" => 3,
                _ => 0,
            });
        }
        if let Some(d) = imp.export_edge_dropdown.borrow().as_ref() {
            d.set_selected(match config.export.max_edge {
                Some(1080) => 1,
                Some(2160) => 2,
                Some(4096) => 3,
                _ => 0,
            });
        }
        if let Some(s) = imp.export_quality_spin.borrow().as_ref() {
            s.set_value(config.export.quality as f64);
        }
        if let Some(d) = imp.export_dest_dropdown.borrow().as_ref() {
            d.set_selected(match config.export.destination.as_str() {
                "source" => 1,
                "custom" => 2,
                _ => 0,
            });
        }
        *imp.export_custom_path.borrow_mut() = config.export.custom_path.clone();
        self.update_custom_destination_labels();
    }

    fn on_pending_config_changed(&self) {
        let Some(pid) = *self.imp().selected_pipeline_id.borrow() else {
            return;
        };
        if self.imp().selected_is_history.get() {
            return;
        }
        let config = self.read_config_from_widgets();
        self.imp()
            .pending_configs
            .borrow_mut()
            .insert(pid, config.clone());
        self.update_summary_for_config(&config);
        self.refresh_chips_for_pipeline(pid);
    }

    fn read_config_from_widgets(&self) -> PendingConfig {
        let imp = self.imp();
        let upscale_on = imp
            .upscale_toggle
            .borrow()
            .as_ref()
            .map(|s| s.is_active())
            .unwrap_or(false);
        let export_on = imp
            .export_toggle
            .borrow()
            .as_ref()
            .map(|s| s.is_active())
            .unwrap_or(false);

        let backend = self.selected_backend().to_string();
        let onnx_model = if backend == "onnx" {
            imp.onnx_model_dropdown
                .borrow()
                .as_ref()
                .map(|d| match d.selected() {
                    1 => "swin2sr-compressed-x4",
                    2 => "swin2sr-real-x4",
                    _ => "swin2sr-lightweight-x2",
                })
                .map(|s| s.to_string())
        } else {
            None
        };
        let comfyui_workflow = if backend == "comfyui" {
            imp.comfyui_workflow_row
                .borrow()
                .as_ref()
                .map(|r| match r.selected() {
                    1 => "seedvr2",
                    _ => "esrgan",
                })
                .map(|s| s.to_string())
        } else {
            None
        };
        let scale = imp
            .scale_dropdown
            .borrow()
            .as_ref()
            .map(|d| match d.selected() {
                1 => 2u32,
                2 => 3,
                3 => 4,
                _ => 0,
            })
            .unwrap_or(0);
        let model = imp
            .state
            .borrow()
            .as_ref()
            .map(|s| s.borrow().settings.upscaler_default_model.clone())
            .unwrap_or_default();

        let export_format = imp
            .export_format_dropdown
            .borrow()
            .as_ref()
            .map(|d| match d.selected() {
                1 => "webp",
                2 => "png",
                3 => "jpeg",
                _ => "jxl",
            })
            .unwrap_or("jxl")
            .to_string();
        let max_edge =
            imp.export_edge_dropdown
                .borrow()
                .as_ref()
                .and_then(|d| match d.selected() {
                    1 => Some(1080u32),
                    2 => Some(2160),
                    3 => Some(4096),
                    _ => None,
                });
        let export_quality = imp
            .export_quality_spin
            .borrow()
            .as_ref()
            .map(|s| s.value() as u8)
            .unwrap_or(90);
        let export_dest = imp
            .export_dest_dropdown
            .borrow()
            .as_ref()
            .map(|d| match d.selected() {
                1 => "source",
                2 => "custom",
                _ => "default",
            })
            .unwrap_or("default")
            .to_string();

        PendingConfig {
            upscale_on,
            upscale: UpscaleStepSettings {
                backend,
                model,
                onnx_model,
                scale,
                compress: false,
                format: "png".to_string(),
                quality: 85,
                keep_png: false,
                destination: "default".to_string(),
                custom_path: None,
                comfyui_workflow,
            },
            export_on,
            export: ExportStepSettings {
                format: export_format,
                max_edge,
                quality: export_quality,
                destination: export_dest,
                custom_path: imp.export_custom_path.borrow().clone(),
            },
        }
    }

    fn commit_pending_configs_to_db(&self) {
        let Some(state_rc) = self.imp().state.borrow().clone() else {
            return;
        };
        let state = state_rc.borrow();
        let Some(idx) = state.library_index.as_ref() else {
            return;
        };
        let queued = idx
            .pipelines_by_status(PipelineStatus::Queued)
            .unwrap_or_default();
        for pipeline in queued {
            let steps = idx.steps_for_pipeline(pipeline.id).unwrap_or_default();
            if !steps.is_empty() {
                continue;
            }
            let config = self
                .imp()
                .pending_configs
                .borrow()
                .get(&pipeline.id)
                .cloned()
                .unwrap_or_else(|| self.read_config_from_widgets());
            if config.upscale_on {
                let json = serde_json::to_string(&config.upscale).unwrap_or_default();
                let _ = idx.append_pipeline_step(pipeline.id, StepType::Upscale, &json);
            }
            if config.export_on {
                let json = serde_json::to_string(&config.export).unwrap_or_default();
                let _ = idx.append_pipeline_step(pipeline.id, StepType::Export, &json);
            }
        }
    }

    fn refresh_chips_for_pipeline(&self, pipeline_id: i64) {
        let Some(queue_list) = self.imp().queue_list.borrow().clone() else {
            return;
        };
        let pipeline_name = pipeline_id.to_string();
        let mut child = queue_list.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            let Some(expander) = widget
                .clone()
                .downcast::<libadwaita::ExpanderRow>()
                .ok()
                .or_else(|| {
                    widget
                        .downcast::<gtk4::ListBoxRow>()
                        .ok()
                        .and_then(|row| row.child())
                        .and_then(|child| child.downcast::<libadwaita::ExpanderRow>().ok())
                })
            else {
                child = next;
                continue;
            };

            if expander.widget_name() != pipeline_name {
                child = next;
                continue;
            }

            if let Some(chips) = self
                .imp()
                .queue_chip_suffixes
                .borrow_mut()
                .remove(&pipeline_id)
            {
                expander.remove(&chips);
            }
            let config = self
                .imp()
                .pending_configs
                .borrow()
                .get(&pipeline_id)
                .cloned()
                .unwrap_or_default();
            let chips = Self::build_action_chips_from_config(&config);
            expander.add_suffix(&chips);
            self.imp()
                .queue_chip_suffixes
                .borrow_mut()
                .insert(pipeline_id, chips);
            return;
        }
    }

    pub fn set_state(&self, state: Rc<RefCell<AppState>>) {
        let imp = self.imp();

        {
            let st = state.borrow();
            self.set_backend(st.settings.upscale_backend.as_str());
            if let Some(onnx_dd) = imp.onnx_model_dropdown.borrow().as_ref() {
                let idx = match st.settings.onnx_upscale_model.as_str() {
                    "swin2sr-compressed-x4" => 1,
                    "swin2sr-real-x4" => 2,
                    _ => 0,
                };
                onnx_dd.set_selected(idx);
            }
            if let Some(row) = imp.comfyui_workflow_row.borrow().as_ref() {
                let idx = match st.settings.comfyui_workflow.as_str() {
                    "seedvr2" => 1,
                    _ => 0,
                };
                row.set_selected(idx);
            }
            if let Some(scale_dd) = imp.scale_dropdown.borrow().as_ref() {
                scale_dd.set_selected(0);
            }
            if let Some(upscale_toggle) = imp.upscale_toggle.borrow().as_ref() {
                upscale_toggle.set_active(true);
            }
            if let Some(box_) = imp.upscale_settings_box.borrow().as_ref() {
                box_.set_visible(true);
            }
            if let Some(export_toggle) = imp.export_toggle.borrow().as_ref() {
                export_toggle.set_active(false);
            }
            if let Some(box_) = imp.export_settings_box.borrow().as_ref() {
                box_.set_visible(false);
            }
            if let Some(format_dd) = imp.export_format_dropdown.borrow().as_ref() {
                format_dd.set_selected(0);
            }
            if let Some(edge_dd) = imp.export_edge_dropdown.borrow().as_ref() {
                edge_dd.set_selected(0);
            }
            if let Some(spin) = imp.export_quality_spin.borrow().as_ref() {
                spin.set_value(90.0);
            }
            if let Some(dest_dd) = imp.export_dest_dropdown.borrow().as_ref() {
                dest_dd.set_selected(0);
            }
            *imp.export_custom_path.borrow_mut() = st.settings.export_output_dir.clone();
            if let Some(spin) = imp.history_cap_spin.borrow().as_ref() {
                spin.set_value(st.settings.pipeline_history_cap as f64);
            }
        }
        self.update_custom_destination_labels();

        for backend in ["onnx", "comfyui", "cli"] {
            let button = match backend {
                "comfyui" => imp.backend_comfyui_btn.borrow().clone(),
                "cli" => imp.backend_cli_btn.borrow().clone(),
                _ => imp.backend_onnx_btn.borrow().clone(),
            };
            if let Some(btn) = button {
                let state_c = state.clone();
                btn.connect_toggled(move |btn| {
                    if !btn.is_active() {
                        return;
                    }
                    if let Ok(mut st) = state_c.try_borrow_mut() {
                        st.settings.set_upscale_backend(backend);
                    }
                });
            }
        }

        if let Some(onnx_dd) = imp.onnx_model_dropdown.borrow().as_ref() {
            let state_c = state.clone();
            onnx_dd.connect_selected_item_notify(move |dd| {
                let key = match dd.selected() {
                    1 => "swin2sr-compressed-x4",
                    2 => "swin2sr-real-x4",
                    _ => "swin2sr-lightweight-x2",
                };
                if let Ok(mut st) = state_c.try_borrow_mut() {
                    st.settings.set_onnx_upscale_model(key);
                }
            });
        }

        if let Some(row) = imp.comfyui_workflow_row.borrow().as_ref() {
            let state_c = state.clone();
            row.connect_selected_item_notify(move |row| {
                let key = match row.selected() {
                    1 => "seedvr2",
                    _ => "esrgan",
                };
                if let Ok(mut st) = state_c.try_borrow_mut() {
                    st.settings.set_comfyui_workflow(key);
                }
            });
        }

        if let Some(history_cap_spin) = imp.history_cap_spin.borrow().as_ref() {
            let state_c = state.clone();
            history_cap_spin.connect_value_changed(move |spin| {
                if let Ok(mut st) = state_c.try_borrow_mut() {
                    st.settings.set_pipeline_history_cap(spin.value() as i32);
                }
            });
        }

        *imp.state.borrow_mut() = Some(state.clone());

        if let Some(queue_list) = imp.queue_list.borrow().as_ref() {
            let widget_weak = self.downgrade();
            let history_list = imp.history_list.borrow().clone();
            let handler = queue_list.connect_row_selected(move |_, row| {
                let Some(widget) = widget_weak.upgrade() else {
                    return;
                };
                let Some(row) = row else {
                    return;
                };
                if let Ok(id) = row.widget_name().parse::<i64>() {
                    if let Some(history_list) = history_list.as_ref() {
                        history_list.unselect_all();
                    }
                    widget.load_settings_for_pipeline(id);
                }
            });
            *imp.queue_row_selected_handler.borrow_mut() = Some(handler);
        }

        if let Some(history_list) = imp.history_list.borrow().as_ref() {
            let widget_weak = self.downgrade();
            let queue_list = imp.queue_list.borrow().clone();
            let handler = history_list.connect_row_selected(move |_, row| {
                let Some(widget) = widget_weak.upgrade() else {
                    return;
                };
                let Some(row) = row else {
                    return;
                };
                if let Ok(id) = row.widget_name().parse::<i64>() {
                    if let Some(queue_list) = queue_list.as_ref() {
                        queue_list.unselect_all();
                    }
                    *widget.imp().selected_pipeline_id.borrow_mut() = Some(id);
                    widget.imp().selected_is_history.set(true);
                    widget.clear_summary();
                    if let Some(scroll) = widget.imp().right_scroll.borrow().as_ref() {
                        scroll.set_visible(false);
                    }
                    if let Some(label) = widget.imp().no_selection_label.borrow().as_ref() {
                        label.set_visible(true);
                    }
                }
            });
            *imp.history_row_selected_handler.borrow_mut() = Some(handler);
        }

        self.refresh();

        {
            let n = state.borrow().library_index_interrupted_count;
            self.set_interrupted_count(n);
        }

        self.try_start_runner();
    }

    pub fn set_compare_requested_cb<F: Fn(CompareItem) + 'static>(&self, f: F) {
        *self.imp().compare_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_user_activity_changed_cb<F: Fn(bool) + 'static>(&self, f: F) {
        *self.imp().user_activity_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn push_background_op(&self, id: u64, title: &str) {
        let imp = self.imp();
        let Some(list_box) = imp.background_list.borrow().clone() else {
            return;
        };
        if imp.background_rows.borrow().contains_key(&id) {
            return;
        }

        let title_label = gtk4::Label::new(Some(title));
        title_label.set_halign(gtk4::Align::Start);
        title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        title_label.add_css_class("caption-heading");

        let status_label = gtk4::Label::new(Some("Running"));
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        status_label.add_css_class("dim-label");
        status_label.add_css_class("caption");

        let progress_bar = gtk4::ProgressBar::new();
        progress_bar.set_pulse_step(0.1);
        progress_bar.pulse();

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.append(&title_label);
        content.append(&status_label);
        content.append(&progress_bar);

        let row = gtk4::ListBoxRow::new();
        row.set_selectable(false);
        row.set_activatable(false);
        row.set_child(Some(&content));

        list_box.append(&row);
        imp.background_rows.borrow_mut().insert(
            id,
            BackgroundTaskRow {
                row,
                progress_bar,
                status_label,
                active: Cell::new(true),
            },
        );
        imp.background_active_count
            .set(imp.background_active_count.get().saturating_add(1));
        self.refresh_background_activity_ui();
    }

    pub fn update_background_op(&self, id: u64, fraction: Option<f32>) {
        let rows = self.imp().background_rows.borrow();
        if let Some(row) = rows.get(&id) {
            match fraction {
                Some(value) => row.progress_bar.set_fraction(value as f64),
                None => row.progress_bar.pulse(),
            }
        }
    }

    pub fn complete_background_op(&self, id: u64) {
        self.finish_background_op(id, "Done", Some("dim-label"));
    }

    pub fn fail_background_op(&self, id: u64, msg: &str) {
        self.finish_background_op(id, &format!("Failed: {msg}"), Some("error"));
    }

    pub fn remove_background_op(&self, id: u64) {
        let imp = self.imp();
        let removed = imp.background_rows.borrow_mut().remove(&id);
        if let Some(row) = removed {
            if row.active.replace(false) {
                imp.background_active_count
                    .set(imp.background_active_count.get().saturating_sub(1));
            }
            if let Some(list_box) = imp.background_list.borrow().as_ref() {
                list_box.remove(&row.row);
            }
            self.refresh_background_activity_ui();
        }
    }

    fn finish_background_op(&self, id: u64, status: &str, css_class: Option<&str>) {
        let imp = self.imp();
        let rows = imp.background_rows.borrow();
        if let Some(row) = rows.get(&id) {
            row.progress_bar.set_visible(false);
            row.status_label.set_text(status);
            row.status_label.remove_css_class("dim-label");
            row.status_label.remove_css_class("error");
            row.status_label.add_css_class("caption");
            if let Some(class_name) = css_class {
                row.status_label.add_css_class(class_name);
            }
            if row.active.replace(false) {
                imp.background_active_count
                    .set(imp.background_active_count.get().saturating_sub(1));
            }
        }
        drop(rows);
        self.refresh_background_activity_ui();

        let widget = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
            widget.remove_background_op(id);
        });
    }

    fn refresh_background_activity_ui(&self) {
        let imp = self.imp();
        let total = imp.background_rows.borrow().len();
        let active = imp.background_active_count.get();

        if let Some(list_box) = imp.background_list.borrow().as_ref() {
            list_box.set_visible(total > 0);
        }
        if let Some(empty_label) = imp.background_empty_label.borrow().as_ref() {
            empty_label.set_visible(total == 0);
        }
        if let Some(status_label) = imp.background_status_label.borrow().as_ref() {
            let status = match active {
                0 => "No background activity".to_string(),
                1 => "1 background task running".to_string(),
                n => format!("{n} background tasks running"),
            };
            status_label.set_text(&status);
        }
    }

    fn emit_user_activity(&self, active: bool) {
        if let Some(cb) = self.imp().user_activity_cb.borrow().as_ref() {
            cb(active);
        }
    }

    pub fn pre_fill_from_path(&self, path: PathBuf) {
        self.add_paths_to_queue(vec![path]);
    }

    fn add_paths_to_queue(&self, paths: Vec<PathBuf>) -> bool {
        if paths.is_empty() {
            return false;
        }
        let mut added = false;
        if let Some(state_rc) = self.imp().state.borrow().as_ref() {
            if let Some(idx) = state_rc.borrow().library_index.as_ref() {
                for path in paths {
                    if idx.create_pipeline(&path).is_ok() {
                        added = true;
                    }
                }
            }
        }
        if added {
            self.refresh_queue();
        }
        added
    }

    pub fn on_pipelines_added(&self) {
        self.refresh_queue();
    }

    fn refresh_queue(&self) {
        let imp = self.imp();
        let Some(list_box) = imp.queue_list.borrow().clone() else {
            return;
        };
        imp.queue_chip_suffixes.borrow_mut().clear();
        while let Some(child) = list_box.first_child() {
            list_box.remove(&child);
        }

        let Some(state_rc) = imp.state.borrow().clone() else {
            return;
        };
        {
            let state = state_rc.borrow();
            let Some(idx) = state.library_index.as_ref() else {
                return;
            };

            let in_progress = idx
                .pipelines_by_status(PipelineStatus::InProgress)
                .unwrap_or_default();
            let queued = idx
                .pipelines_by_status(PipelineStatus::Queued)
                .unwrap_or_default();

            let mut pipelines = in_progress;
            pipelines.extend(queued);
            let visible_queue_ids: HashSet<i64> = pipelines.iter().map(|p| p.id).collect();
            imp.queue_checked_ids
                .borrow_mut()
                .retain(|pid| visible_queue_ids.contains(pid));

            if let Some(status) = imp.queue_empty_status.borrow().as_ref() {
                status.set_visible(pipelines.is_empty());
            }

            let queued_count = pipelines
                .iter()
                .filter(|p| p.status == PipelineStatus::Queued)
                .count();
            let running_count = pipelines
                .iter()
                .filter(|p| p.status == PipelineStatus::InProgress)
                .count();

            self.emit_user_activity(running_count > 0);

            if let Some(lbl) = imp.queue_count_label.borrow().as_ref() {
                let pause_suffix = if imp.paused.get() && running_count > 0 {
                    " — pausing after this job"
                } else {
                    ""
                };
                let status = match (running_count, queued_count) {
                    (0, 0) => "No user tasks".to_string(),
                    (0, queued) => {
                        let noun = if queued == 1 { "task" } else { "tasks" };
                        format!("{queued} queued {noun}")
                    }
                    (running, 0) => {
                        let noun = if running == 1 { "task" } else { "tasks" };
                        format!("{running} running {noun}{pause_suffix}")
                    }
                    (running, queued) => {
                        format!("{running} running • {queued} queued{pause_suffix}")
                    }
                };
                lbl.set_visible(true);
                lbl.set_label(&status);
            }

            let runner_active = imp.runner_active.get();
            let paused = imp.paused.get();

            if let Some(btn) = imp.start_btn.borrow().as_ref() {
                btn.set_sensitive((!runner_active || paused) && queued_count > 0);
            }
            if let Some(btn) = imp.pause_btn.borrow().as_ref() {
                btn.set_sensitive(runner_active && !paused);
            }
            if let Some(btn) = imp.clear_btn.borrow().as_ref() {
                btn.set_sensitive(queued_count > 0);
            }
            if let Some(btn) = imp.remove_btn.borrow().as_ref() {
                btn.set_sensitive(!imp.queue_checked_ids.borrow().is_empty());
            }

            for pipeline in pipelines {
                let expander = self.build_queue_expander_row(&pipeline, idx);
                list_box.append(&expander);
            }
        }

        let queue_selected_id = *imp.selected_pipeline_id.borrow();
        if !imp.selected_is_history.get() {
            if let Some(selected_id) = queue_selected_id {
                let mut found = false;
                let mut child = list_box.first_child();
                while let Some(widget) = child {
                    let next = widget.next_sibling();
                    if let Ok(row) = widget.clone().downcast::<gtk4::ListBoxRow>() {
                        if row.widget_name().parse::<i64>().ok() == Some(selected_id) {
                            list_box.select_row(Some(&row));
                            found = true;
                            break;
                        }
                    }
                    child = next;
                }
                if !found {
                    list_box.unselect_all();
                    *imp.selected_pipeline_id.borrow_mut() = None;
                    self.clear_summary();
                    if let Some(scroll) = imp.right_scroll.borrow().as_ref() {
                        scroll.set_visible(false);
                    }
                    if let Some(label) = imp.no_selection_label.borrow().as_ref() {
                        label.set_visible(true);
                    }
                }
            } else {
                list_box.unselect_all();
                if let Some(scroll) = imp.right_scroll.borrow().as_ref() {
                    scroll.set_visible(false);
                }
                if let Some(label) = imp.no_selection_label.borrow().as_ref() {
                    label.set_visible(true);
                }
            }
        }
    }

    pub fn refresh(&self) {
        let imp = self.imp();
        let Some(list_box) = imp.queue_list.borrow().clone() else {
            return;
        };
        let Some(history_list) = imp.history_list.borrow().clone() else {
            return;
        };
        let Some(history_section) = imp.history_section.borrow().clone() else {
            return;
        };
        imp.queue_chip_suffixes.borrow_mut().clear();
        while let Some(child) = list_box.first_child() {
            list_box.remove(&child);
        }
        while let Some(child) = history_list.first_child() {
            history_list.remove(&child);
        }

        let Some(state_rc) = imp.state.borrow().clone() else {
            return;
        };

        // Scope the state borrow so it is released before selection-restore,
        // which fires row_selected signals that re-borrow state.
        {
            let state = state_rc.borrow();
            let Some(idx) = state.library_index.as_ref() else {
                return;
            };

            let in_progress = idx
                .pipelines_by_status(PipelineStatus::InProgress)
                .unwrap_or_default();
            let queued = idx
                .pipelines_by_status(PipelineStatus::Queued)
                .unwrap_or_default();

            let mut pipelines = in_progress;
            pipelines.extend(queued);
            let visible_queue_ids: HashSet<i64> = pipelines.iter().map(|p| p.id).collect();
            imp.queue_checked_ids
                .borrow_mut()
                .retain(|pid| visible_queue_ids.contains(pid));

            if let Some(status) = imp.queue_empty_status.borrow().as_ref() {
                status.set_visible(pipelines.is_empty());
            }

            let queued_count = pipelines
                .iter()
                .filter(|p| p.status == PipelineStatus::Queued)
                .count();
            let running_count = pipelines
                .iter()
                .filter(|p| p.status == PipelineStatus::InProgress)
                .count();

            self.emit_user_activity(running_count > 0);

            if let Some(lbl) = imp.queue_count_label.borrow().as_ref() {
                let status = match (running_count, queued_count) {
                    (0, 0) => "No user tasks".to_string(),
                    (0, queued) => {
                        let noun = if queued == 1 { "task" } else { "tasks" };
                        format!("{queued} queued {noun}")
                    }
                    (running, 0) => {
                        let noun = if running == 1 { "task" } else { "tasks" };
                        format!("{running} running {noun}")
                    }
                    (running, queued) => format!("{running} running • {queued} queued"),
                };
                lbl.set_visible(true);
                lbl.set_label(&status);
            }

            let runner_active = imp.runner_active.get();
            let paused = imp.paused.get();

            if let Some(btn) = imp.start_btn.borrow().as_ref() {
                btn.set_sensitive((!runner_active || paused) && queued_count > 0);
            }
            if let Some(btn) = imp.pause_btn.borrow().as_ref() {
                btn.set_sensitive(runner_active && !paused);
            }
            if let Some(btn) = imp.clear_btn.borrow().as_ref() {
                btn.set_sensitive(queued_count > 0);
            }
            if let Some(btn) = imp.remove_btn.borrow().as_ref() {
                btn.set_sensitive(!imp.queue_checked_ids.borrow().is_empty());
            }

            for pipeline in pipelines {
                let expander = self.build_queue_expander_row(&pipeline, idx);
                list_box.append(&expander);
            }

            let mut history = idx
                .pipelines_by_status(PipelineStatus::Completed)
                .unwrap_or_default();
            history.extend(
                idx.pipelines_by_status(PipelineStatus::Failed)
                    .unwrap_or_default(),
            );
            history.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            history_section.set_visible(!history.is_empty());

            for pipeline in &history {
                let row = self.build_history_row(pipeline, idx);
                history_list.append(&row);
            }

            // Sync compare_queue with history
            {
                let mut compare_queue = Vec::new();
                for pipeline in &history {
                    if let Some(item) = build_compare_item_from_pipeline(pipeline, idx) {
                        compare_queue.push(item);
                    }
                }
                drop(state); // Drop the immutable borrow before mutably borrowing
                state_rc.borrow_mut().compare_queue = compare_queue;
            }
        } // state and idx dropped — row_selected signals are now safe to fire

        // Sync compare view if active
        if let Some(root) = self.root() {
            if let Some(window) = root.downcast_ref::<crate::ui::window::SharprWindow>() {
                if window.app_state().borrow().scope == crate::ui::window::ViewScope::Compare {
                    window.refresh_compare_view();
                }
            }
        }

        // --- Queue selection restore ---
        // Copy value out before the block — select_row fires load_settings_for_pipeline
        // which calls borrow_mut() on selected_pipeline_id; holding a borrow() here panics.
        let queue_selected_id = *imp.selected_pipeline_id.borrow();
        if !imp.selected_is_history.get() {
            if let Some(selected_id) = queue_selected_id {
                let mut found = false;
                let mut child = list_box.first_child();
                while let Some(widget) = child {
                    let next = widget.next_sibling();
                    if let Ok(row) = widget.clone().downcast::<gtk4::ListBoxRow>() {
                        if row.widget_name().parse::<i64>().ok() == Some(selected_id) {
                            list_box.select_row(Some(&row));
                            found = true;
                            break;
                        }
                    }
                    child = next;
                }
                if !found {
                    list_box.unselect_all();
                    *imp.selected_pipeline_id.borrow_mut() = None;
                    self.clear_summary();
                    if let Some(scroll) = imp.right_scroll.borrow().as_ref() {
                        scroll.set_visible(false);
                    }
                    if let Some(label) = imp.no_selection_label.borrow().as_ref() {
                        label.set_visible(true);
                    }
                }
            } else {
                list_box.unselect_all();
                if let Some(scroll) = imp.right_scroll.borrow().as_ref() {
                    scroll.set_visible(false);
                }
                if let Some(label) = imp.no_selection_label.borrow().as_ref() {
                    label.set_visible(true);
                }
            }
        } else {
            list_box.unselect_all();
        }

        // --- History selection restore ---
        let history_selected_id = *imp.selected_pipeline_id.borrow();
        if imp.selected_is_history.get() {
            if let Some(selected_id) = history_selected_id {
                let mut found = false;
                let mut child = history_list.first_child();
                while let Some(widget) = child {
                    let next = widget.next_sibling();
                    if let Ok(row) = widget.clone().downcast::<gtk4::ListBoxRow>() {
                        if row.widget_name().parse::<i64>().ok() == Some(selected_id) {
                            if let Some(handler) =
                                imp.history_row_selected_handler.borrow().as_ref()
                            {
                                history_list.block_signal(handler);
                            }
                            history_list.select_row(Some(&row));
                            if let Some(handler) =
                                imp.history_row_selected_handler.borrow().as_ref()
                            {
                                history_list.unblock_signal(handler);
                            }
                            found = true;
                            break;
                        }
                    }
                    child = next;
                }
                if !found {
                    history_list.unselect_all();
                    *imp.selected_pipeline_id.borrow_mut() = None;
                    imp.selected_is_history.set(false);
                    self.clear_summary();
                    if let Some(scroll) = imp.right_scroll.borrow().as_ref() {
                        scroll.set_visible(false);
                    }
                    if let Some(label) = imp.no_selection_label.borrow().as_ref() {
                        label.set_visible(true);
                    }
                }
            }
        } else {
            history_list.unselect_all();
        }
    }

    fn build_queue_expander_row(
        &self,
        pipeline: &Pipeline,
        idx: &LibraryIndex,
    ) -> libadwaita::ExpanderRow {
        let expander = libadwaita::ExpanderRow::new();
        expander.set_selectable(true);
        expander.set_activatable(true);
        expander.set_widget_name(&pipeline.id.to_string());

        let filename = pipeline
            .source_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unknown".to_string());
        expander.set_title(&filename);

        let steps = idx.steps_for_pipeline(pipeline.id).unwrap_or_default();
        expander.set_subtitle(&format_pipeline_composite(pipeline, &steps));
        expander.set_enable_expansion(!steps.is_empty());

        let check_btn = gtk4::CheckButton::new();
        check_btn.set_active(self.imp().queue_checked_ids.borrow().contains(&pipeline.id));
        {
            let widget_weak = self.downgrade();
            let pid = pipeline.id;
            check_btn.connect_toggled(move |btn| {
                let Some(w) = widget_weak.upgrade() else {
                    return;
                };
                if btn.is_active() {
                    w.imp().queue_checked_ids.borrow_mut().insert(pid);
                } else {
                    w.imp().queue_checked_ids.borrow_mut().remove(&pid);
                }
                let has_checked = !w.imp().queue_checked_ids.borrow().is_empty();
                let remove_btn = w.imp().remove_btn.borrow().clone();
                if let Some(remove_btn) = remove_btn {
                    remove_btn.set_sensitive(has_checked);
                }
            });
        }
        expander.add_prefix(&check_btn);

        let drag_handle = gtk4::Label::new(Some("⠿"));
        drag_handle.add_css_class("dim-label");
        drag_handle.set_sensitive(false);
        expander.add_prefix(&drag_handle);

        let picture = gtk4::Picture::new();
        picture.set_size_request(48, 48);
        picture.set_content_fit(gtk4::ContentFit::Cover);
        expander.add_prefix(&picture);

        {
            let picture_c = picture.clone();
            let path = pipeline.source_path.clone();
            glib::spawn_future_local(async move {
                if let Ok(texture) = load_thumbnail_for_row(&path).await {
                    picture_c.set_paintable(Some(&texture));
                }
            });
        }

        let chips = self.build_action_chips(pipeline, &steps);
        expander.add_suffix(&chips);
        self.imp()
            .queue_chip_suffixes
            .borrow_mut()
            .insert(pipeline.id, chips);

        if pipeline.status == PipelineStatus::InProgress {
            let progress = gtk4::ProgressBar::new();
            progress.set_pulse_step(0.1);
            progress.pulse();
            progress.set_valign(gtk4::Align::Center);
            expander.add_suffix(&progress);
        }

        for step in &steps {
            let child = libadwaita::ActionRow::new();
            child.set_title(&format_step_summary(step));
            child.set_subtitle(match step.status {
                PipelineStatus::Queued => "Queued",
                PipelineStatus::InProgress => "In Progress",
                PipelineStatus::Completed => "Completed",
                PipelineStatus::Failed => step.error_msg.as_deref().unwrap_or("Failed"),
            });

            let badge = gtk4::Label::new(Some(match step.status {
                PipelineStatus::Completed => "●",
                PipelineStatus::Failed => "●",
                PipelineStatus::InProgress => "●",
                PipelineStatus::Queued => "○",
            }));
            badge.add_css_class(match step.status {
                PipelineStatus::Completed => "success",
                PipelineStatus::Failed => "error",
                PipelineStatus::InProgress => "accent",
                PipelineStatus::Queued => "dim-label",
            });
            child.add_prefix(&badge);
            expander.add_row(&child);
        }

        let drag_source = gtk4::DragSource::new();
        drag_source.set_actions(gdk4::DragAction::MOVE);
        let pipeline_id = pipeline.id;
        drag_source.connect_prepare(move |_, _, _| {
            Some(gdk4::ContentProvider::for_value(
                &pipeline_id.to_string().to_value(),
            ))
        });
        expander.add_controller(drag_source);

        let widget_weak = self.downgrade();
        let target_expander = expander.clone();
        let drop_target = gtk4::DropTarget::new(glib::Type::STRING, gdk4::DragAction::MOVE);
        drop_target.connect_drop(move |_, value, _, _| {
            let Ok(dragged_str) = value.get::<String>() else {
                return false;
            };
            let Ok(dragged_id) = dragged_str.parse::<i64>() else {
                return false;
            };
            let Ok(target_id) = target_expander.widget_name().parse::<i64>() else {
                return false;
            };
            if dragged_id == target_id {
                return false;
            }
            let Some(w) = widget_weak.upgrade() else {
                return false;
            };
            let Some(state_rc) = w.imp().state.borrow().as_ref().cloned() else {
                return false;
            };
            let state = state_rc.borrow();
            let Some(idx) = state.library_index.as_ref() else {
                return false;
            };
            let queued = idx
                .pipelines_by_status(PipelineStatus::Queued)
                .unwrap_or_default();
            let Some(dragged_queue_order) = queued
                .iter()
                .find(|pipeline| pipeline.id == dragged_id)
                .map(|pipeline| pipeline.queue_order)
            else {
                return false;
            };
            let Some(target_queue_order) = queued
                .iter()
                .find(|pipeline| pipeline.id == target_id)
                .map(|pipeline| pipeline.queue_order)
            else {
                return false;
            };
            let new_order = if dragged_queue_order < target_queue_order {
                target_queue_order + 1
            } else {
                target_queue_order - 1
            };
            if idx.reorder_pipeline(dragged_id, new_order).is_err() {
                return false;
            }
            drop(state);
            w.refresh_queue();
            true
        });
        expander.add_controller(drop_target);

        expander
    }

    fn build_action_chips(&self, pipeline: &Pipeline, steps: &[PipelineStep]) -> gtk4::Box {
        let chips = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

        fn make_upscale_chip(backend: &str, scale: u32) -> gtk4::Label {
            let scale_str = match scale {
                2 => "2×",
                3 => "3×",
                4 => "4×",
                _ => "Smart scale",
            };
            let chip = gtk4::Label::new(Some(&format!("Upscale · {} · {}", backend, scale_str)));
            chip.add_css_class("accent");
            chip.add_css_class("pill");
            chip
        }

        fn make_export_chip(format: &str, quality: u8) -> gtk4::Label {
            let chip = gtk4::Label::new(Some(&format!(
                "Convert · {} · Q{}",
                format.to_uppercase(),
                quality
            )));
            chip.add_css_class("success");
            chip.add_css_class("pill");
            chip
        }

        fn append_chip_pair(
            chips: &gtk4::Box,
            upscale_chip: Option<gtk4::Label>,
            export_chip: Option<gtk4::Label>,
        ) {
            let has_upscale = upscale_chip.is_some();
            let has_export = export_chip.is_some();
            if let Some(upscale_chip) = upscale_chip {
                chips.append(&upscale_chip);
            }
            if has_upscale && has_export {
                chips.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
            }
            if let Some(export_chip) = export_chip {
                chips.append(&export_chip);
            }
        }

        if !steps.is_empty() {
            let mut upscale_chip = None;
            let mut export_chip = None;
            for step in steps {
                match step.step_type {
                    StepType::Upscale => {
                        if let Ok(settings) =
                            serde_json::from_str::<UpscaleStepSettings>(&step.settings_json)
                        {
                            upscale_chip =
                                Some(make_upscale_chip(&settings.backend, settings.scale));
                        }
                    }
                    StepType::Export => {
                        if let Ok(settings) =
                            serde_json::from_str::<ExportStepSettings>(&step.settings_json)
                        {
                            export_chip =
                                Some(make_export_chip(&settings.format, settings.quality));
                        }
                    }
                }
            }
            append_chip_pair(&chips, upscale_chip, export_chip);
        } else if let Some(config) = self.imp().pending_configs.borrow().get(&pipeline.id) {
            return Self::build_action_chips_from_config(config);
        } else {
            let chip = gtk4::Label::new(Some("⚠ No actions assigned"));
            chip.add_css_class("warning");
            chips.append(&chip);
        }

        chips
    }

    fn build_action_chips_from_config(config: &PendingConfig) -> gtk4::Box {
        let chips = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

        let upscale_chip = config.upscale_on.then(|| {
            let scale_str = match config.upscale.scale {
                2 => "2×",
                3 => "3×",
                4 => "4×",
                _ => "Smart scale",
            };
            let chip = gtk4::Label::new(Some(&format!(
                "Upscale · {} · {}",
                config.upscale.backend, scale_str
            )));
            chip.add_css_class("accent");
            chip.add_css_class("pill");
            chip
        });
        let export_chip = config.export_on.then(|| {
            let chip = gtk4::Label::new(Some(&format!(
                "Convert · {} · Q{}",
                config.export.format.to_uppercase(),
                config.export.quality
            )));
            chip.add_css_class("success");
            chip.add_css_class("pill");
            chip
        });

        let has_upscale = upscale_chip.is_some();
        let has_export = export_chip.is_some();
        if let Some(upscale_chip) = upscale_chip {
            chips.append(&upscale_chip);
        }
        if has_upscale && has_export {
            chips.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
        }
        if let Some(export_chip) = export_chip {
            chips.append(&export_chip);
        }
        if !config.upscale_on && !config.export_on {
            let chip = gtk4::Label::new(Some("⚠ No actions assigned"));
            chip.add_css_class("warning");
            chips.append(&chip);
        }

        chips
    }

    fn build_history_row(&self, pipeline: &Pipeline, idx: &LibraryIndex) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::new();
        row.set_activatable(true);
        row.set_selectable(true);
        row.set_widget_name(&pipeline.id.to_string());
        let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row_box.set_margin_top(8);
        row_box.set_margin_bottom(8);
        row_box.set_margin_start(8);
        row_box.set_margin_end(8);

        // --- Thumbnail pair ---
        let thumb_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);

        let src_picture = gtk4::Picture::new();
        src_picture.set_size_request(48, 48);
        src_picture.set_content_fit(gtk4::ContentFit::Cover);
        thumb_box.append(&src_picture);

        let out_picture = gtk4::Picture::new();
        out_picture.set_size_request(48, 48);
        out_picture.set_content_fit(gtk4::ContentFit::Cover);
        thumb_box.append(&out_picture);

        row_box.append(&thumb_box);

        // Async-load source thumbnail
        {
            let p = src_picture.clone();
            let path = pipeline.source_path.clone();
            glib::spawn_future_local(async move {
                if let Ok(tex) = load_thumbnail_for_row(&path).await {
                    p.set_paintable(Some(&tex));
                }
            });
        }

        let steps = idx.steps_for_pipeline(pipeline.id).unwrap_or_default();
        let display_step = Self::latest_output_step(&steps)
            .or_else(|| Self::latest_completed_step(&steps))
            .or_else(|| Self::active_step(&steps))
            .or_else(|| Self::first_step(&steps))
            .cloned();
        let status_step = Self::failed_step(&steps)
            .or_else(|| Self::active_step(&steps))
            .or(display_step.as_ref())
            .cloned();

        // Async-load output thumbnail (if output exists)
        if let Some(output_path) = display_step.as_ref().and_then(|s| s.output_path.clone()) {
            let p = out_picture.clone();
            glib::spawn_future_local(async move {
                if let Ok(tex) = load_thumbnail_for_row(&output_path).await {
                    p.set_paintable(Some(&tex));
                }
            });
        }

        // --- Info column ---
        let info_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        info_box.set_hexpand(true);

        let filename = pipeline
            .source_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unknown".to_string());
        let name_label = gtk4::Label::new(Some(&filename));
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        name_label.add_css_class("bold");
        info_box.append(&name_label);

        // Operation + settings summary
        let op_summary = display_step
            .as_ref()
            .map(format_step_summary)
            .unwrap_or_default();
        let op_label = gtk4::Label::new(Some(&op_summary));
        op_label.set_halign(gtk4::Align::Start);
        op_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        op_label.add_css_class("dim-label");
        op_label.add_css_class("caption");
        info_box.append(&op_label);

        // Timestamp
        let ts = format_timestamp(pipeline.created_at);
        let ts_label = gtk4::Label::new(Some(&ts));
        ts_label.set_halign(gtk4::Align::Start);
        ts_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        ts_label.add_css_class("dim-label");
        ts_label.add_css_class("caption");
        info_box.append(&ts_label);

        // Status + file-move degradation
        let (status_text, status_class) =
            self.resolve_status_display(pipeline, status_step.as_ref());
        let status_label = gtk4::Label::new(Some(&status_text));
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        status_label.add_css_class(status_class);
        status_label.add_css_class("caption");
        info_box.append(&status_label);

        row_box.append(&info_box);

        // --- Action buttons ---
        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        btn_box.set_valign(gtk4::Align::Center);

        // Compare button (navigates to Compare page)
        let compare_btn = gtk4::Button::with_label("Compare");
        compare_btn.add_css_class("flat");
        let output_path_opt = display_step.as_ref().and_then(|s| s.output_path.clone());
        let can_compare = pipeline.source_path.exists()
            && output_path_opt
                .as_ref()
                .map(|p| p.exists())
                .unwrap_or(false);
        compare_btn.set_sensitive(can_compare);
        compare_btn.set_tooltip_text(Some("Open in Compare page"));
        if can_compare {
            let pipeline_clone = pipeline.clone();
            let widget_weak = self.downgrade();
            compare_btn.connect_clicked(move |_| {
                let Some(w) = widget_weak.upgrade() else {
                    return;
                };

                let item = {
                    let state_rc = w.imp().state.borrow();
                    let Some(state) = state_rc.as_ref() else {
                        return;
                    };
                    let idx = state.borrow().library_index.clone();
                    let Some(idx) = idx else {
                        return;
                    };
                    build_compare_item_from_pipeline(&pipeline_clone, &idx)
                };

                if let Some(item) = item {
                    let imp = w.imp();
                    let cb_borrow = imp.compare_cb.borrow();
                    if let Some(cb) = cb_borrow.as_ref() {
                        cb(item);
                    }
                }
            });
        }
        let source_toggle_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        source_toggle_box.add_css_class("linked");
        let original_btn = gtk4::ToggleButton::with_label("Original");
        let result_btn = gtk4::ToggleButton::with_label("Result");
        source_toggle_box.append(&original_btn);
        source_toggle_box.append(&result_btn);
        let output_path_for_requeue = display_step.as_ref().and_then(|s| s.output_path.clone());
        let png_path_for_requeue = output_path_for_requeue
            .as_ref()
            .map(|path| preserved_png_temp_path(path.as_path()))
            .filter(|path: &PathBuf| path.exists());
        let png_btn: Option<gtk4::ToggleButton> = png_path_for_requeue.as_ref().map(|_| {
            let btn = gtk4::ToggleButton::with_label("PNG");
            source_toggle_box.append(&btn);
            btn
        });

        let has_output = display_step
            .as_ref()
            .and_then(|s| s.output_path.as_ref())
            .map(|p| p.exists())
            .unwrap_or(false);
        original_btn.set_active(!has_output);
        result_btn.set_active(has_output);
        result_btn.set_sensitive(has_output);
        if let Some(btn) = png_btn.as_ref() {
            btn.set_active(false);
        }

        {
            let ob = original_btn.clone();
            let png_btn = png_btn.clone();
            result_btn.connect_toggled(move |btn| {
                if btn.is_active() && ob.is_active() {
                    ob.set_active(false);
                }
                if btn.is_active() {
                    if let Some(png_btn) = png_btn.as_ref() {
                        if png_btn.is_active() {
                            png_btn.set_active(false);
                        }
                    }
                } else if !btn.is_active() && !ob.is_active() {
                    if let Some(png_btn) = png_btn.as_ref() {
                        if !png_btn.is_active() {
                            ob.set_active(true);
                        }
                    } else {
                        ob.set_active(true);
                    }
                }
            });
        }
        {
            let rb = result_btn.clone();
            let png_btn = png_btn.clone();
            original_btn.connect_toggled(move |btn| {
                if btn.is_active() && rb.is_active() {
                    rb.set_active(false);
                }
                if btn.is_active() {
                    if let Some(png_btn) = png_btn.as_ref() {
                        if png_btn.is_active() {
                            png_btn.set_active(false);
                        }
                    }
                } else if !btn.is_active() && !rb.is_active() {
                    if let Some(png_btn) = png_btn.as_ref() {
                        if !png_btn.is_active() {
                            btn.set_active(true);
                        }
                    } else {
                        btn.set_active(true);
                    }
                }
            });
        }
        if let Some(png_btn_ref) = png_btn.as_ref() {
            let original_btn = original_btn.clone();
            let result_btn = result_btn.clone();
            png_btn_ref.connect_toggled(move |btn| {
                if btn.is_active() {
                    if original_btn.is_active() {
                        original_btn.set_active(false);
                    }
                    if result_btn.is_active() {
                        result_btn.set_active(false);
                    }
                } else if !original_btn.is_active() && !result_btn.is_active() {
                    btn.set_active(true);
                }
            });
        }

        btn_box.append(&source_toggle_box);

        let requeue_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        requeue_btn.add_css_class("flat");
        requeue_btn.add_css_class("circular");
        requeue_btn.set_tooltip_text(Some("Re-queue with selected source"));
        let source_path_for_requeue = pipeline.source_path.clone();
        let png_btn_for_requeue = png_btn.clone();
        let widget_weak = self.downgrade();
        requeue_btn.connect_clicked(move |_| {
            let Some(w) = widget_weak.upgrade() else {
                return;
            };
            let use_png = png_btn_for_requeue
                .as_ref()
                .map(|btn| btn.is_active())
                .unwrap_or(false)
                && png_path_for_requeue
                    .as_ref()
                    .map(|p| p.exists())
                    .unwrap_or(false);
            let use_result = result_btn.is_active()
                && output_path_for_requeue
                    .as_ref()
                    .map(|p| p.exists())
                    .unwrap_or(false);
            let source = if use_png {
                png_path_for_requeue.clone().unwrap()
            } else if use_result {
                output_path_for_requeue.clone().unwrap()
            } else {
                source_path_for_requeue.clone()
            };
            if let Some(state_rc) = w.imp().state.borrow().as_ref() {
                if let Some(idx) = state_rc.borrow().library_index.as_ref() {
                    let _ = idx.create_pipeline(&source);
                }
            }
            w.refresh();
        });
        btn_box.append(&requeue_btn);
        btn_box.append(&compare_btn);

        // Delete button
        let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
        del_btn.add_css_class("flat");
        del_btn.add_css_class("circular");
        del_btn.set_tooltip_text(Some("Remove from history"));
        {
            let widget_weak = self.downgrade();
            let pid = pipeline.id;
            del_btn.connect_clicked(move |_| {
                let Some(w) = widget_weak.upgrade() else {
                    return;
                };
                if let Some(state_rc) = w.imp().state.borrow().as_ref() {
                    if let Some(idx) = state_rc.borrow().library_index.as_ref() {
                        let _ = idx.delete_pipeline(pid);
                    }
                }
                w.refresh();
            });
        }
        btn_box.append(&del_btn);

        row_box.append(&btn_box);
        row.set_child(Some(&row_box));
        row
    }

    fn resolve_status_display(
        &self,
        pipeline: &Pipeline,
        step: Option<&PipelineStep>,
    ) -> (String, &'static str) {
        if pipeline.status == PipelineStatus::Failed {
            let msg = step
                .and_then(|s| s.error_msg.as_deref())
                .unwrap_or("Unknown error");
            return (format!("Failed: {}", msg), "error");
        }
        // Completed — check file existence
        let source_ok = pipeline.source_path.exists();
        let output_ok = step
            .and_then(|s| s.output_path.as_ref())
            .map(|p| p.exists())
            .unwrap_or(false);

        match (source_ok, output_ok) {
            (true, true) => ("Done".to_string(), "success"),
            (false, true) => ("Done — source moved".to_string(), "warning"),
            (true, false) => ("Done — output missing".to_string(), "warning"),
            (false, false) => ("Done — files moved".to_string(), "warning"),
        }
    }

    fn try_start_runner(&self) {
        let imp = self.imp();
        if imp.polling_timer.borrow().is_some() {
            return;
        }

        // Dead-man's switch: if the runner is active but somehow stalled,
        // nudge it forward. Never auto-starts — that's the caller's job.
        let widget_weak = self.downgrade();
        let source_id = glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
            if let Some(w) = widget_weak.upgrade() {
                if w.imp().runner_active.get() {
                    w.run_next_pipeline();
                }
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
        *imp.polling_timer.borrow_mut() = Some(source_id);
    }

    fn run_next_pipeline(&self) {
        let imp = self.imp();
        if !imp.runner_active.get() || imp.paused.get() {
            return;
        }

        let Some(state_rc) = imp.state.borrow().clone() else {
            imp.runner_active.set(false);
            return;
        };

        let result = {
            let state = state_rc.borrow();
            let idx = match state.library_index.as_ref() {
                Some(i) => i,
                None => {
                    imp.runner_active.set(false);
                    return;
                }
            };

            if idx
                .pipelines_by_status(PipelineStatus::InProgress)
                .map(|pipelines| !pipelines.is_empty())
                .unwrap_or(false)
            {
                return;
            }

            let pipeline = match idx.next_queued_pipeline().ok().flatten() {
                Some(p) => p,
                None => {
                    imp.runner_active.set(false);
                    drop(state);
                    self.refresh();
                    return;
                }
            };

            let steps = idx.steps_for_pipeline(pipeline.id).unwrap_or_default();
            if steps.is_empty() {
                imp.runner_active.set(false);
                drop(state);
                self.refresh();
                return;
            }
            let step = match steps
                .iter()
                .find(|s| s.status == PipelineStatus::Queued)
                .cloned()
            {
                Some(s) => s,
                None => {
                    // Pipeline has no queued steps — mark complete
                    let _ = idx.set_pipeline_status(pipeline.id, PipelineStatus::Completed);
                    std::fs::remove_dir_all(
                        glib::user_data_dir()
                            .join("sharpr")
                            .join("pipeline_work")
                            .join(pipeline.id.to_string()),
                    )
                    .ok();
                    drop(state);
                    self.refresh();
                    self.run_next_pipeline();
                    return;
                }
            };

            let effective_source = if step.step_order == 1 {
                pipeline.source_path.clone()
            } else {
                let prev = steps
                    .iter()
                    .find(|s| s.step_order == step.step_order - 1)
                    .and_then(|s| s.output_path.clone());
                match prev {
                    Some(p) => p,
                    None => {
                        let _ = idx.set_step_status(
                            step.id,
                            PipelineStatus::Failed,
                            None,
                            Some("Previous step has no output"),
                        );
                        let _ = idx.set_pipeline_status(pipeline.id, PipelineStatus::Failed);
                        drop(state);
                        self.refresh();
                        return;
                    }
                }
            };
            let _ = idx.set_step_input_path(step.id, &effective_source);

            let _ = idx.set_pipeline_status(pipeline.id, PipelineStatus::InProgress);
            let _ = idx.set_step_status(step.id, PipelineStatus::InProgress, None, None);
            (pipeline, step, effective_source)
        };

        self.refresh();

        let (pipeline, step, effective_source) = result;
        let widget_weak = self.downgrade();
        let state_rc_c = state_rc.clone();

        match step.step_type {
            StepType::Export => {
                self.run_export_step(pipeline, step, effective_source, widget_weak, state_rc_c);
            }
            StepType::Upscale => {
                self.run_upscale_step(pipeline, step, effective_source, widget_weak, state_rc_c);
            }
        }
    }

    fn run_export_step(
        &self,
        pipeline: Pipeline,
        step: PipelineStep,
        effective_source: PathBuf,
        widget_weak: WeakRef<TasksPage>,
        state_rc: Rc<RefCell<AppState>>,
    ) {
        let (tx, rx) = async_channel::bounded::<Result<PathBuf, String>>(1);
        let source = effective_source;
        let source_path_for_metadata = pipeline.source_path.clone();
        let settings_json = step.settings_json.clone();

        let export_output_dir = state_rc.borrow().settings.export_output_dir.clone();

        std::thread::spawn(move || {
            let settings: ExportStepSettings = match serde_json::from_str(&settings_json) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send_blocking(Err(e.to_string()));
                    return;
                }
            };
            let format = match settings.format.as_str() {
                "webp" => ExportFormat::Webp,
                "png" => ExportFormat::Png,
                "jpeg" => ExportFormat::Jpeg,
                _ => ExportFormat::Jxl,
            };

            let dest_dir = if settings.destination == "source" {
                pipeline
                    .source_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("."))
            } else if settings.destination == "custom" {
                settings.custom_path.clone().unwrap_or_else(|| {
                    resolve_output_dir(export_output_dir.as_ref(), OutputFolderKind::Export)
                })
            } else {
                resolve_output_dir(export_output_dir.as_ref(), OutputFolderKind::Export)
            };

            let output = unique_output_path(&dest_dir, &source, format);
            let result = export_to_path(
                &source,
                &output,
                settings.max_edge,
                format,
                settings.quality,
            );

            let _ = tx.send_blocking(match result {
                Ok(_) => Ok(output),
                Err(e) => Err(e.to_string()),
            });
        });

        glib::spawn_future_local(async move {
            let result = rx.recv().await;
            if let Some(w) = widget_weak.upgrade() {
                if let Some(state_rc) = w.imp().state.borrow().as_ref() {
                    let state = state_rc.borrow();
                    if let Some(idx) = state.library_index.as_ref() {
                        match result {
                            Ok(Ok(path)) => {
                                let _ = idx.set_step_status(
                                    step.id,
                                    PipelineStatus::Completed,
                                    Some(&path),
                                    None,
                                );
                                inherit_generated_output_metadata(
                                    &state,
                                    &source_path_for_metadata,
                                    &path,
                                    step.step_type,
                                );
                                let steps = idx.steps_for_pipeline(pipeline.id).unwrap_or_default();
                                if steps.iter().all(|s| s.status == PipelineStatus::Completed) {
                                    let _ = idx.set_pipeline_status(
                                        pipeline.id,
                                        PipelineStatus::Completed,
                                    );
                                    std::fs::remove_dir_all(
                                        glib::user_data_dir()
                                            .join("sharpr")
                                            .join("pipeline_work")
                                            .join(pipeline.id.to_string()),
                                    )
                                    .ok();
                                } else {
                                    // More steps remain — reset pipeline to Queued so
                                    // run_next_pipeline can find it (it queries status='queued').
                                    let _ = idx
                                        .set_pipeline_status(pipeline.id, PipelineStatus::Queued);
                                }
                            }
                            Ok(Err(e)) => {
                                let _ = idx.set_step_status(
                                    step.id,
                                    PipelineStatus::Failed,
                                    None,
                                    Some(&e),
                                );
                                let _ =
                                    idx.set_pipeline_status(pipeline.id, PipelineStatus::Failed);
                            }
                            Err(_) => {
                                let _ = idx.set_step_status(
                                    step.id,
                                    PipelineStatus::Failed,
                                    None,
                                    Some("Channel closed"),
                                );
                                let _ =
                                    idx.set_pipeline_status(pipeline.id, PipelineStatus::Failed);
                            }
                        }

                        // Auto-prune history to cap
                        let cap = state.settings.pipeline_history_cap as usize;
                        let _ = idx.prune_pipeline_history(cap);
                    }
                }
                w.refresh();
                w.run_next_pipeline();
            }
        });
    }

    fn run_upscale_step(
        &self,
        pipeline: Pipeline,
        step: PipelineStep,
        effective_source: PathBuf,
        widget_weak: WeakRef<TasksPage>,
        state_rc: Rc<RefCell<AppState>>,
    ) {
        let (tx, rx) = async_channel::bounded::<Result<PathBuf, String>>(1);
        let source = effective_source;
        let settings_json = step.settings_json.clone();

        let (
            upscaler_binary_path,
            upscaled_output_dir,
            comfyui_url,
            comfyui_workflow_global,
            onnx_upscale_model,
        ) = {
            let st = state_rc.borrow();
            (
                st.settings
                    .upscaler_binary_path
                    .clone()
                    .or_else(crate::upscale::UpscaleDetector::find_realesrgan),
                st.settings.upscaled_output_dir.clone(),
                st.settings.comfyui_url.clone(),
                st.settings.comfyui_workflow.clone(), // global fallback
                st.settings.onnx_upscale_model.clone(),
            )
        };

        std::thread::spawn(move || {
            let settings: UpscaleStepSettings = match serde_json::from_str(&settings_json) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send_blocking(Err(e.to_string()));
                    return;
                }
            };

            let backend_kind = UpscaleBackendKind::from_settings(&settings.backend);

            let model = UpscaleModel::from_settings(&settings.model);
            let format = UpscaleOutputFormat::from_settings(&settings.format);

            let source_dimensions = image::image_dimensions(&source).unwrap_or((0, 0));
            let target_dimensions =
                if settings.scale == 0 && backend_kind == UpscaleBackendKind::ComfyUi {
                    UpscaleRunner::comfyui_smart_target_dimensions(
                        source_dimensions.0,
                        source_dimensions.1,
                    )
                } else {
                    None
                };

            let job = UpscaleJobConfig {
                source_dimensions,
                requested_scale: settings.scale,
                execution_scale: if settings.scale == 0 {
                    4
                } else {
                    settings.scale
                },
                target_dimensions,
                model,
                compress_output: settings.compress,
                compressed_format: format,
                keep_raw_png_sidecar: settings.keep_png,
                compression_mode: UpscaleCompressionMode::Auto,
                quality: settings.quality,
                tile_size: None,
                gpu_id: None,
            };

            let dest_dir = if step.step_order > 1 {
                let work_dir = glib::user_data_dir()
                    .join("sharpr")
                    .join("pipeline_work")
                    .join(pipeline.id.to_string());
                std::fs::create_dir_all(&work_dir).ok();
                work_dir
            } else if settings.destination == "source" {
                source
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("."))
            } else if settings.destination == "custom" {
                settings.custom_path.clone().unwrap_or_else(|| {
                    resolve_output_dir(upscaled_output_dir.as_ref(), OutputFolderKind::Upscaled)
                })
            } else {
                resolve_output_dir(upscaled_output_dir.as_ref(), OutputFolderKind::Upscaled)
            };

            let onnx_model = settings
                .onnx_model
                .map(|s| crate::upscale::OnnxUpscaleModel::from_settings(&s))
                .unwrap_or_else(|| {
                    crate::upscale::OnnxUpscaleModel::from_settings(&onnx_upscale_model)
                });
            let effective_comfyui_workflow = settings
                .comfyui_workflow
                .as_deref()
                .unwrap_or(&comfyui_workflow_global);
            let comfyui_workflow =
                crate::upscale::ComfyUiWorkflow::from_settings(effective_comfyui_workflow);

            let backend = make_upscale_backend(
                backend_kind,
                upscaler_binary_path,
                onnx_model,
                &comfyui_url,
                comfyui_workflow,
            );

            let Some(backend) = backend else {
                let _ = tx.send_blocking(Err("Failed to build upscale backend".to_string()));
                return;
            };

            // Derive the output extension from what save_image will actually produce,
            // so the stored path and the file on disk always match.
            let output_ext = if settings.compress {
                match settings.format.as_str() {
                    "webp" => "webp",
                    "jpeg" => "jpg",
                    "png" => "png",
                    _ => "jxl",
                }
            } else {
                "png"
            };
            let output_filename =
                crate::export::unique_output_path_for_extension(&dest_dir, &source, output_ext);
            let rx_events = backend.run(source.clone(), output_filename, job);

            // Wait for completion
            let mut last_result = Err("Job did not finish".to_string());
            while let Ok(event) = rx_events.recv_blocking() {
                match event {
                    crate::upscale::runner::UpscaleEvent::Done(path) => {
                        last_result = Ok(path);
                    }
                    crate::upscale::runner::UpscaleEvent::Failed(msg) => {
                        last_result = Err(msg);
                    }
                    _ => {}
                }
            }

            let _ = tx.send_blocking(last_result);
        });

        glib::spawn_future_local(async move {
            let result = rx.recv().await;
            if let Some(w) = widget_weak.upgrade() {
                if let Some(state_rc) = w.imp().state.borrow().as_ref() {
                    let state = state_rc.borrow();
                    if let Some(idx) = state.library_index.as_ref() {
                        match result {
                            Ok(Ok(path)) => {
                                let _ = idx.set_step_status(
                                    step.id,
                                    PipelineStatus::Completed,
                                    Some(&path),
                                    None,
                                );
                                inherit_generated_output_metadata(
                                    &state,
                                    &pipeline.source_path,
                                    &path,
                                    step.step_type,
                                );
                                let steps = idx.steps_for_pipeline(pipeline.id).unwrap_or_default();
                                if steps.iter().all(|s| s.status == PipelineStatus::Completed) {
                                    let _ = idx.set_pipeline_status(
                                        pipeline.id,
                                        PipelineStatus::Completed,
                                    );
                                    std::fs::remove_dir_all(
                                        glib::user_data_dir()
                                            .join("sharpr")
                                            .join("pipeline_work")
                                            .join(pipeline.id.to_string()),
                                    )
                                    .ok();
                                } else {
                                    // More steps remain — reset pipeline to Queued so
                                    // run_next_pipeline can find it (it queries status='queued').
                                    let _ = idx
                                        .set_pipeline_status(pipeline.id, PipelineStatus::Queued);
                                }
                            }
                            Ok(Err(e)) => {
                                let _ = idx.set_step_status(
                                    step.id,
                                    PipelineStatus::Failed,
                                    None,
                                    Some(&e),
                                );
                                let _ =
                                    idx.set_pipeline_status(pipeline.id, PipelineStatus::Failed);
                            }
                            Err(_) => {
                                let _ = idx.set_step_status(
                                    step.id,
                                    PipelineStatus::Failed,
                                    None,
                                    Some("Channel closed"),
                                );
                                let _ =
                                    idx.set_pipeline_status(pipeline.id, PipelineStatus::Failed);
                            }
                        }

                        // Auto-prune history to cap
                        let cap = state.settings.pipeline_history_cap as usize;
                        let _ = idx.prune_pipeline_history(cap);
                    }
                }
                w.refresh();
                w.run_next_pipeline();
            }
        });
    }
}

async fn load_thumbnail_for_row(path: &Path) -> Result<gdk4::Texture, String> {
    let path = path.to_path_buf();
    let (tx, rx) = async_channel::bounded::<Result<gdk4::Texture, String>>(1);

    std::thread::spawn(move || {
        let result = (|| -> Result<gdk4::Texture, String> {
            let img = image::open(&path).map_err(|e| e.to_string())?;
            let thumb = img.thumbnail(64, 64);
            let rgb = thumb.to_rgba8();
            let bytes = glib::Bytes::from(rgb.as_raw());
            Ok(gdk4::MemoryTexture::new(
                rgb.width() as i32,
                rgb.height() as i32,
                gdk4::MemoryFormat::R8g8b8a8,
                &bytes,
                (rgb.width() * 4) as usize,
            )
            .upcast())
        })();
        let _ = tx.send_blocking(result);
    });

    rx.recv()
        .await
        .unwrap_or_else(|_| Err("Thumbnail thread died".to_string()))
}

fn format_pipeline_composite(pipeline: &Pipeline, steps: &[PipelineStep]) -> String {
    if steps.is_empty() {
        return String::new();
    }

    if steps
        .iter()
        .all(|step| step.status == PipelineStatus::Queued)
    {
        let noun = if steps.len() == 1 { "step" } else { "steps" };
        return format!("Queued · {} {}", steps.len(), noun);
    }

    if let Some(active_step) = steps
        .iter()
        .find(|step| step.status == PipelineStatus::InProgress)
    {
        return format!(
            "Step {} of {}: {}",
            active_step.step_order,
            steps.len(),
            format_step_summary(active_step)
        );
    }

    if pipeline.status == PipelineStatus::Queued {
        let noun = if steps.len() == 1 { "step" } else { "steps" };
        return format!("Queued · {} {}", steps.len(), noun);
    }

    "In Progress".to_string()
}

fn format_step_summary(step: &PipelineStep) -> String {
    match step.step_type {
        StepType::Upscale => {
            if let Ok(s) = serde_json::from_str::<UpscaleStepSettings>(&step.settings_json) {
                let model_display = match s.model.as_str() {
                    "anime" => "Anime",
                    _ => "Standard",
                };
                let scale = match s.scale {
                    0 => "Smart scale".to_string(),
                    n => format!("{}×", n),
                };
                format!("Upscale · {} · {}", model_display, scale)
            } else {
                "Upscale".to_string()
            }
        }
        StepType::Export => {
            if let Ok(s) = serde_json::from_str::<ExportStepSettings>(&step.settings_json) {
                let edge = s
                    .max_edge
                    .map(|e| format!(" · {}px", e))
                    .unwrap_or_default();
                format!("Export · {}{}", s.format.to_uppercase(), edge)
            } else {
                "Export".to_string()
            }
        }
    }
}

fn format_timestamp(unix_secs: i64) -> String {
    // Simple relative formatting without external crates.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = now - unix_secs;
    if delta < 60 {
        "Just now".to_string()
    } else if delta < 3600 {
        format!("{} min ago", delta / 60)
    } else if delta < 86400 {
        format!("{} hr ago", delta / 3600)
    } else {
        format!("{} days ago", delta / 86400)
    }
}

#[cfg(test)]
mod tests {
    // Pipeline formatting tests or other tasks_page specific tests could go here
}
