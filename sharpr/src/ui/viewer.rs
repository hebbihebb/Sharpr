use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Once};

use crate::quality::{scorer, QualityScore};
use crate::tags::TagDatabase;
use crate::ui::metadata_chip::MetadataChip;
use crate::ui::state::AppState;

const TAG_CHIP_COLOR_PALETTE: &[&str] = &[
    "#57e389", "#62a0ea", "#ff7800", "#f5c211", "#dc8add", "#5bc8af", "#e01b24", "#9141ac",
];

fn fallback_collection_color(collection_id: i64) -> &'static str {
    TAG_CHIP_COLOR_PALETTE[(collection_id as usize) % TAG_CHIP_COLOR_PALETTE.len()]
}

fn root_collection_id(collection_id: i64, parent_by_id: &HashMap<i64, Option<i64>>) -> i64 {
    let mut root_id = collection_id;
    let mut current = parent_by_id.get(&collection_id).copied().flatten();
    while let Some(parent_id) = current {
        root_id = parent_id;
        current = parent_by_id.get(&parent_id).copied().flatten();
    }
    root_id
}

fn collection_tag_color_map(
    collections: &[crate::library_index::Collection],
) -> HashMap<String, String> {
    let parent_by_id: HashMap<i64, Option<i64>> = collections
        .iter()
        .map(|collection| (collection.id, collection.parent_id))
        .collect();
    let collection_by_id: HashMap<i64, &crate::library_index::Collection> = collections
        .iter()
        .map(|collection| (collection.id, collection))
        .collect();
    let mut tag_to_color = HashMap::new();

    for collection in collections {
        let root_id = root_collection_id(collection.id, &parent_by_id);
        let resolved_color = collection_by_id
            .get(&root_id)
            .and_then(|root| root.color.clone())
            .unwrap_or_else(|| fallback_collection_color(root_id).to_string());

        tag_to_color.insert(collection.primary_tag.clone(), resolved_color.clone());
        for tag in &collection.extra_tags {
            tag_to_color.insert(tag.clone(), resolved_color.clone());
        }
    }

    tag_to_color
}

fn apply_tag_icon_tint(icon: &gtk4::Image, color: &str) {
    use std::sync::{LazyLock, Mutex};

    static REGISTERED: LazyLock<Mutex<std::collections::HashSet<String>>> =
        LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

    let key = color
        .replace([' ', '(', ')', ',', '#', '.'], "_")
        .to_lowercase();
    let class_name = format!("tag-icon-color-{key}");

    if let Ok(mut registered) = REGISTERED.lock() {
        if registered.insert(key) {
            let provider = gtk4::CssProvider::new();
            provider.load_from_string(&format!(".{class_name} {{ color: {color}; }}"));
            if let Some(display) = gtk4::gdk::Display::default() {
                gtk4::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk4::STYLE_PROVIDER_PRIORITY_USER,
                );
            }
        }
    }

    icon.add_css_class(&class_name);
}

fn apply_tag_osd_chip_tint(btn: &gtk4::Button, color: &str) {
    use std::sync::{LazyLock, Mutex};

    static REGISTERED: LazyLock<Mutex<std::collections::HashSet<String>>> =
        LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

    let hex = color.trim_start_matches('#');
    if hex.len() != 6 {
        return;
    }
    let (Ok(r), Ok(g), Ok(b)) = (
        u8::from_str_radix(&hex[0..2], 16),
        u8::from_str_radix(&hex[2..4], 16),
        u8::from_str_radix(&hex[4..6], 16),
    ) else {
        return;
    };

    let key = hex.to_lowercase();
    let class_name = format!("tag-osd-chip-color-{key}");

    if let Ok(mut registered) = REGISTERED.lock() {
        if registered.insert(key) {
            let provider = gtk4::CssProvider::new();
            provider.load_from_string(&format!(
                ".{class_name} {{ background-color: rgba({r},{g},{b},0.35); }}"
            ));
            if let Some(display) = gtk4::gdk::Display::default() {
                gtk4::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
        }
    }
    btn.add_css_class(&class_name);
}

// ---------------------------------------------------------------------------
// ViewerPane
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ZoomMode {
    Fit,
    OneToOne,
}

#[derive(Default)]
struct TagPopoverState {
    db: Option<Arc<TagDatabase>>,
    path: Option<PathBuf>,
}

mod imp {
    use super::*;

    pub struct ViewerPane {
        pub content_box: gtk4::Box,
        pub overlay: gtk4::Overlay,
        pub scrolled_window: gtk4::ScrolledWindow,
        pub picture: gtk4::Picture,
        pub metadata_chip: MetadataChip,
        pub tag_osd: gtk4::Box,
        pub tag_chips_box: gtk4::Box,
        pub tag_label: gtk4::Label,
        pub tag_button: gtk4::Button,
        pub tag_add_button: gtk4::Button,
        pub spinner: gtk4::Spinner,
        /// Shown briefly when a decode error prevents the image from loading.
        pub error_label: gtk4::Label,
        pub zoom_label: gtk4::Label,
        pub zoom_hide_source: RefCell<Option<glib::SourceId>>,
        pub tag_popover: gtk4::Popover,
        pub tag_entry: gtk4::SearchEntry,
        pub selected_list: gtk4::ListBox,
        pub all_tags_list: gtk4::ListBox,
        pub new_tag_btn: gtk4::Button,
        pub new_tag_entry: gtk4::Entry,
        pub new_tag_create_btn: gtk4::Button,
        pub new_tag_cancel_btn: gtk4::Button,
        pub(super) tag_state: Rc<RefCell<TagPopoverState>>,
        /// Path of the image currently displayed — set by load_image(), cleared by clear().
        pub current_path: RefCell<Option<PathBuf>>,
        /// Zoom/Fit toggle button in the header — stored so mode changes can
        /// update the icon without an extra signal.
        pub zoom_btn: std::cell::RefCell<Option<gtk4::Button>>,
        pub zoom: Cell<f64>,
        pub zoom_mode: Cell<super::ZoomMode>,
        pub metadata_visible: Cell<bool>,
        pub tag_chips_expanded: Cell<bool>,
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
        pub pointer_pos: Cell<(f64, f64)>,
        pub drag_origin: Cell<Option<(f64, f64)>>,
        pub drag_adjustments: Cell<(f64, f64)>,
        /// Monotonically increasing counter; each `load_image` call increments it.
        /// Async callbacks capture the value at dispatch time and discard their
        /// result if it no longer matches (i.e. a newer load was requested).
        pub load_gen: Cell<u64>,
        pub tag_refresh_gen: Cell<u64>,
        pub info_pending: Cell<bool>,
        pub tag_summary_gen: Cell<u64>,
        /// Handle for submitting metadata load requests to the shared metadata worker.
        pub metadata_handle: RefCell<Option<crate::image_pipeline::worker::MetadataHandle>>,
        /// Called when the "Manage tags" button in the popover is clicked.
        /// Last metadata-only quality score set for the displayed image, kept so
        /// `apply_sharpness` can blend without re-reading metadata.
        pub base_quality: RefCell<Option<crate::quality::QualityScore>>,
    }

    impl Default for ViewerPane {
        fn default() -> Self {
            let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

            let picture = gtk4::Picture::new();
            picture.set_content_fit(gtk4::ContentFit::Contain);
            picture.set_can_shrink(true);
            picture.set_halign(gtk4::Align::Center);
            picture.set_valign(gtk4::Align::Center);

            let scrolled_window = gtk4::ScrolledWindow::new();
            scrolled_window.set_hexpand(true);
            scrolled_window.set_vexpand(true);
            scrolled_window.set_hscrollbar_policy(gtk4::PolicyType::Automatic);
            scrolled_window.set_vscrollbar_policy(gtk4::PolicyType::Automatic);
            scrolled_window.set_child(Some(&picture));

            let spinner = gtk4::Spinner::new();
            spinner.set_halign(gtk4::Align::Center);
            spinner.set_valign(gtk4::Align::Center);
            spinner.set_size_request(48, 48);
            spinner.set_visible(false);

            let error_label = gtk4::Label::new(None);
            error_label.add_css_class("osd");
            error_label.set_halign(gtk4::Align::Center);
            error_label.set_valign(gtk4::Align::Center);
            error_label.set_wrap(true);
            error_label.set_visible(false);

            let metadata_chip = MetadataChip::new();
            metadata_chip.set_halign(gtk4::Align::End);
            metadata_chip.set_valign(gtk4::Align::End);
            metadata_chip.set_margin_end(16);
            metadata_chip.set_margin_bottom(4);

            let tag_osd = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            tag_osd.add_css_class("osd");
            tag_osd.add_css_class("tag-osd");
            tag_osd.set_halign(gtk4::Align::Start);
            tag_osd.set_valign(gtk4::Align::End);
            tag_osd.set_margin_start(16);
            tag_osd.set_margin_bottom(16);
            tag_osd.set_visible(false);

            let tag_chips_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

            let tag_button = gtk4::Button::new();
            tag_button.add_css_class("flat");
            tag_button.add_css_class("tag-osd-pill");
            tag_button.set_focus_on_click(false);

            let tag_label = gtk4::Label::new(Some("Tags"));
            tag_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            tag_label.set_max_width_chars(18);
            tag_label.set_xalign(0.0);
            tag_button.set_child(Some(&tag_label));

            let tag_add_button = gtk4::Button::from_icon_name("list-add-symbolic");
            tag_add_button.add_css_class("flat");
            tag_add_button.add_css_class("tag-osd-add");
            tag_add_button.set_focus_on_click(false);
            tag_add_button.set_tooltip_text(Some("Add or edit tags"));

            tag_osd.append(&tag_chips_box);
            tag_osd.append(&tag_add_button);

            let zoom_label = gtk4::Label::new(None);
            zoom_label.add_css_class("osd");
            zoom_label.add_css_class("title-2");
            zoom_label.add_css_class("zoom-osd");
            zoom_label.set_halign(gtk4::Align::Center);
            zoom_label.set_valign(gtk4::Align::Center);
            zoom_label.set_margin_start(24);
            zoom_label.set_margin_end(24);
            zoom_label.set_margin_top(16);
            zoom_label.set_margin_bottom(16);
            zoom_label.set_visible(false);

            let tag_popover = gtk4::Popover::new();
            tag_popover.set_has_arrow(true);
            tag_popover.set_autohide(true);
            tag_popover.set_position(gtk4::PositionType::Top);
            tag_popover.set_parent(&tag_add_button);
            tag_popover.add_css_class("tag-popover");

            let popover_vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            popover_vbox.set_margin_start(12);
            popover_vbox.set_margin_end(12);
            popover_vbox.set_margin_top(12);
            popover_vbox.set_margin_bottom(12);
            popover_vbox.set_width_request(260);

            let tag_entry = gtk4::SearchEntry::new();
            tag_entry.set_placeholder_text(Some("Search tags…"));
            tag_entry.set_hexpand(true);
            popover_vbox.append(&tag_entry);

            let popover_scroll = gtk4::ScrolledWindow::new();
            popover_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
            popover_scroll.set_min_content_height(200);
            popover_scroll.set_max_content_height(400);

            let lists_vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
            popover_scroll.set_child(Some(&lists_vbox));
            popover_vbox.append(&popover_scroll);

            let selected_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            let selected_header = gtk4::Label::new(Some("SELECTED"));
            selected_header.set_halign(gtk4::Align::Start);
            selected_header.add_css_class("dim-label");
            selected_header.add_css_class("caption-heading");
            let selected_list = gtk4::ListBox::new();
            selected_list.set_selection_mode(gtk4::SelectionMode::None);
            selected_list.add_css_class("rich-list");
            selected_box.append(&selected_header);
            selected_box.append(&selected_list);
            lists_vbox.append(&selected_box);

            let all_tags_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            let all_tags_header = gtk4::Label::new(Some("ALL TAGS"));
            all_tags_header.set_halign(gtk4::Align::Start);
            all_tags_header.add_css_class("dim-label");
            all_tags_header.add_css_class("caption-heading");
            let all_tags_list = gtk4::ListBox::new();
            all_tags_list.set_selection_mode(gtk4::SelectionMode::None);
            all_tags_list.add_css_class("rich-list");
            all_tags_box.append(&all_tags_header);
            all_tags_box.append(&all_tags_list);
            lists_vbox.append(&all_tags_box);

            let action_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            action_bar.set_margin_top(4);

            let new_tag_btn = gtk4::Button::with_label("+ New tag");
            new_tag_btn.add_css_class("flat");
            new_tag_btn.set_halign(gtk4::Align::Start);

            let new_tag_entry = gtk4::Entry::new();
            new_tag_entry.set_placeholder_text(Some("Tag name"));
            new_tag_entry.set_visible(false);
            new_tag_entry.set_hexpand(true);

            let new_tag_create_btn = gtk4::Button::with_label("Create");
            new_tag_create_btn.add_css_class("suggested-action");
            new_tag_create_btn.set_visible(false);

            let new_tag_cancel_btn = gtk4::Button::with_label("Cancel");
            new_tag_cancel_btn.add_css_class("flat");
            new_tag_cancel_btn.set_visible(false);

            action_bar.append(&new_tag_btn);
            action_bar.append(&new_tag_entry);
            action_bar.append(&new_tag_cancel_btn);
            action_bar.append(&new_tag_create_btn);

            popover_vbox.append(&action_bar);
            tag_popover.set_child(Some(&popover_vbox));

            let tag_state = Rc::new(RefCell::new(TagPopoverState::default()));

            let overlay = gtk4::Overlay::new();
            overlay.set_child(Some(&scrolled_window));
            overlay.add_overlay(&tag_osd);
            overlay.add_overlay(&metadata_chip);
            overlay.add_overlay(&spinner);
            overlay.add_overlay(&error_label);
            overlay.add_overlay(&zoom_label);

            content_box.append(&overlay);

            Self {
                content_box,
                overlay,
                scrolled_window,
                picture,
                metadata_chip,
                tag_osd,
                tag_chips_box,
                tag_label,
                tag_button,
                tag_add_button,
                spinner,
                error_label,
                zoom_label,
                zoom_hide_source: RefCell::new(None),
                tag_popover,
                tag_entry,
                selected_list,
                all_tags_list,
                new_tag_btn,
                new_tag_entry,
                new_tag_create_btn,
                new_tag_cancel_btn,
                tag_state,
                current_path: RefCell::new(None),
                zoom_btn: std::cell::RefCell::new(None),
                zoom: Cell::new(1.0),
                zoom_mode: Cell::new(super::ZoomMode::Fit),
                metadata_visible: Cell::new(true),
                tag_chips_expanded: Cell::new(false),
                state: RefCell::new(None),
                pointer_pos: Cell::new((0.0, 0.0)),
                drag_origin: Cell::new(None),
                drag_adjustments: Cell::new((0.0, 0.0)),
                load_gen: Cell::new(0),
                tag_refresh_gen: Cell::new(0),
                info_pending: Cell::new(false),
                tag_summary_gen: Cell::new(0),
                metadata_handle: RefCell::new(None),
                base_quality: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ViewerPane {
        const NAME: &'static str = "SharprViewerPane";
        type Type = super::ViewerPane;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for ViewerPane {
        fn dispose(&self) {
            self.content_box.unparent();
        }
    }

    impl WidgetImpl for ViewerPane {}
}

glib::wrapper! {
    pub struct ViewerPane(ObjectSubclass<imp::ViewerPane>)
        @extends gtk4::Widget,
                 @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl ViewerPane {
    pub fn new(state: Rc<RefCell<AppState>>) -> Self {
        let widget: Self = glib::Object::new();
        *widget.imp().state.borrow_mut() = Some(state);
        widget.build_ui();
        widget
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        self.imp().current_path.borrow().clone()
    }

    fn build_ui(&self) {
        let imp = self.imp();
        imp.content_box.set_parent(self);
        install_viewer_osd_css();
        self.build_tag_popover();

        let motion = gtk4::EventControllerMotion::new();
        let w = self.downgrade();
        motion.connect_motion(move |_, x, y| {
            if let Some(viewer) = w.upgrade() {
                viewer.imp().pointer_pos.set((x, y));
            }
        });
        imp.scrolled_window.add_controller(motion);

        let drag = gtk4::GestureDrag::new();
        drag.set_button(0);

        let w = self.downgrade();
        drag.connect_drag_begin(move |_, start_x, start_y| {
            if let Some(viewer) = w.upgrade() {
                let imp = viewer.imp();
                imp.pointer_pos.set((start_x, start_y));
                imp.drag_origin.set(Some((start_x, start_y)));
                imp.drag_adjustments.set((
                    imp.scrolled_window.hadjustment().value(),
                    imp.scrolled_window.vadjustment().value(),
                ));
            }
        });

        let w = self.downgrade();
        drag.connect_drag_update(move |_, offset_x, offset_y| {
            if let Some(viewer) = w.upgrade() {
                viewer.pan_drag(offset_x, offset_y);
            }
        });

        let w = self.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(viewer) = w.upgrade() {
                viewer.imp().drag_origin.set(None);
            }
        });
        imp.scrolled_window.add_controller(drag);

        // Double-click → toggle presentation mode
        let dblclick = gtk4::GestureClick::new();
        dblclick.set_button(1);
        let w = self.downgrade();
        dblclick.connect_released(move |_, n_press, _, _| {
            if n_press == 2 {
                if let Some(viewer) = w.upgrade() {
                    let _ = viewer.activate_action("win.toggle-presentation", None);
                }
            }
        });
        imp.scrolled_window.add_controller(dblclick);

        // -----------------------------------------------------------------------
        // Keyboard shortcuts
        // -----------------------------------------------------------------------
        let shortcuts = gtk4::ShortcutController::new();
        shortcuts.set_scope(gtk4::ShortcutScope::Managed);

        // Ctrl+0 — reset zoom
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Control>0").unwrap()),
            Some(gtk4::CallbackAction::new(move |widget, _| {
                let _ = widget.activate_action("win.zoom-mode", Some(&"fit".to_variant()));
                glib::Propagation::Stop
            })),
        ));

        // Alt+Return — toggle metadata overlay
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Alt>Return").unwrap()),
            Some(gtk4::NamedAction::new("win.show-metadata")),
        ));

        self.add_controller(shortcuts);

        // -----------------------------------------------------------------------
        // Scroll → zoom
        // -----------------------------------------------------------------------
        // Capture phase so we intercept scroll before the ScrolledWindow
        // consumes it (which it does whenever there is scrollable overflow).
        let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let w = self.downgrade();
        scroll.connect_scroll(move |ctrl, _dx, dy| {
            if let Some(viewer) = w.upgrade() {
                if let Some((x, y)) = ctrl.current_event().and_then(|event| event.position()) {
                    viewer.imp().pointer_pos.set((x, y));
                }
                let factor = if dy < 0.0 { 1.1 } else { 1.0 / 1.1 };
                viewer.apply_zoom(factor);
            }
            glib::Propagation::Stop
        });
        self.add_controller(scroll);

        let w = self.downgrade();
        self.connect_map(move |_| {
            if let Some(viewer) = w.upgrade() {
                viewer.ensure_fit_on_layout();
            }
        });

        let viewer_weak = self.downgrade();
        imp.tag_button.connect_clicked(move |_| {
            if let Some(viewer) = viewer_weak.upgrade() {
                viewer.open_tag_popover();
            }
        });

        let viewer_weak = self.downgrade();
        imp.tag_add_button.connect_clicked(move |_| {
            if let Some(viewer) = viewer_weak.upgrade() {
                viewer.open_tag_popover();
            }
        });
    }

    // -----------------------------------------------------------------------
    // Image loading (async via background thread + idle callback)
    // -----------------------------------------------------------------------

    /// Call once from window setup. Metadata results are drained on the GTK
    /// main thread; stale results are discarded by generation comparison.
    pub fn set_metadata_worker(
        &self,
        handle: crate::image_pipeline::worker::MetadataHandle,
        result_rx: async_channel::Receiver<crate::image_pipeline::worker::MetadataResult>,
    ) {
        *self.imp().metadata_handle.borrow_mut() = Some(handle);

        let widget_weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(result) = result_rx.recv().await {
                let Some(viewer) = widget_weak.upgrade() else {
                    break;
                };
                let imp = viewer.imp();
                if result.gen != imp.load_gen.get() {
                    continue;
                }
                imp.metadata_chip.update_metadata(&result.metadata);
                viewer.update_quality_indicator(&result.metadata);
                if imp.info_pending.replace(false) {
                    imp.metadata_chip.show_info();
                }
                viewer.refresh_tag_summary();
            }
        });
    }

    /// Opens the image Info panel, now or as soon as the current image's
    /// details have loaded (decision D18).
    pub fn show_info(&self) {
        let imp = self.imp();
        if !imp.metadata_chip.show_info() {
            imp.info_pending.set(imp.current_path.borrow().is_some());
        }
    }

    /// Clear the viewer (called when the folder changes).
    pub fn clear(&self) {
        let imp = self.imp();
        imp.load_gen.set(imp.load_gen.get().wrapping_add(1));
        imp.tag_popover.popdown();
        imp.picture.set_paintable(None::<&gdk4::Paintable>);
        imp.metadata_chip.clear();
        imp.metadata_chip.update_tags(&[]);
        imp.metadata_chip.update_collections(&[]);
        imp.metadata_chip.update_duplicate_count(0);
        imp.tag_osd.set_visible(false);
        self.set_quality_score(None);
        imp.spinner.stop();
        imp.spinner.set_visible(false);
        imp.error_label.set_visible(false);
        self.reset_zoom();
        *imp.current_path.borrow_mut() = None;
    }

    /// Load and display a full-resolution image from `path`.
    ///
    /// The image is loaded asynchronously using `glycin::Loader` on the GTK
    /// main thread. This leverages sandboxed decoding and supports high
    /// bit-depth formats like JPEG XL.
    pub fn load_image(&self, path: PathBuf) {
        if self.imp().current_path.borrow().as_ref() == Some(&path) {
            return;
        }
        crate::bench_event!(
            "viewer.load.request",
            serde_json::json!({
                "path": path.display().to_string(),
            }),
        );
        let imp = self.imp();
        let load_gen = imp.load_gen.get().wrapping_add(1);
        imp.load_gen.set(load_gen);
        imp.tag_chips_expanded.set(false);
        imp.tag_popover.popdown();
        *imp.current_path.borrow_mut() = Some(path.clone());
        imp.spinner.start();
        imp.spinner.set_visible(true);
        imp.picture.set_paintable(None::<&gdk4::Paintable>);
        imp.metadata_chip.clear();
        imp.metadata_chip.update_location(&path);
        imp.metadata_chip.update_tags(&[]);
        imp.metadata_chip.update_collections(&[]);
        imp.metadata_chip.update_duplicate_count(0);
        imp.tag_osd.set_visible(false);
        imp.error_label.set_visible(false);
        self.set_quality_score(None);
        self.refresh_tag_summary();

        let picture = imp.picture.clone();
        let spinner = imp.spinner.clone();
        let error_label = imp.error_label.clone();
        let path_clone = path.clone();
        let viewer = self.clone();

        glib::MainContext::default().spawn_local(async move {
            let file = gio::File::for_path(&path_clone);
            let result = async {
                let loader = glycin::Loader::new(file);
                let image = loader.load().await.map_err(|e| e.to_string())?;
                let frame = image.next_frame().await.map_err(|e| e.to_string())?;
                Ok::<_, String>(frame.texture())
            }
            .await;

            if viewer.imp().load_gen.get() != load_gen {
                return; // User navigated away
            }

            spinner.stop();
            spinner.set_visible(false);

            match result {
                Ok(texture) => {
                    picture.set_paintable(Some(texture.upcast_ref::<gdk4::Paintable>()));
                    viewer.reset_zoom();
                    viewer.ensure_fit_on_layout();
                }
                Err(e) => {
                    error_label.set_text(&format!("Could not load image: {}", e));
                    error_label.set_visible(true);
                }
            }
        });

        // Metadata loads via the shared worker; result drains in set_metadata_worker loop.
        if let Some(ref handle) = *imp.metadata_handle.borrow() {
            handle.request(path.clone(), load_gen);
        }
    }

    // -----------------------------------------------------------------------
    // Zoom
    // -----------------------------------------------------------------------------------

    fn apply_zoom(&self, factor: f64) {
        let imp = self.imp();
        let old_zoom = imp.zoom.get();
        let Some(paintable) = imp.picture.paintable() else {
            return;
        };
        let fit_scale = self.fit_scale_for_paintable(&paintable);
        let max_zoom = (1.0 / fit_scale.max(f64::EPSILON)).max(1.0) * 20.0;
        let new_zoom = (old_zoom * factor).clamp(1.0, max_zoom);
        if (new_zoom - old_zoom).abs() < f64::EPSILON {
            return;
        }

        let hadj = imp.scrolled_window.hadjustment();
        let vadj = imp.scrolled_window.vadjustment();
        let (focus_x, focus_y) = imp.pointer_pos.get();
        let content_focus_x = hadj.value() + focus_x;
        let content_focus_y = vadj.value() + focus_y;
        imp.zoom.set(new_zoom);

        let base_width = paintable.intrinsic_width().max(1);
        let base_height = paintable.intrinsic_height().max(1);
        let scaled_width = (base_width as f64 * fit_scale * new_zoom).round().max(1.0) as i32;
        let scaled_height = (base_height as f64 * fit_scale * new_zoom).round().max(1.0) as i32;

        imp.picture.set_size_request(scaled_width, scaled_height);

        let scale_ratio = new_zoom / old_zoom;
        self.set_adjustment_value(&hadj, content_focus_x * scale_ratio - focus_x);
        self.set_adjustment_value(&vadj, content_focus_y * scale_ratio - focus_y);
        let pct = (new_zoom * 100.0).round() as u32;
        self.show_zoom_osd(&format!("{pct}%"));
    }

    pub fn ensure_fit_on_layout(&self) {
        let viewer = self.clone();
        glib::idle_add_local_once(move || {
            let imp = viewer.imp();
            if imp.zoom_mode.get() == ZoomMode::Fit {
                viewer.update_picture_zoom();
            }
        });
    }

    pub fn reset_zoom(&self) {
        let imp = self.imp();
        imp.zoom.set(1.0);
        imp.zoom_mode.set(ZoomMode::Fit);
        self.update_picture_zoom();
        imp.scrolled_window.hadjustment().set_value(0.0);
        imp.scrolled_window.vadjustment().set_value(0.0);
        self.sync_zoom_button();
    }

    pub fn set_zoom_mode(&self, mode: ZoomMode) {
        let imp = self.imp();
        imp.zoom_mode.set(mode);
        match mode {
            ZoomMode::Fit => {
                imp.zoom.set(1.0);
                self.update_picture_zoom();
                imp.scrolled_window.hadjustment().set_value(0.0);
                imp.scrolled_window.vadjustment().set_value(0.0);
            }
            ZoomMode::OneToOne => {
                let Some(paintable) = imp.picture.paintable() else {
                    return;
                };
                let fit_scale = self.fit_scale_for_paintable(&paintable);
                imp.zoom.set((1.0 / fit_scale.max(f64::EPSILON)).max(1.0));
                self.update_picture_zoom();
                // Centre the scroll on the image after layout.
                let hadj = imp.scrolled_window.hadjustment();
                let vadj = imp.scrolled_window.vadjustment();
                glib::idle_add_local_once({
                    let hadj = hadj.clone();
                    let vadj = vadj.clone();
                    move || {
                        hadj.set_value((hadj.upper() - hadj.page_size()) / 2.0);
                        vadj.set_value((vadj.upper() - vadj.page_size()) / 2.0);
                    }
                });
            }
        }
        if mode == ZoomMode::OneToOne {
            let pct = (imp.zoom.get() * 100.0).round() as u32;
            self.show_zoom_osd(&format!("{pct}%"));
        }
        self.sync_zoom_button();
    }

    /// Store the zoom/fit toggle button so mode changes can update its icon.
    pub fn set_zoom_button(&self, btn: gtk4::Button) {
        *self.imp().zoom_btn.borrow_mut() = Some(btn);
    }

    /// Toggle between Fit and 1:1 pixel mode. Updates the stored button icon.
    pub fn toggle_zoom_mode(&self) {
        let imp = self.imp();
        let new_mode = match imp.zoom_mode.get() {
            ZoomMode::Fit => ZoomMode::OneToOne,
            ZoomMode::OneToOne => ZoomMode::Fit,
        };
        imp.zoom_mode.set(new_mode);
        match new_mode {
            ZoomMode::Fit => {
                imp.zoom.set(1.0);
                self.update_picture_zoom();
                imp.scrolled_window.hadjustment().set_value(0.0);
                imp.scrolled_window.vadjustment().set_value(0.0);
            }
            ZoomMode::OneToOne => {
                let Some(paintable) = imp.picture.paintable() else {
                    return;
                };
                let fit_scale = self.fit_scale_for_paintable(&paintable);
                imp.zoom.set((1.0 / fit_scale.max(f64::EPSILON)).max(1.0));
                self.update_picture_zoom();
                // Centre the scroll on the image after layout.
                let hadj = imp.scrolled_window.hadjustment();
                let vadj = imp.scrolled_window.vadjustment();
                glib::idle_add_local_once({
                    let hadj = hadj.clone();
                    let vadj = vadj.clone();
                    move || {
                        hadj.set_value((hadj.upper() - hadj.page_size()) / 2.0);
                        vadj.set_value((vadj.upper() - vadj.page_size()) / 2.0);
                    }
                });
            }
        }
        if new_mode == ZoomMode::OneToOne {
            let pct = (imp.zoom.get() * 100.0).round() as u32;
            self.show_zoom_osd(&format!("{pct}%"));
        }
        self.sync_zoom_button();
    }

    fn fit_scale_for_paintable(&self, paintable: &gdk4::Paintable) -> f64 {
        let base_width = paintable.intrinsic_width().max(1) as f64;
        let base_height = paintable.intrinsic_height().max(1) as f64;
        let imp = self.imp();
        let hadj = imp.scrolled_window.hadjustment();
        let vadj = imp.scrolled_window.vadjustment();
        let viewport_width = if hadj.page_size() > 0.0 {
            hadj.page_size()
        } else {
            imp.scrolled_window.width() as f64
        };
        let viewport_height = if vadj.page_size() > 0.0 {
            vadj.page_size()
        } else {
            imp.scrolled_window.height() as f64
        };

        if viewport_width <= 0.0 || viewport_height <= 0.0 {
            1.0
        } else {
            (viewport_width / base_width).min(viewport_height / base_height)
        }
    }

    fn update_picture_zoom(&self) {
        let imp = self.imp();
        let Some(paintable) = imp.picture.paintable() else {
            return;
        };
        let fit_scale = self.fit_scale_for_paintable(&paintable);
        let render_scale = (fit_scale * imp.zoom.get()).max(0.01);
        let width = (paintable.intrinsic_width().max(1) as f64 * render_scale)
            .round()
            .max(1.0) as i32;
        let height = (paintable.intrinsic_height().max(1) as f64 * render_scale)
            .round()
            .max(1.0) as i32;
        imp.picture.set_size_request(width, height);
    }

    fn show_zoom_osd(&self, text: &str) {
        let imp = self.imp();
        imp.zoom_label.set_text(text);
        imp.zoom_label.set_visible(true);
        if let Some(source) = imp.zoom_hide_source.borrow_mut().take() {
            source.remove();
        }
        let w = self.downgrade();
        let source =
            glib::timeout_add_local_once(std::time::Duration::from_millis(2000), move || {
                if let Some(viewer) = w.upgrade() {
                    let imp = viewer.imp();
                    imp.zoom_label.set_visible(false);
                    *imp.zoom_hide_source.borrow_mut() = None;
                }
            });
        *imp.zoom_hide_source.borrow_mut() = Some(source);
    }

    fn sync_zoom_button(&self) {
        let imp = self.imp();
        if let Some(ref btn) = *imp.zoom_btn.borrow() {
            match self.zoom_mode() {
                ZoomMode::Fit => {
                    btn.set_icon_name("zoom-fit-best-symbolic");
                    btn.set_tooltip_text(Some("1:1 Pixels (switch to actual size)"));
                }
                ZoomMode::OneToOne => {
                    btn.set_icon_name("zoom-original-symbolic");
                    btn.set_tooltip_text(Some("Fit to Window (switch to fit)"));
                }
            }
        }
    }

    pub fn metadata_visible(&self) -> bool {
        self.imp().metadata_visible.get()
    }

    pub fn set_metadata_visible(&self, visible: bool) {
        let imp = self.imp();
        imp.metadata_visible.set(visible);
        imp.metadata_chip.set_enabled(visible);
        if visible {
            self.refresh_tag_summary();
        } else {
            imp.tag_osd.set_visible(false);
        }
    }

    fn update_quality_indicator(&self, metadata: &crate::metadata::ImageMetadata) {
        self.set_quality_score(Some(&scorer::score_metadata(metadata)));
    }

    fn set_quality_score(&self, quality: Option<&QualityScore>) {
        let imp = self.imp();
        *imp.base_quality.borrow_mut() = quality.cloned();
        let Some(quality) = quality else {
            imp.metadata_chip.update_quality(None);
            return;
        };
        imp.metadata_chip.update_quality(Some(quality));
    }

    /// Refresh the quality display for the currently shown image after a
    /// sharpness result arrives. Resolution remains the displayed tier.
    pub fn apply_sharpness(&self, path: &std::path::Path, _sharpness_norm: f64) {
        let imp = self.imp();
        if imp.current_path.borrow().as_deref() != Some(path) {
            return;
        }
        let base = imp.base_quality.borrow().clone();
        let Some(base) = base else { return };
        imp.metadata_chip.update_quality(Some(&base));
    }

    pub fn zoom_mode(&self) -> ZoomMode {
        self.imp().zoom_mode.get()
    }

    // -----------------------------------------------------------------------
    // Metadata overlay
    // -----------------------------------------------------------------------

    pub fn toggle_metadata(&self) {
        self.set_metadata_visible(!self.metadata_visible());
    }

    pub fn open_tag_popover(&self) {
        let imp = self.imp();
        let path = match imp.current_path.borrow().clone() {
            Some(path) => path,
            None => return,
        };
        let db = imp
            .state
            .borrow()
            .as_ref()
            .and_then(|state| state.borrow().tags.clone());
        if db.is_none() {
            return;
        }

        {
            let mut tag_state = imp.tag_state.borrow_mut();
            tag_state.db = db;
            tag_state.path = Some(path);
        }

        imp.tag_entry.set_text("");
        imp.new_tag_entry.set_text("");
        imp.new_tag_entry.set_visible(false);
        imp.new_tag_create_btn.set_visible(false);
        imp.new_tag_cancel_btn.set_visible(false);
        imp.new_tag_btn.set_visible(true);
        self.refresh_tag_chips();
        self.refresh_tag_summary();
        imp.tag_popover.popup();
    }

    fn change_tag(
        &self,
        db: std::sync::Arc<crate::tags::TagDatabase>,
        path: PathBuf,
        tag: String,
        remove: bool,
    ) {
        let (tx, rx) = async_channel::bounded(1);
        let changed_path = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(db.change_tags_checked(&[path], &[tag], remove));
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(viewer) = weak.upgrade() else { return };
            match result {
                Ok(_) => {
                    if viewer.imp().current_path.borrow().as_ref() == Some(&changed_path) {
                        viewer.refresh_tag_chips();
                        viewer.refresh_tag_summary();
                    }
                    if let Some(window) = viewer
                        .root()
                        .and_then(|r| r.downcast::<crate::ui::window::SharprWindow>().ok())
                    {
                        window.tags_changed();
                    }
                }
                Err(error) => {
                    if let Some(window) = viewer
                        .root()
                        .and_then(|r| r.downcast::<crate::ui::window::SharprWindow>().ok())
                    {
                        window.add_toast(libadwaita::Toast::new(&format!(
                            "Could not save tag: {error}"
                        )));
                    }
                }
            }
        });
    }

    fn build_tag_popover(&self) {
        let imp = self.imp();

        let entry = imp.tag_entry.clone();
        imp.tag_popover.connect_show(move |_| {
            entry.grab_focus();
        });

        let key = gtk4::EventControllerKey::new();
        let popover = imp.tag_popover.clone();
        key.connect_key_pressed(move |_, key, _, _| {
            if key == gdk4::Key::Escape {
                popover.popdown();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        imp.tag_entry.add_controller(key);

        let viewer_weak = self.downgrade();
        imp.tag_entry.connect_search_changed(move |entry| {
            if let Some(viewer) = viewer_weak.upgrade() {
                viewer.filter_tag_lists(entry.text().as_str());
            }
        });

        let new_tag_entry = imp.new_tag_entry.clone();
        let new_tag_btn = imp.new_tag_btn.clone();
        let new_tag_create_btn = imp.new_tag_create_btn.clone();
        let new_tag_cancel_btn = imp.new_tag_cancel_btn.clone();

        imp.new_tag_btn.connect_clicked({
            let entry = new_tag_entry.clone();
            let btn = new_tag_btn.clone();
            let create_btn = new_tag_create_btn.clone();
            let cancel_btn = new_tag_cancel_btn.clone();
            move |_| {
                btn.set_visible(false);
                entry.set_visible(true);
                create_btn.set_visible(true);
                cancel_btn.set_visible(true);
                entry.grab_focus();
            }
        });

        let hide_new_tag_flow = {
            let entry = new_tag_entry.clone();
            let btn = new_tag_btn.clone();
            let create_btn = new_tag_create_btn.clone();
            let cancel_btn = new_tag_cancel_btn.clone();
            move || {
                entry.set_text("");
                entry.set_visible(false);
                create_btn.set_visible(false);
                cancel_btn.set_visible(false);
                btn.set_visible(true);
            }
        };

        imp.new_tag_cancel_btn.connect_clicked({
            let hide = hide_new_tag_flow.clone();
            move |_| hide()
        });

        let handle_create_tag = move |viewer: &ViewerPane, text: &str| {
            let tag = text.trim();
            if tag.is_empty() {
                return;
            }
            let state = viewer.imp().tag_state.borrow();
            if let (Some(db), Some(path)) = (state.db.clone(), state.path.clone()) {
                viewer.change_tag(db, path, tag.to_string(), false);
            }
            drop(state);
            viewer.refresh_tag_chips();
            viewer.refresh_tag_summary();
        };

        let viewer_weak_create = self.downgrade();
        let hide_clone1 = hide_new_tag_flow.clone();
        let entry_clone = imp.new_tag_entry.clone();
        imp.new_tag_create_btn.connect_clicked(move |_| {
            if let Some(viewer) = viewer_weak_create.upgrade() {
                handle_create_tag(&viewer, entry_clone.text().as_str());
                hide_clone1();
            }
        });

        let viewer_weak_activate = self.downgrade();
        let hide_clone2 = hide_new_tag_flow.clone();
        imp.new_tag_entry.connect_activate(move |entry| {
            if let Some(viewer) = viewer_weak_activate.upgrade() {
                handle_create_tag(&viewer, entry.text().as_str());
                hide_clone2();
            }
        });
    }

    fn filter_tag_lists(&self, query: &str) {
        let query = query.to_lowercase();
        let imp = self.imp();

        let q1 = query.clone();
        imp.selected_list.set_filter_func(move |row| {
            if q1.is_empty() {
                return true;
            }
            if let Some(box_container) = row.child().and_then(|w| w.downcast::<gtk4::Box>().ok()) {
                if let Some(label) = box_container
                    .last_child()
                    .and_then(|w| w.downcast::<gtk4::Label>().ok())
                {
                    return label.text().to_lowercase().contains(&q1);
                }
            }
            true
        });

        let show_all = !query.is_empty();
        if let Some(all_tags_box) = imp
            .all_tags_list
            .parent()
            .and_then(|w| w.downcast::<gtk4::Box>().ok())
        {
            all_tags_box.set_visible(show_all);
        }

        let q2 = query.clone();
        imp.all_tags_list.set_filter_func(move |row| {
            if let Some(box_container) = row.child().and_then(|w| w.downcast::<gtk4::Box>().ok()) {
                if let Some(label) = box_container
                    .last_child()
                    .and_then(|w| w.downcast::<gtk4::Label>().ok())
                {
                    return label.text().to_lowercase().contains(&q2);
                }
            }
            true
        });
    }

    fn refresh_tag_chips(&self) {
        let imp = self.imp();
        let tag_generation = imp.tag_refresh_gen.get().wrapping_add(1);
        imp.tag_refresh_gen.set(tag_generation);
        while let Some(child) = imp.selected_list.first_child() {
            imp.selected_list.remove(&child);
        }
        while let Some(child) = imp.all_tags_list.first_child() {
            imp.all_tags_list.remove(&child);
        }

        let state = imp.tag_state.borrow();
        let (Some(db), Some(path)) = (state.db.clone(), state.path.clone()) else {
            return;
        };
        drop(state);

        let generation = imp.load_gen.get();
        let (tx, rx) = async_channel::bounded(1);
        let worker_db = db.clone();
        let worker_path = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(worker_db.tag_choices_checked(&worker_path));
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(viewer) = weak.upgrade() else { return };
            let imp = viewer.imp();
            if imp.load_gen.get() != generation
                || imp.tag_refresh_gen.get() != tag_generation
                || imp.current_path.borrow().as_ref() != Some(&path)
            {
                return;
            }
            let (active_tags, all_tags) = match result {
                Ok(result) => (result.active, result.available),
                Err(error) => {
                    if let Some(window) = viewer
                        .root()
                        .and_then(|r| r.downcast::<crate::ui::window::SharprWindow>().ok())
                    {
                        window.add_toast(libadwaita::Toast::new(&format!(
                            "Could not read tags: {error}"
                        )));
                    }
                    return;
                }
            };
            while let Some(child) = imp.selected_list.first_child() {
                imp.selected_list.remove(&child);
            }
            while let Some(child) = imp.all_tags_list.first_child() {
                imp.all_tags_list.remove(&child);
            }

            let mut active_set = std::collections::HashSet::new();
            for tag in active_tags {
                active_set.insert(tag.clone());
                viewer.create_tag_row(&imp.selected_list, &tag, true, &db, &path);
            }

            if let Some(parent) = imp
                .selected_list
                .parent()
                .and_then(|w| w.downcast::<gtk4::Box>().ok())
            {
                parent.set_visible(!active_set.is_empty());
            }

            for tag in all_tags {
                if !active_set.contains(&tag.0) {
                    viewer.create_tag_row(&imp.all_tags_list, &tag.0, false, &db, &path);
                }
            }

            if let Some(parent) = imp
                .all_tags_list
                .parent()
                .and_then(|w| w.downcast::<gtk4::Box>().ok())
            {
                parent.set_visible(false);
            }
            viewer.filter_tag_lists(imp.tag_entry.text().as_str());
        });
    }

    fn create_tag_row(
        &self,
        list: &gtk4::ListBox,
        tag: &str,
        selected: bool,
        db: &std::sync::Arc<crate::tags::TagDatabase>,
        path: &std::path::Path,
    ) {
        let row = gtk4::ListBoxRow::new();
        row.set_selectable(false);
        row.add_css_class("tag-popover-row");

        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        hbox.set_margin_top(6);
        hbox.set_margin_bottom(6);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);

        let icon_name = if selected {
            "object-select-symbolic"
        } else {
            "radio-mixed-symbolic"
        };
        let icon = gtk4::Image::from_icon_name(icon_name);
        if selected {
            icon.add_css_class("accent");
        } else {
            icon.add_css_class("dim-label");
        }
        hbox.append(&icon);

        let label = gtk4::Label::new(Some(tag));
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        hbox.append(&label);

        row.set_child(Some(&hbox));

        let viewer_weak = self.downgrade();
        let tag_clone = tag.to_string();
        let db_clone = db.clone();
        let path_clone = path.to_path_buf();

        row.connect_activate(move |_| {
            if let Some(viewer) = viewer_weak.upgrade() {
                viewer.change_tag(
                    db_clone.clone(),
                    path_clone.clone(),
                    tag_clone.clone(),
                    selected,
                );
            }
        });

        list.append(&row);
    }

    fn refresh_tag_summary(&self) {
        let imp = self.imp();
        let path = match imp.current_path.borrow().clone() {
            Some(path) => path,
            None => {
                imp.metadata_chip.update_tags(&[]);
                imp.metadata_chip.update_collections(&[]);
                imp.metadata_chip.update_duplicate_count(0);
                imp.tag_osd.set_visible(false);
                return;
            }
        };

        let (db, index) = imp
            .state
            .borrow()
            .as_ref()
            .map(|state| {
                let st = state.borrow();
                (st.tags.clone(), st.library_index.clone())
            })
            .unwrap_or_default();
        let generation = imp.tag_summary_gen.get().wrapping_add(1);
        imp.tag_summary_gen.set(generation);
        imp.metadata_chip.update_tags(&[]);
        imp.metadata_chip.update_collections(&[]);
        imp.metadata_chip.update_duplicate_count(0);
        imp.tag_osd.set_visible(false);
        let worker_path = path.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                let tags = db
                    .map(|db| db.tags_for_path_checked(&worker_path))
                    .transpose()?
                    .unwrap_or_default();
                let collections = index
                    .as_ref()
                    .map(|index| index.list_collections())
                    .transpose()
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let tag_set: HashSet<&str> = tags.iter().map(String::as_str).collect();
                let names = collections
                    .iter()
                    .filter(|c| {
                        tag_set.contains(c.primary_tag.as_str())
                            || c.extra_tags.iter().any(|t| tag_set.contains(t.as_str()))
                    })
                    .map(|c| c.name.clone())
                    .collect::<Vec<_>>();
                let duplicates = index
                    .as_ref()
                    .map(|index| index.duplicate_groups())
                    .transpose()
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default()
                    .iter()
                    .find(|g| g.contains(&worker_path))
                    .map(|g| g.len().saturating_sub(1))
                    .unwrap_or(0);
                Ok((
                    tags,
                    collection_tag_color_map(&collections),
                    names,
                    duplicates,
                ))
            })();
            let _ = tx.send_blocking(result);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(viewer) = weak.upgrade() else { return };
            let imp = viewer.imp();
            if imp.tag_summary_gen.get() != generation
                || imp.current_path.borrow().as_ref() != Some(&path)
            {
                return;
            }
            match result {
                Ok((tags, colors, names, duplicates)) => {
                    imp.metadata_chip.update_tags(&tags);
                    imp.metadata_chip.update_collections(&names);
                    imp.metadata_chip.update_duplicate_count(duplicates);
                    viewer.render_tag_summary(&tags, &colors);
                }
                Err(error) => crate::bench_warn!(
                    "viewer.tags.summary.fail",
                    serde_json::json!({"error":error})
                ),
            }
        });
    }

    fn render_tag_summary(&self, tags: &[String], tag_to_color: &HashMap<String, String>) {
        let imp = self.imp();
        while let Some(child) = imp.tag_chips_box.first_child() {
            imp.tag_chips_box.remove(&child);
        }
        const MAX_CHIPS: usize = 4;
        let expanded = imp.tag_chips_expanded.get();
        let shown = if expanded {
            tags.len()
        } else {
            tags.len().min(MAX_CHIPS)
        };

        for tag in &tags[..shown] {
            let chip = gtk4::Button::new();
            chip.add_css_class("flat");
            chip.add_css_class("tag-osd-pill");
            chip.set_focus_on_click(false);
            chip.set_tooltip_text(Some(tag.as_str()));

            let chip_inner = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            let icon = gtk4::Image::from_icon_name("bookmark-new-symbolic");
            icon.set_pixel_size(12);
            let label = gtk4::Label::new(Some(tag.as_str()));
            label.set_max_width_chars(12);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            chip_inner.append(&icon);
            chip_inner.append(&label);
            chip.set_child(Some(&chip_inner));

            if let Some(color) = tag_to_color.get(tag.as_str()) {
                apply_tag_osd_chip_tint(&chip, color);
                apply_tag_icon_tint(&icon, color);
            }

            imp.tag_chips_box.append(&chip);

            let viewer_weak = self.downgrade();
            chip.connect_clicked(move |_| {
                if let Some(viewer) = viewer_weak.upgrade() {
                    viewer.open_tag_popover();
                }
            });
        }

        if !expanded && tags.len() > MAX_CHIPS {
            let overflow = gtk4::Button::new();
            overflow.add_css_class("flat");
            overflow.add_css_class("tag-osd-pill");
            overflow.set_focus_on_click(false);
            overflow.set_tooltip_text(Some("Show all tags"));
            let overflow_label = gtk4::Label::new(Some(&format!("+{}", tags.len() - MAX_CHIPS)));
            overflow.set_child(Some(&overflow_label));
            let viewer_weak = self.downgrade();
            overflow.connect_clicked(move |_| {
                if let Some(viewer) = viewer_weak.upgrade() {
                    viewer.imp().tag_chips_expanded.set(true);
                    viewer.refresh_tag_summary();
                }
            });
            imp.tag_chips_box.append(&overflow);
        }

        imp.tag_osd.set_visible(imp.metadata_visible.get());
    }

    fn pan_drag(&self, offset_x: f64, offset_y: f64) {
        let imp = self.imp();
        let (start_h, start_v) = imp.drag_adjustments.get();

        self.set_adjustment_value(&imp.scrolled_window.hadjustment(), start_h - offset_x);
        self.set_adjustment_value(&imp.scrolled_window.vadjustment(), start_v - offset_y);
    }

    fn set_adjustment_value(&self, adjustment: &gtk4::Adjustment, value: f64) {
        let max_value = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        adjustment.set_value(value.clamp(adjustment.lower(), max_value));
    }
}

fn install_viewer_osd_css() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_resource("/io/github/hebbihebb/Sharpr/viewer-osd.css");
        if let Some(display) = gdk4::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
}
