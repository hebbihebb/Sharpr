use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc};

use glib::WeakRef;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;

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
    #[serde(default)]
    pub onnx_model: Option<String>,
    pub scale: u32,            // 0 = smart/auto, 2, 3, 4
    pub compress: bool,
    pub format: String,        // "jxl" | "webp" | "jpeg" | "png"
    pub quality: u8,
    #[serde(default)]
    pub keep_png: bool,
    #[serde(default = "default_destination")]
    pub destination: String,   // "default" | "source" | "custom"
    #[serde(default)]
    pub custom_path: Option<PathBuf>,
}

fn default_destination() -> String {
    "default".to_string()
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ExportStepSettings {
    pub format: String,        // "jxl" | "webp" | "png" | "jpeg"
    pub max_edge: Option<u32>, // None = original size
    pub quality: u8,
    #[serde(default = "default_destination")]
    pub destination: String,   // "default" | "source" | "custom"
    #[serde(default)]
    pub custom_path: Option<PathBuf>,
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
        pub op_upscale_btn: RefCell<Option<gtk4::ToggleButton>>,
        pub op_export_btn: RefCell<Option<gtk4::ToggleButton>>,
        pub crash_banner: RefCell<Option<libadwaita::Banner>>,
        pub queue_empty_label: RefCell<Option<gtk4::Label>>,

        // Upscale settings widgets
        pub backend_onnx_btn: RefCell<Option<gtk4::ToggleButton>>,
        pub backend_comfyui_btn: RefCell<Option<gtk4::ToggleButton>>,
        pub backend_cli_btn: RefCell<Option<gtk4::ToggleButton>>,
        pub onnx_model_dropdown: RefCell<Option<libadwaita::ComboRow>>,
        pub onnx_model_row: RefCell<Option<libadwaita::ComboRow>>,
        pub scale_dropdown: RefCell<Option<libadwaita::ComboRow>>,
        pub compress_check: RefCell<Option<libadwaita::SwitchRow>>,
        pub keep_png_check: RefCell<Option<libadwaita::SwitchRow>>,
        pub format_dropdown: RefCell<Option<libadwaita::ComboRow>>,
        pub quality_spin: RefCell<Option<gtk4::SpinButton>>,
        pub upscale_dest_dropdown: RefCell<Option<libadwaita::ComboRow>>,

        // Export settings widgets
        pub export_format_dropdown: RefCell<Option<libadwaita::ComboRow>>,
        pub export_edge_dropdown: RefCell<Option<libadwaita::ComboRow>>,
        pub export_quality_spin: RefCell<Option<gtk4::SpinButton>>,
        pub export_dest_dropdown: RefCell<Option<libadwaita::ComboRow>>,

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

            let crash_banner = libadwaita::Banner::new("Unfinished jobs from previous session detected.");
            crash_banner.set_button_label(Some("Resume All"));
            crash_banner.set_revealed(false);
            left_col.append(&crash_banner);

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

            let queue_empty_label = gtk4::Label::new(Some("Queue is empty"));
            queue_empty_label.add_css_class("dim-label");
            queue_empty_label.set_margin_top(20);
            queue_empty_label.set_margin_bottom(20);
            queue_empty_label.set_visible(false);
            
            let queue_overlay = gtk4::Overlay::new();
            queue_overlay.set_child(Some(&scrolled));
            queue_overlay.add_overlay(&queue_empty_label);

            left_col.append(&toolbar);
            left_col.append(&queue_overlay);

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

            let op_switcher = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            op_switcher.add_css_class("linked");
            let op_upscale_btn = gtk4::ToggleButton::with_label("Upscale");
            let op_export_btn = gtk4::ToggleButton::with_label("Export");
            op_switcher.append(&op_upscale_btn);
            op_switcher.append(&op_export_btn);

            let settings_stack = gtk4::Stack::new();
            settings_stack.set_transition_type(gtk4::StackTransitionType::SlideLeftRight);

            // Upscale Settings Page
            let upscale_group = libadwaita::PreferencesGroup::new();

            let backend_row = libadwaita::ActionRow::new();
            backend_row.set_title("Backend");
            let backend_switcher = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            backend_switcher.add_css_class("linked");
            backend_switcher.set_valign(gtk4::Align::Center);
            let backend_onnx_btn = gtk4::ToggleButton::with_label("ONNX");
            let backend_comfyui_btn = gtk4::ToggleButton::with_label("ComfyUI");
            let backend_cli_btn = gtk4::ToggleButton::with_label("External CLI");
            backend_switcher.append(&backend_onnx_btn);
            backend_switcher.append(&backend_comfyui_btn);
            backend_switcher.append(&backend_cli_btn);
            backend_row.add_suffix(&backend_switcher);

            let onnx_model_row = libadwaita::ComboRow::new();
            onnx_model_row.set_title("Model");
            let onnx_model_list = gtk4::StringList::new(&[
                "Lightweight ×2 — 8 MB",
                "Compressed ×4 — 55 MB",
                "Realworld ×4 — 53 MB",
            ]);
            onnx_model_row.set_model(Some(&onnx_model_list));
            let onnx_model_dropdown = onnx_model_row.clone();

            let scale_row = libadwaita::ComboRow::new();
            scale_row.set_title("Scale");
            let scale_model = gtk4::StringList::new(&["Auto (Smart)", "2x", "3x", "4x"]);
            scale_row.set_model(Some(&scale_model));
            let scale_dropdown = scale_row.clone();

            let compress_check = libadwaita::SwitchRow::new();
            compress_check.set_title("Compress output");

            let format_row = libadwaita::ComboRow::new();
            format_row.set_title("Format");
            let format_model = gtk4::StringList::new(&["JXL", "WebP", "JPEG", "PNG"]);
            format_row.set_model(Some(&format_model));
            let format_dropdown = format_row.clone();

            let quality_row = libadwaita::ActionRow::new();
            quality_row.set_title("Quality");
            let quality_adj = gtk4::Adjustment::new(85.0, 1.0, 100.0, 1.0, 10.0, 0.0);
            let quality_spin = gtk4::SpinButton::new(Some(&quality_adj), 1.0, 0);
            quality_spin.set_valign(gtk4::Align::Center);
            quality_row.add_suffix(&quality_spin);
            quality_row.set_activatable_widget(Some(&quality_spin));

            let keep_png_check = libadwaita::SwitchRow::new();
            keep_png_check.set_title("Keep raw PNG sidecar");

            let upscale_dest_row = libadwaita::ComboRow::new();
            upscale_dest_row.set_title("Destination");
            let upscale_dest_model = gtk4::StringList::new(&["Default (Pictures/Upscaled)", "Same as source"]);
            upscale_dest_row.set_model(Some(&upscale_dest_model));
            let upscale_dest_dropdown = upscale_dest_row.clone();

            upscale_group.add(&backend_row);
            upscale_group.add(&onnx_model_row);
            upscale_group.add(&scale_row);
            upscale_group.add(&compress_check);
            upscale_group.add(&format_row);
            upscale_group.add(&quality_row);
            upscale_group.add(&keep_png_check);
            upscale_group.add(&upscale_dest_row);

            settings_stack.add_named(&upscale_group, Some("upscale"));

            // Export Settings Page
            let export_group = libadwaita::PreferencesGroup::new();

            let export_format_row = libadwaita::ComboRow::new();
            export_format_row.set_title("Format");
            let export_format_model = gtk4::StringList::new(&["JXL", "WebP", "PNG", "JPEG"]);
            export_format_row.set_model(Some(&export_format_model));
            let export_format_dropdown = export_format_row.clone();

            let export_edge_row = libadwaita::ComboRow::new();
            export_edge_row.set_title("Max Edge");
            let export_edge_model = gtk4::StringList::new(&["Original", "1080px", "2160px", "4096px"]);
            export_edge_row.set_model(Some(&export_edge_model));
            let export_edge_dropdown = export_edge_row.clone();

            let export_quality_row = libadwaita::ActionRow::new();
            export_quality_row.set_title("Quality");
            let export_quality_adj = gtk4::Adjustment::new(85.0, 1.0, 100.0, 1.0, 10.0, 0.0);
            let export_quality_spin = gtk4::SpinButton::new(Some(&export_quality_adj), 1.0, 0);
            export_quality_spin.set_valign(gtk4::Align::Center);
            export_quality_row.add_suffix(&export_quality_spin);
            export_quality_row.set_activatable_widget(Some(&export_quality_spin));

            let export_dest_row = libadwaita::ComboRow::new();
            export_dest_row.set_title("Destination");
            let export_dest_model = gtk4::StringList::new(&["Default (Pictures/Export)", "Same as source"]);
            export_dest_row.set_model(Some(&export_dest_model));
            let export_dest_dropdown = export_dest_row.clone();

            export_group.add(&export_format_row);
            export_group.add(&export_edge_row);
            export_group.add(&export_quality_row);
            export_group.add(&export_dest_row);

            settings_stack.add_named(&export_group, Some("export"));

            right_col.append(&op_switcher);
            right_col.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
            right_col.append(&settings_stack);

            main_box.append(&left_col);
            main_box.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
            main_box.append(&right_col);

            // Wire operation switcher to stack
            {
                let settings_stack_c = settings_stack.clone();
                let export_btn = op_export_btn.clone();
                op_upscale_btn.connect_toggled(move |btn| {
                    if btn.is_active() {
                        if export_btn.is_active() {
                            export_btn.set_active(false);
                        }
                        settings_stack_c.set_visible_child_name("upscale");
                    } else if !export_btn.is_active() {
                        btn.set_active(true);
                    }
                });
            }
            {
                let settings_stack_c = settings_stack.clone();
                let upscale_btn = op_upscale_btn.clone();
                op_export_btn.connect_toggled(move |btn| {
                    if btn.is_active() {
                        if upscale_btn.is_active() {
                            upscale_btn.set_active(false);
                        }
                        settings_stack_c.set_visible_child_name("export");
                    } else if !upscale_btn.is_active() {
                        btn.set_active(true);
                    }
                });
            }

            // Wire backend switcher and dependent row visibility
            {
                let comfy_btn = backend_comfyui_btn.clone();
                let cli_btn = backend_cli_btn.clone();
                let onnx_row = onnx_model_row.clone();
                backend_onnx_btn.connect_toggled(move |btn| {
                    if btn.is_active() {
                        if comfy_btn.is_active() {
                            comfy_btn.set_active(false);
                        }
                        if cli_btn.is_active() {
                            cli_btn.set_active(false);
                        }
                        onnx_row.set_visible(true);
                    } else if !comfy_btn.is_active() && !cli_btn.is_active() {
                        btn.set_active(true);
                    } else {
                        onnx_row.set_visible(false);
                    }
                });
            }
            {
                let onnx_btn = backend_onnx_btn.clone();
                let cli_btn = backend_cli_btn.clone();
                let onnx_row = onnx_model_row.clone();
                backend_comfyui_btn.connect_toggled(move |btn| {
                    if btn.is_active() {
                        if onnx_btn.is_active() {
                            onnx_btn.set_active(false);
                        }
                        if cli_btn.is_active() {
                            cli_btn.set_active(false);
                        }
                        onnx_row.set_visible(false);
                    } else if !onnx_btn.is_active() && !cli_btn.is_active() {
                        btn.set_active(true);
                    }
                });
            }
            {
                let onnx_btn = backend_onnx_btn.clone();
                let comfy_btn = backend_comfyui_btn.clone();
                let onnx_row = onnx_model_row.clone();
                backend_cli_btn.connect_toggled(move |btn| {
                    if btn.is_active() {
                        if onnx_btn.is_active() {
                            onnx_btn.set_active(false);
                        }
                        if comfy_btn.is_active() {
                            comfy_btn.set_active(false);
                        }
                        onnx_row.set_visible(false);
                    } else if !onnx_btn.is_active() && !comfy_btn.is_active() {
                        btn.set_active(true);
                    }
                });
            }

            // Wire compression row to dependent options visibility
            {
                let format_row_c = format_row.clone();
                let quality_row_c = quality_row.clone();
                compress_check.connect_active_notify(move |row| {
                    let visible = row.is_active();
                    format_row_c.set_visible(visible);
                    quality_row_c.set_visible(visible);
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

            {
                let widget_weak = widget.downgrade();
                crash_banner.connect_button_clicked(move |_| {
                    if let Some(w) = widget_weak.upgrade() {
                        w.imp().crash_banner.borrow().as_ref().map(|b| b.set_revealed(false));
                        w.try_start_runner();
                    }
                });
            }

            *self.queue_list.borrow_mut() = Some(queue_list);
            *self.queue_empty_label.borrow_mut() = Some(queue_empty_label);
            *self.settings_stack.borrow_mut() = Some(settings_stack);
            *self.start_btn.borrow_mut() = Some(start_btn);
            *self.stop_btn.borrow_mut() = Some(stop_btn);
            *self.op_upscale_btn.borrow_mut() = Some(op_upscale_btn);
            *self.op_export_btn.borrow_mut() = Some(op_export_btn);
            *self.crash_banner.borrow_mut() = Some(crash_banner);

            *self.backend_onnx_btn.borrow_mut() = Some(backend_onnx_btn);
            *self.backend_comfyui_btn.borrow_mut() = Some(backend_comfyui_btn);
            *self.backend_cli_btn.borrow_mut() = Some(backend_cli_btn);
            *self.onnx_model_dropdown.borrow_mut() = Some(onnx_model_dropdown);
            *self.onnx_model_row.borrow_mut() = Some(onnx_model_row);
            *self.scale_dropdown.borrow_mut() = Some(scale_dropdown);
            *self.compress_check.borrow_mut() = Some(compress_check);
            *self.keep_png_check.borrow_mut() = Some(keep_png_check);
            *self.format_dropdown.borrow_mut() = Some(format_dropdown);
            *self.quality_spin.borrow_mut() = Some(quality_spin);
            *self.upscale_dest_dropdown.borrow_mut() = Some(upscale_dest_dropdown);

            *self.export_format_dropdown.borrow_mut() = Some(export_format_dropdown);
            *self.export_edge_dropdown.borrow_mut() = Some(export_edge_dropdown);
            *self.export_quality_spin.borrow_mut() = Some(export_quality_spin);
            *self.export_dest_dropdown.borrow_mut() = Some(export_dest_dropdown);

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

    pub fn set_interrupted_count(&self, n: usize) {
        if n == 0 { return; }
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

    fn operation_is_export(&self) -> bool {
        self.imp()
            .op_export_btn
            .borrow()
            .as_ref()
            .map(|btn| btn.is_active())
            .unwrap_or(false)
    }

    fn set_operation(&self, operation: &str) {
        let imp = self.imp();
        let is_export = operation == "export";
        if let Some(btn) = imp.op_export_btn.borrow().as_ref() {
            btn.set_active(is_export);
        }
        if let Some(btn) = imp.op_upscale_btn.borrow().as_ref() {
            btn.set_active(!is_export);
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
    }

    pub fn set_state(&self, state: Rc<RefCell<AppState>>) {
        let imp = self.imp();

        // Load initial defaults from settings
        {
            let st = state.borrow();
            self.set_operation("upscale");
            self.set_backend(st.settings.upscale_backend.as_str());
            if let Some(onnx_dd) = imp.onnx_model_dropdown.borrow().as_ref() {
                let idx = match st.settings.onnx_upscale_model.as_str() {
                    "swin2sr-compressed-x4" => 1,
                    "swin2sr-real-x4" => 2,
                    _ => 0, // lightweight-x2
                };
                onnx_dd.set_selected(idx);
            }
            if let Some(scale_dd) = imp.scale_dropdown.borrow().as_ref() {
                scale_dd.set_selected(0);
            }
            if let Some(compress) = imp.compress_check.borrow().as_ref() {
                compress.set_active(st.settings.upscale_compress_output);
            }
            if let Some(keep_png) = imp.keep_png_check.borrow().as_ref() {
                keep_png.set_active(st.settings.upscale_keep_raw_png_sidecar);
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
                spin.set_value(st.settings.upscaler_quality as f64);
            }
            if let Some(dest_dd) = imp.upscale_dest_dropdown.borrow().as_ref() {
                dest_dd.set_selected(0);
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
            if let Some(dest_dd) = imp.export_dest_dropdown.borrow().as_ref() {
                dest_dd.set_selected(0);
            }
        }

        // Wire backend toggles to persist the selection back to AppSettings
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

        *imp.state.borrow_mut() = Some(state.clone());
        self.refresh();

        // Reveal crash recovery banner if there are unfinished pipelines
        {
            let n = state.borrow().library_index_interrupted_count;
            self.set_interrupted_count(n);
        }

        self.try_start_runner();
    }

    pub fn set_compare_requested_cb<F: Fn(PathBuf, PathBuf, String) + 'static>(&self, f: F) {
        *self.imp().compare_cb.borrow_mut() = Some(Box::new(f));
    }

    /// Returns the step type and settings JSON currently shown in the settings panel.
    /// Used by window.rs when adding a job from the filmstrip.
    pub fn current_step_config(&self) -> (StepType, String) {
        let imp = self.imp();
        if self.operation_is_export() {
            // Export
            let format = imp.export_format_dropdown.borrow().as_ref().map(|d| match d.selected() {
                1 => "webp", 2 => "png", 3 => "jpeg", _ => "jxl",
            }).unwrap_or("jxl");
            let max_edge = imp.export_edge_dropdown.borrow().as_ref().and_then(|d| match d.selected() {
                1 => Some(1080u32), 2 => Some(2160), 3 => Some(4096), _ => None,
            });
            let quality = imp.export_quality_spin.borrow().as_ref().map(|s| s.value() as u8).unwrap_or(85);
            let destination = imp.export_dest_dropdown.borrow().as_ref().map(|d| match d.selected() {
                1 => "source",
                _ => "default",
            }).unwrap_or("default");
            let settings = ExportStepSettings { format: format.into(), max_edge, quality, destination: destination.into(), custom_path: None };
            (StepType::Export, serde_json::to_string(&settings).unwrap_or_default())
        } else {
            // Upscale (default)
            let backend = self.selected_backend();
            let onnx_model = if backend == "onnx" {
                imp.onnx_model_dropdown.borrow().as_ref().map(|d| match d.selected() {
                    1 => "swin2sr-compressed-x4",
                    2 => "swin2sr-real-x4",
                    _ => "swin2sr-lightweight-x2",
                })
            } else {
                None
            };
            let scale = imp.scale_dropdown.borrow().as_ref().map(|d| match d.selected() {
                1 => 2u32, 2 => 3, 3 => 4, _ => 0,
            }).unwrap_or(0);
            let compress = imp.compress_check.borrow().as_ref().map(|c| c.is_active()).unwrap_or(false);
            let keep_png = imp.keep_png_check.borrow().as_ref().map(|c| c.is_active()).unwrap_or(false);
            let format = imp.format_dropdown.borrow().as_ref().map(|d| match d.selected() {
                1 => "webp", 2 => "jpeg", 3 => "png", _ => "jxl",
            }).unwrap_or("jxl");
            let quality = imp.quality_spin.borrow().as_ref().map(|s| s.value() as u8).unwrap_or(85);
            let model = imp.state.borrow().as_ref()
                .map(|s| s.borrow().settings.upscaler_default_model.clone())
                .unwrap_or_default();
            let destination = imp.upscale_dest_dropdown.borrow().as_ref().map(|d| match d.selected() {
                1 => "source",
                _ => "default",
            }).unwrap_or("default");
            let settings = UpscaleStepSettings { 
                backend: backend.into(), 
                model, 
                onnx_model: onnx_model.map(|s| s.to_string()),
                scale, 
                compress, 
                format: format.into(), 
                quality,
                keep_png,
                destination: destination.into(),
                custom_path: None
            };
            (StepType::Upscale, serde_json::to_string(&settings).unwrap_or_default())
        }
    }

    pub fn pre_fill_from_path(&self, path: PathBuf) {
        if let Some(state_rc) = self.imp().state.borrow().as_ref() {
            if let Some(idx) = state_rc.borrow().library_index.as_ref() {
                let (step_type, settings_json) = self.current_step_config();
                if let Ok(pid) = idx.create_pipeline(&path) {
                    let _ = idx.append_pipeline_step(pid, step_type, &settings_json);
                }
            }
        }
        self.refresh();
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

        if let Some(lbl) = imp.queue_empty_label.borrow().as_ref() {
            lbl.set_visible(pipelines.is_empty());
        }

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

        if pipeline.status != PipelineStatus::InProgress {
            let del_btn = gtk4::Button::from_icon_name("window-close-symbolic");
            del_btn.add_css_class("flat");
            del_btn.add_css_class("destructive-action");
            
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
        }

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
            
            let dest_dir = if settings.destination == "source" {
                source.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
            } else if settings.destination == "custom" {
                settings.custom_path.clone().unwrap_or_else(|| {
                    resolve_output_dir(export_output_dir.as_ref(), OutputFolderKind::Export)
                })
            } else {
                resolve_output_dir(export_output_dir.as_ref(), OutputFolderKind::Export)
            };
            
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
                keep_raw_png_sidecar: settings.keep_png,
                compression_mode: UpscaleCompressionMode::Auto,
                quality: settings.quality,
                tile_size: None,
                gpu_id: None,
            };

            let dest_dir = if settings.destination == "source" {
                source.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
            } else if settings.destination == "custom" {
                settings.custom_path.clone().unwrap_or_else(|| {
                    resolve_output_dir(upscaled_output_dir.as_ref(), OutputFolderKind::Upscaled)
                })
            } else {
                resolve_output_dir(upscaled_output_dir.as_ref(), OutputFolderKind::Upscaled)
            };

            let onnx_model = settings.onnx_model
                .map(|s| crate::upscale::OnnxUpscaleModel::from_settings(&s))
                .unwrap_or_else(|| crate::upscale::OnnxUpscaleModel::from_settings(&onnx_upscale_model));
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
            let output_filename = crate::export::unique_output_path_for_extension(&dest_dir, &source, output_ext);
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
