use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc};

use glib::WeakRef;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::library_index::{LibraryIndex, Pipeline, PipelineStatus, PipelineStep, StepType};
use crate::ui::window::AppState;
use crate::upscale::{
    backend::make_upscale_backend,
    UpscaleBackendKind, UpscaleJobConfig, UpscaleModel, UpscaleOutputFormat,
    UpscaleCompressionMode,
};
use crate::export::{ExportFormat, OutputFolderKind, resolve_output_dir, unique_output_path, export_to_path};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct UpscaleStepSettings {
    pub backend: String,       // "cli" | "onnx" | "comfyui"
    pub model: String,         // "standard" | "anime"
    pub scale: u32,            // 0 = smart/auto, 2, 3, 4
    pub compress: bool,
    pub format: String,        // "jxl" | "webp" | "jpeg" | "png"
    pub quality: u8,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ExportStepSettings {
    pub format: String,        // "jxl" | "webp" | "png" | "jpeg"
    pub max_edge: Option<u32>, // None = original size
    pub quality: u8,
}

pub type CompareCallback = Box<dyn Fn(PathBuf, PathBuf, String)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct TasksPage {
        pub queue_list: RefCell<Option<gtk4::ListBox>>,
        pub settings_stack: RefCell<Option<gtk4::Stack>>,
        pub start_btn: RefCell<Option<gtk4::Button>>,
        pub stop_btn: RefCell<Option<gtk4::Button>>,
        pub op_dropdown: RefCell<Option<gtk4::DropDown>>,

        // Upscale settings widgets
        pub backend_dropdown: RefCell<Option<gtk4::DropDown>>,
        pub scale_dropdown: RefCell<Option<gtk4::DropDown>>,
        pub compress_check: RefCell<Option<gtk4::CheckButton>>,
        pub format_dropdown: RefCell<Option<gtk4::DropDown>>,
        pub quality_spin: RefCell<Option<gtk4::SpinButton>>,

        // Export settings widgets
        pub export_format_dropdown: RefCell<Option<gtk4::DropDown>>,
        pub export_edge_dropdown: RefCell<Option<gtk4::DropDown>>,
        pub export_quality_spin: RefCell<Option<gtk4::SpinButton>>,

        // History
        pub history_list: RefCell<Option<gtk4::ListBox>>,
        pub clear_history_btn: RefCell<Option<gtk4::Button>>,
        pub history_section: RefCell<Option<gtk4::Box>>,

        // State
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
        pub compare_cb: RefCell<Option<CompareCallback>>,
        pub runner_active: Rc<Cell<bool>>,
        pub polling_timer: RefCell<Option<glib::SourceId>>,
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

            let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            toolbar.set_halign(gtk4::Align::End);
            let start_btn = gtk4::Button::with_label("Start Queue");
            start_btn.add_css_class("suggested-action");
            let stop_btn = gtk4::Button::with_label("Stop");
            stop_btn.add_css_class("destructive-action");
            toolbar.append(&start_btn);
            toolbar.append(&stop_btn);

            let queue_list = gtk4::ListBox::new();
            queue_list.add_css_class("boxed-list");
            queue_list.set_selection_mode(gtk4::SelectionMode::None);

            let scrolled = gtk4::ScrolledWindow::new();
            scrolled.set_vexpand(true);
            scrolled.set_child(Some(&queue_list));

            left_col.append(&toolbar);
            left_col.append(&scrolled);

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
            history_list.set_selection_mode(gtk4::SelectionMode::None);
            history_section.append(&history_list);

            // Hidden until there are history entries
            history_section.set_visible(false);

            left_col.append(&history_section);

            // --- Right Column ---
            let right_col = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
            right_col.set_width_request(300);
            right_col.set_margin_top(12);
            right_col.set_margin_bottom(12);
            right_col.set_margin_start(12);
            right_col.set_margin_end(12);

            let op_label = gtk4::Label::new(Some("Operation"));
            op_label.set_halign(gtk4::Align::Start);
            op_label.add_css_class("heading");

            let op_model = gtk4::StringList::new(&["Upscale", "Export"]);
            let op_dropdown = gtk4::DropDown::new(Some(op_model), None::<gtk4::Expression>);

            let settings_stack = gtk4::Stack::new();
            settings_stack.set_transition_type(gtk4::StackTransitionType::SlideLeftRight);

            // Upscale Settings Page
            let upscale_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);

            let backend_label = gtk4::Label::new(Some("Backend"));
            backend_label.set_halign(gtk4::Align::Start);
            let backend_model = gtk4::StringList::new(&["ONNX", "CLI", "ComfyUI"]);
            let backend_dropdown = gtk4::DropDown::new(Some(backend_model), None::<gtk4::Expression>);
            upscale_box.append(&backend_label);
            upscale_box.append(&backend_dropdown);

            let scale_label = gtk4::Label::new(Some("Scale"));
            scale_label.set_halign(gtk4::Align::Start);
            let scale_model = gtk4::StringList::new(&["Auto (Smart)", "2x", "3x", "4x"]);
            let scale_dropdown = gtk4::DropDown::new(Some(scale_model), None::<gtk4::Expression>);

            let compress_check = gtk4::CheckButton::with_label("Compress output");
            
            let format_label = gtk4::Label::new(Some("Format"));
            format_label.set_halign(gtk4::Align::Start);
            let format_model = gtk4::StringList::new(&["JXL", "WebP", "JPEG", "PNG"]);
            let format_dropdown = gtk4::DropDown::new(Some(format_model), None::<gtk4::Expression>);

            let quality_label = gtk4::Label::new(Some("Quality"));
            quality_label.set_halign(gtk4::Align::Start);
            let quality_adj = gtk4::Adjustment::new(85.0, 1.0, 100.0, 1.0, 10.0, 0.0);
            let quality_spin = gtk4::SpinButton::new(Some(&quality_adj), 1.0, 0);

            upscale_box.append(&scale_label);
            upscale_box.append(&scale_dropdown);
            upscale_box.append(&compress_check);
            upscale_box.append(&format_label);
            upscale_box.append(&format_dropdown);
            upscale_box.append(&quality_label);
            upscale_box.append(&quality_spin);

            settings_stack.add_named(&upscale_box, Some("upscale"));

            // Export Settings Page
            let export_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);

            let export_format_label = gtk4::Label::new(Some("Format"));
            export_format_label.set_halign(gtk4::Align::Start);
            let export_format_model = gtk4::StringList::new(&["JXL", "WebP", "PNG", "JPEG"]);
            let export_format_dropdown = gtk4::DropDown::new(Some(export_format_model), None::<gtk4::Expression>);

            let export_edge_label = gtk4::Label::new(Some("Max Edge"));
            export_edge_label.set_halign(gtk4::Align::Start);
            let export_edge_model = gtk4::StringList::new(&["Original", "1080px", "2160px", "4096px"]);
            let export_edge_dropdown = gtk4::DropDown::new(Some(export_edge_model), None::<gtk4::Expression>);

            let export_quality_label = gtk4::Label::new(Some("Quality"));
            export_quality_label.set_halign(gtk4::Align::Start);
            let export_quality_adj = gtk4::Adjustment::new(85.0, 1.0, 100.0, 1.0, 10.0, 0.0);
            let export_quality_spin = gtk4::SpinButton::new(Some(&export_quality_adj), 1.0, 0);

            export_box.append(&export_format_label);
            export_box.append(&export_format_dropdown);
            export_box.append(&export_edge_label);
            export_box.append(&export_edge_dropdown);
            export_box.append(&export_quality_label);
            export_box.append(&export_quality_spin);

            settings_stack.add_named(&export_box, Some("export"));

            right_col.append(&op_label);
            right_col.append(&op_dropdown);
            right_col.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
            right_col.append(&settings_stack);

            main_box.append(&left_col);
            main_box.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
            main_box.append(&right_col);

            // Wire op_dropdown to stack
            {
                let settings_stack_c = settings_stack.clone();
                op_dropdown.connect_selected_notify(move |dd| {
                    match dd.selected() {
                        0 => settings_stack_c.set_visible_child_name("upscale"),
                        1 => settings_stack_c.set_visible_child_name("export"),
                        _ => {}
                    }
                });
            }

            // Wire Start/Stop
            {
                let widget_weak = widget.downgrade();
                start_btn.connect_clicked(move |_| {
                    if let Some(w) = widget_weak.upgrade() {
                        w.try_start_runner();
                    }
                });
            }
            {
                let widget_weak = widget.downgrade();
                stop_btn.connect_clicked(move |_| {
                    if let Some(w) = widget_weak.upgrade() {
                        w.imp().runner_active.set(false);
                        w.refresh();
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

            *self.queue_list.borrow_mut() = Some(queue_list);
            *self.settings_stack.borrow_mut() = Some(settings_stack);
            *self.start_btn.borrow_mut() = Some(start_btn);
            *self.stop_btn.borrow_mut() = Some(stop_btn);
            *self.op_dropdown.borrow_mut() = Some(op_dropdown);

            *self.backend_dropdown.borrow_mut() = Some(backend_dropdown);
            *self.scale_dropdown.borrow_mut() = Some(scale_dropdown);
            *self.compress_check.borrow_mut() = Some(compress_check);
            *self.format_dropdown.borrow_mut() = Some(format_dropdown);
            *self.quality_spin.borrow_mut() = Some(quality_spin);

            *self.export_format_dropdown.borrow_mut() = Some(export_format_dropdown);
            *self.export_edge_dropdown.borrow_mut() = Some(export_edge_dropdown);
            *self.export_quality_spin.borrow_mut() = Some(export_quality_spin);

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

    pub fn set_state(&self, state: Rc<RefCell<AppState>>) {
        let imp = self.imp();

        // Load initial defaults from settings
        {
            let st = state.borrow();
            if let Some(backend_dd) = imp.backend_dropdown.borrow().as_ref() {
                let idx = match st.settings.upscale_backend.as_str() {
                    "cli" => 1,
                    "comfyui" => 2,
                    _ => 0, // onnx
                };
                backend_dd.set_selected(idx);
            }
            if let Some(scale_dd) = imp.scale_dropdown.borrow().as_ref() {
                scale_dd.set_selected(0);
            }
            if let Some(compress) = imp.compress_check.borrow().as_ref() {
                compress.set_active(st.settings.upscale_compress_output);
            }
            if let Some(format_dd) = imp.format_dropdown.borrow().as_ref() {
                let idx = match st.settings.upscale_compressed_format.as_str() {
                    "webp" => 1,
                    "jpeg" => 2,
                    "png" => 3,
                    _ => 0, // jxl
                };
                format_dd.set_selected(idx);
            }
            if let Some(spin) = imp.quality_spin.borrow().as_ref() {
                spin.set_value(85.0);
            }

            // Export defaults
            if let Some(format_dd) = imp.export_format_dropdown.borrow().as_ref() {
                format_dd.set_selected(0); // JXL
            }
            if let Some(edge_dd) = imp.export_edge_dropdown.borrow().as_ref() {
                edge_dd.set_selected(0); // Original
            }
            if let Some(spin) = imp.export_quality_spin.borrow().as_ref() {
                spin.set_value(85.0);
            }
        }

        // Wire backend dropdown to persist the selection back to AppSettings
        if let Some(backend_dd) = imp.backend_dropdown.borrow().as_ref() {
            let state_c = state.clone();
            backend_dd.connect_selected_notify(move |dd| {
                let key = match dd.selected() {
                    1 => "cli",
                    2 => "comfyui",
                    _ => "onnx",
                };
                if let Ok(mut st) = state_c.try_borrow_mut() {
                    st.settings.set_upscale_backend(key);
                }
            });
        }

        *imp.state.borrow_mut() = Some(state.clone());
        self.refresh();

        // Auto-start on load if queued jobs exist (crash recovery / app restart).
        {
            let st = state.borrow();
            if let Some(idx) = st.library_index.as_ref() {
                let has_queued = idx
                    .pipelines_by_status(PipelineStatus::Queued)
                    .map(|v| !v.is_empty())
                    .unwrap_or(false);
                if has_queued {
                    imp.runner_active.set(true);
                }
            }
        }

        self.try_start_runner();
        if imp.runner_active.get() {
            self.run_next_pipeline();
        }
    }

    pub fn set_compare_requested_cb<F: Fn(PathBuf, PathBuf, String) + 'static>(&self, f: F) {
        *self.imp().compare_cb.borrow_mut() = Some(Box::new(f));
    }

    /// Returns the step type and settings JSON currently shown in the settings panel.
    /// Used by window.rs when adding a job from the filmstrip.
    pub fn current_step_config(&self) -> (StepType, String) {
        let imp = self.imp();
        let op_idx = imp.op_dropdown.borrow().as_ref().map(|d| d.selected()).unwrap_or(0);
        if op_idx == 1 {
            // Export
            let format = imp.export_format_dropdown.borrow().as_ref().map(|d| match d.selected() {
                1 => "webp", 2 => "png", 3 => "jpeg", _ => "jxl",
            }).unwrap_or("jxl");
            let max_edge = imp.export_edge_dropdown.borrow().as_ref().and_then(|d| match d.selected() {
                1 => Some(1080u32), 2 => Some(2160), 3 => Some(4096), _ => None,
            });
            let quality = imp.export_quality_spin.borrow().as_ref().map(|s| s.value() as u8).unwrap_or(85);
            let settings = ExportStepSettings { format: format.into(), max_edge, quality };
            (StepType::Export, serde_json::to_string(&settings).unwrap_or_default())
        } else {
            // Upscale (default)
            let backend = imp.backend_dropdown.borrow().as_ref().map(|d| match d.selected() {
                1 => "cli", 2 => "comfyui", _ => "onnx",
            }).unwrap_or("onnx");
            let scale = imp.scale_dropdown.borrow().as_ref().map(|d| match d.selected() {
                1 => 2u32, 2 => 3, 3 => 4, _ => 0,
            }).unwrap_or(0);
            let compress = imp.compress_check.borrow().as_ref().map(|c| c.is_active()).unwrap_or(false);
            let format = imp.format_dropdown.borrow().as_ref().map(|d| match d.selected() {
                1 => "webp", 2 => "jpeg", 3 => "png", _ => "jxl",
            }).unwrap_or("jxl");
            let quality = imp.quality_spin.borrow().as_ref().map(|s| s.value() as u8).unwrap_or(85);
            let model = imp.state.borrow().as_ref()
                .map(|s| s.borrow().settings.upscaler_default_model.clone())
                .unwrap_or_default();
            let settings = UpscaleStepSettings { backend: backend.into(), model, scale, compress, format: format.into(), quality };
            (StepType::Upscale, serde_json::to_string(&settings).unwrap_or_default())
        }
    }

    pub fn on_pipelines_added(&self) {
        self.refresh();
        self.try_start_runner();
    }

    pub fn refresh(&self) {
        let imp = self.imp();
        let Some(list_box) = imp.queue_list.borrow().clone() else { return };
        
        // Clear existing rows
        while let Some(child) = list_box.first_child() {
            list_box.remove(&child);
        }

        let Some(state_rc) = imp.state.borrow().clone() else { return };
        let state = state_rc.borrow();
        let Some(idx) = state.library_index.as_ref() else { return };

        let in_progress = idx.pipelines_by_status(PipelineStatus::InProgress).unwrap_or_default();
        let queued = idx.pipelines_by_status(PipelineStatus::Queued).unwrap_or_default();
        
        let mut pipelines = in_progress;
        pipelines.extend(queued);

        let queued_count = pipelines.iter().filter(|p| p.status == PipelineStatus::Queued).count();
        let runner_active = imp.runner_active.get();

        if let Some(btn) = imp.start_btn.borrow().as_ref() {
            btn.set_sensitive(!runner_active && queued_count > 0);
        }
        if let Some(btn) = imp.stop_btn.borrow().as_ref() {
            btn.set_sensitive(runner_active);
        }

        for pipeline in pipelines {
            let row = self.build_queue_row(&pipeline, idx);
            list_box.append(&row);
        }

        // --- History ---
        let Some(history_list) = imp.history_list.borrow().clone() else {
            return;
        };
        let Some(history_section) = imp.history_section.borrow().clone() else {
            return;
        };

        while let Some(child) = history_list.first_child() {
            history_list.remove(&child);
        }

        let mut history = idx
            .pipelines_by_status(PipelineStatus::Completed)
            .unwrap_or_default();
        history.extend(
            idx.pipelines_by_status(PipelineStatus::Failed)
                .unwrap_or_default(),
        );
        // Sort by created_at DESC (most recent first)
        history.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        history_section.set_visible(!history.is_empty());

        for pipeline in &history {
            let row = self.build_history_row(pipeline, idx);
            history_list.append(&row);
        }
    }

    fn build_queue_row(&self, pipeline: &Pipeline, idx: &LibraryIndex) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::new();
        let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row_box.set_margin_top(8);
        row_box.set_margin_bottom(8);
        row_box.set_margin_start(8);
        row_box.set_margin_end(8);

        let picture = gtk4::Picture::new();
        picture.set_size_request(48, 48);
        picture.set_content_fit(gtk4::ContentFit::Cover);
        row_box.append(&picture);

        // Load thumbnail
        {
            let picture_c = picture.clone();
            let path = pipeline.source_path.clone();
            glib::spawn_future_local(async move {
                if let Ok(texture) = load_thumbnail_for_row(&path).await {
                    picture_c.set_paintable(Some(&texture));
                }
            });
        }

        let info_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        info_box.set_hexpand(true);
        
        let filename = pipeline.source_path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unknown".to_string());
        let name_label = gtk4::Label::new(Some(&filename));
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        name_label.add_css_class("bold");

        let steps = idx.steps_for_pipeline(pipeline.id).unwrap_or_default();
        let op_type = steps.first().map(|s| match s.step_type {
            StepType::Upscale => "Upscale",
            StepType::Export => "Export",
        }).unwrap_or("Unknown");
        
        let op_label = gtk4::Label::new(Some(op_type));
        op_label.set_halign(gtk4::Align::Start);
        op_label.add_css_class("dim-label");
        op_label.add_css_class("caption");

        info_box.append(&name_label);
        info_box.append(&op_label);

        if pipeline.status == PipelineStatus::InProgress {
            let progress = gtk4::ProgressBar::new();
            progress.set_pulse_step(0.1);
            progress.pulse();
            info_box.append(&progress);
        }

        row_box.append(&info_box);

        let del_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        del_btn.add_css_class("flat");
        del_btn.add_css_class("destructive-action");
        if pipeline.status == PipelineStatus::InProgress {
            del_btn.set_sensitive(false);
        }
        
        {
            let widget_weak = self.downgrade();
            let pid = pipeline.id;
            del_btn.connect_clicked(move |_| {
                if let Some(w) = widget_weak.upgrade() {
                    if let Some(state_rc) = w.imp().state.borrow().as_ref() {
                        if let Some(idx) = state_rc.borrow().library_index.as_ref() {
                            let _ = idx.delete_pipeline(pid);
                        }
                    }
                    w.refresh();
                }
            });
        }
        row_box.append(&del_btn);

        row.set_child(Some(&row_box));
        row
    }

    fn build_history_row(&self, pipeline: &Pipeline, idx: &LibraryIndex) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::new();
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

        // Get the step to find output path and settings
        let steps = idx.steps_for_pipeline(pipeline.id).unwrap_or_default();
        let step = steps.first().cloned();

        // Async-load output thumbnail (if output exists)
        if let Some(output_path) = step.as_ref().and_then(|s| s.output_path.clone()) {
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
        let op_summary = step
            .as_ref()
            .map(format_step_summary)
            .unwrap_or_default();
        let op_label = gtk4::Label::new(Some(&op_summary));
        op_label.set_halign(gtk4::Align::Start);
        op_label.add_css_class("dim-label");
        op_label.add_css_class("caption");
        info_box.append(&op_label);

        // Timestamp
        let ts = format_timestamp(pipeline.created_at);
        let ts_label = gtk4::Label::new(Some(&ts));
        ts_label.set_halign(gtk4::Align::Start);
        ts_label.add_css_class("dim-label");
        ts_label.add_css_class("caption");
        info_box.append(&ts_label);

        // Status + file-move degradation
        let (status_text, status_class) = self.resolve_status_display(pipeline, step.as_ref());
        let status_label = gtk4::Label::new(Some(&status_text));
        status_label.set_halign(gtk4::Align::Start);
        status_label.add_css_class(status_class);
        status_label.add_css_class("caption");
        info_box.append(&status_label);

        row_box.append(&info_box);

        // --- Action buttons ---
        let btn_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        btn_box.set_valign(gtk4::Align::Center);

        // Compare button (navigates to Compare page)
        let compare_btn = gtk4::Button::with_label("Compare");
        compare_btn.add_css_class("flat");
        let output_path_opt = step.as_ref().and_then(|s| s.output_path.clone());
        let can_compare = pipeline.source_path.exists()
            && output_path_opt.as_ref().map(|p| p.exists()).unwrap_or(false);
        compare_btn.set_sensitive(can_compare);
        compare_btn.set_tooltip_text(Some("Open in Compare page"));
        if can_compare {
            let source = pipeline.source_path.clone();
            let output = output_path_opt.clone().unwrap();
            let filename = pipeline
                .source_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Unknown".to_string());
            let widget_weak = self.downgrade();
            compare_btn.connect_clicked(move |_| {
                let Some(w) = widget_weak.upgrade() else {
                    return;
                };
                let imp = w.imp();
                let cb_borrow = imp.compare_cb.borrow();
                if let Some(cb) = cb_borrow.as_ref() {
                    cb(source.clone(), output.clone(), filename.clone());
                }
            });
        }
        btn_box.append(&compare_btn);

        // Re-queue button (failed pipelines only)
        if pipeline.status == PipelineStatus::Failed {
            let requeue_btn = gtk4::Button::with_label("Re-queue");
            requeue_btn.add_css_class("flat");
            let widget_weak = self.downgrade();
            let pid = pipeline.id;
            requeue_btn.connect_clicked(move |_| {
                let Some(w) = widget_weak.upgrade() else {
                    return;
                };
                if let Some(state_rc) = w.imp().state.borrow().as_ref() {
                    if let Some(idx) = state_rc.borrow().library_index.as_ref() {
                        let _ = idx.requeue_pipeline(pid);
                    }
                }
                w.refresh();
                w.try_start_runner();
            });
            btn_box.append(&requeue_btn);
        }

        // Delete button
        let del_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        del_btn.add_css_class("flat");
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
        if imp.polling_timer.borrow().is_some() { return; }

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
        if !imp.runner_active.get() { return; }

        let Some(state_rc) = imp.state.borrow().clone() else {
            imp.runner_active.set(false);
            return;
        };

        let result = {
            let state = state_rc.borrow();
            let idx = match state.library_index.as_ref() {
                Some(i) => i,
                None => { imp.runner_active.set(false); return; }
            };
            
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
            let step = match steps.into_iter().find(|s| s.status == PipelineStatus::Queued) {
                Some(s) => s,
                None => {
                    // Pipeline has no queued steps — mark complete
                    let _ = idx.set_pipeline_status(pipeline.id, PipelineStatus::Completed);
                    drop(state);
                    self.refresh();
                    self.run_next_pipeline();
                    return;
                }
            };

            let _ = idx.set_pipeline_status(pipeline.id, PipelineStatus::InProgress);
            let _ = idx.set_step_status(step.id, PipelineStatus::InProgress, None, None);
            (pipeline, step)
        };

        self.refresh();

        let (pipeline, step) = result;
        let widget_weak = self.downgrade();
        let state_rc_c = state_rc.clone();

        match step.step_type {
            StepType::Export => {
                self.run_export_step(pipeline, step, widget_weak, state_rc_c);
            }
            StepType::Upscale => {
                self.run_upscale_step(pipeline, step, widget_weak, state_rc_c);
            }
        }
    }

    fn run_export_step(
        &self,
        pipeline: Pipeline,
        step: PipelineStep,
        widget_weak: WeakRef<TasksPage>,
        state_rc: Rc<RefCell<AppState>>,
    ) {
        let (tx, rx) = async_channel::bounded::<Result<PathBuf, String>>(1);
        let source = pipeline.source_path.clone();
        let settings_json = step.settings_json.clone();

        let export_output_dir = state_rc.borrow().settings.export_output_dir.clone();

        std::thread::spawn(move || {
            let settings: ExportStepSettings = match serde_json::from_str(&settings_json) {
                Ok(s) => s,
                Err(e) => { let _ = tx.send_blocking(Err(e.to_string())); return; }
            };
            let format = match settings.format.as_str() {
                "webp" => ExportFormat::Webp,
                "png" => ExportFormat::Png,
                "jpeg" => ExportFormat::Jpeg,
                _ => ExportFormat::Jxl,
            };
            
            let dest_dir = resolve_output_dir(
                export_output_dir.as_ref(),
                OutputFolderKind::Export
            );
            
            let output = unique_output_path(&dest_dir, &source, format);
            let result = export_to_path(&source, &output, settings.max_edge, format, settings.quality);
            
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
                                let _ =
                                    idx.set_pipeline_status(pipeline.id, PipelineStatus::Completed);
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
        widget_weak: WeakRef<TasksPage>,
        state_rc: Rc<RefCell<AppState>>,
    ) {
        let (tx, rx) = async_channel::bounded::<Result<PathBuf, String>>(1);
        let source = pipeline.source_path.clone();
        let settings_json = step.settings_json.clone();

        let (upscaler_binary_path, upscaled_output_dir, comfyui_url, comfyui_workflow, onnx_upscale_model) = {
            let st = state_rc.borrow();
            (
                st.settings.upscaler_binary_path.clone()
                    .or_else(crate::upscale::UpscaleDetector::find_realesrgan),
                st.settings.upscaled_output_dir.clone(),
                st.settings.comfyui_url.clone(),
                st.settings.comfyui_workflow.clone(),
                st.settings.onnx_upscale_model.clone(),
            )
        };

        std::thread::spawn(move || {
            let settings: UpscaleStepSettings = match serde_json::from_str(&settings_json) {
                Ok(s) => s,
                Err(e) => { let _ = tx.send_blocking(Err(e.to_string())); return; }
            };

            let backend_kind = UpscaleBackendKind::from_settings(&settings.backend);
            
            if backend_kind == UpscaleBackendKind::ComfyUi {
                let _ = tx.send_blocking(Err("ComfyUI backend not yet supported in queue".to_string()));
                return;
            }

            let model = UpscaleModel::from_settings(&settings.model);
            let format = UpscaleOutputFormat::from_settings(&settings.format);
            
            let job = UpscaleJobConfig {
                source_dimensions: image::image_dimensions(&source).unwrap_or((0, 0)),
                requested_scale: settings.scale,
                execution_scale: if settings.scale == 0 { 4 } else { settings.scale },
                model,
                compress_output: settings.compress,
                compressed_format: format,
                keep_raw_png_sidecar: false,
                compression_mode: UpscaleCompressionMode::Auto,
                quality: settings.quality,
                tile_size: None,
                gpu_id: None,
            };

            let output_dir = resolve_output_dir(
                upscaled_output_dir.as_ref(),
                OutputFolderKind::Upscaled
            );

            let onnx_model = crate::upscale::OnnxUpscaleModel::from_settings(&onnx_upscale_model);
            let comfyui_workflow = crate::upscale::ComfyUiWorkflow::from_settings(&comfyui_workflow);

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
                    "png"  => "png",
                    _      => "jxl",
                }
            } else {
                "png"
            };
            let output_stem = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let output_filename = format!("{}.{}", output_stem, output_ext);
            let rx_events = backend.run(source.clone(), output_dir.join(&output_filename), job);
            
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
                                let _ =
                                    idx.set_pipeline_status(pipeline.id, PipelineStatus::Completed);
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
            ).upcast())
        })();
        let _ = tx.send_blocking(result);
    });

    rx.recv().await.unwrap_or_else(|_| Err("Thumbnail thread died".to_string()))
}

fn format_step_summary(step: &PipelineStep) -> String {
    match step.step_type {
        StepType::Upscale => {
            if let Ok(s) = serde_json::from_str::<UpscaleStepSettings>(&step.settings_json) {
                let scale = if s.scale == 0 {
                    "auto".to_string()
                } else {
                    format!("{}×", s.scale)
                };
                format!("Upscale · {} · {}", s.model, scale)
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
