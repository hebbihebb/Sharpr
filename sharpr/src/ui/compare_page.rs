use std::cell::RefCell;
use std::path::PathBuf;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::ui::window::CompareItem;
use crate::upscale::BeforeAfterViewer;

type CompareRemoveCallback = Box<dyn Fn(PathBuf)>;

glib::wrapper! {
    pub struct ComparePage(ObjectSubclass<imp::ComparePage>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl ComparePage {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_compare_remove_cb<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.imp().compare_remove_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_comparison(&self, item: CompareItem) {
        let imp = self.imp();
        *imp.current_item.borrow_mut() = Some(item.clone());

        imp.viewer
            .load(item.source_path.clone(), item.output_path.clone(), || {});

        let filename = item
            .output_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        imp.filename_label.set_text(&filename);

        if let Ok(date_str) = item.date_added.format("%B %e, %Y • %H:%M") {
            imp.date_label.set_text(&format!("Added {}", date_str));
        }

        imp.model_value.set_text(&item.model);
        imp.scale_value.set_text(&item.scale);
        imp.format_value.set_text(&item.format);
        imp.dims_value
            .set_text(&format!("{} × {}", item.dimensions.0, item.dimensions.1));
        imp.size_value
            .set_text(&format!("{:.2} MB", item.file_size as f64 / 1_048_576.0));

        imp.details_stack.set_visible_child_name("details");
    }

    pub fn clear(&self) {
        let imp = self.imp();
        *imp.current_item.borrow_mut() = None;
        imp.viewer.clear();
        imp.details_stack.set_visible_child_name("empty");
    }
}

mod imp {
    use super::*;

    pub struct ComparePage {
        pub split_view: libadwaita::OverlaySplitView,
        pub details_stack: gtk4::Stack,
        pub viewer: BeforeAfterViewer,
        pub toolbar_revealer: gtk4::Revealer,

        pub filename_label: gtk4::Label,
        pub date_label: gtk4::Label,
        pub model_value: gtk4::Label,
        pub scale_value: gtk4::Label,
        pub format_value: gtk4::Label,
        pub dims_value: gtk4::Label,
        pub size_value: gtk4::Label,

        pub current_item: RefCell<Option<CompareItem>>,
        pub compare_remove_cb: RefCell<Option<CompareRemoveCallback>>,
    }

    impl Default for ComparePage {
        fn default() -> Self {
            Self {
                split_view: libadwaita::OverlaySplitView::new(),
                details_stack: gtk4::Stack::new(),
                viewer: BeforeAfterViewer::new(),
                toolbar_revealer: gtk4::Revealer::new(),
                filename_label: gtk4::Label::new(None),
                date_label: gtk4::Label::new(None),
                model_value: gtk4::Label::new(None),
                scale_value: gtk4::Label::new(None),
                format_value: gtk4::Label::new(None),
                dims_value: gtk4::Label::new(None),
                size_value: gtk4::Label::new(None),
                current_item: RefCell::new(None),
                compare_remove_cb: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ComparePage {
        const NAME: &'static str = "ComparePage";
        type Type = super::ComparePage;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for ComparePage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.split_view.set_parent(&*obj);
            self.split_view.set_sidebar_position(gtk4::PackType::End);
            self.split_view.set_min_sidebar_width(300.0);
            self.split_view.set_max_sidebar_width(340.0);

            let center_overlay = gtk4::Overlay::new();
            center_overlay.set_child(Some(&self.viewer));

            let original_lbl = gtk4::Label::new(Some("Original"));
            original_lbl.add_css_class("osd");
            original_lbl.set_halign(gtk4::Align::Start);
            original_lbl.set_valign(gtk4::Align::Start);
            original_lbl.set_margin_top(12);
            original_lbl.set_margin_start(12);
            center_overlay.add_overlay(&original_lbl);

            let upscaled_lbl = gtk4::Label::new(Some("Upscaled"));
            upscaled_lbl.add_css_class("osd");
            upscaled_lbl.set_halign(gtk4::Align::End);
            upscaled_lbl.set_valign(gtk4::Align::Start);
            upscaled_lbl.set_margin_top(12);
            upscaled_lbl.set_margin_end(12);
            center_overlay.add_overlay(&upscaled_lbl);

            self.toolbar_revealer
                .set_transition_type(gtk4::RevealerTransitionType::SlideUp);
            self.toolbar_revealer.set_reveal_child(true);
            self.toolbar_revealer.set_halign(gtk4::Align::Center);
            self.toolbar_revealer.set_valign(gtk4::Align::End);
            self.toolbar_revealer.set_margin_bottom(20);

            let toolbar_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            toolbar_box.add_css_class("osd");
            toolbar_box.add_css_class("toolbar");

            let fit_btn = gtk4::Button::from_icon_name("zoom-fit-best-symbolic");
            fit_btn.set_tooltip_text(Some("Fit to window"));
            let zoom_1_1_btn = gtk4::Button::with_label("1:1");
            zoom_1_1_btn.set_tooltip_text(Some("Zoom 100%"));
            let zoom_out_btn = gtk4::Button::from_icon_name("zoom-out-symbolic");
            let zoom_in_btn = gtk4::Button::from_icon_name("zoom-in-symbolic");

            toolbar_box.append(&fit_btn);
            toolbar_box.append(&zoom_1_1_btn);
            toolbar_box.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
            toolbar_box.append(&zoom_out_btn);
            toolbar_box.append(&zoom_in_btn);

            self.toolbar_revealer.set_child(Some(&toolbar_box));
            center_overlay.add_overlay(&self.toolbar_revealer);

            let inspector_toggle = gtk4::Button::from_icon_name("info-symbolic");
            inspector_toggle.add_css_class("osd");
            inspector_toggle.set_halign(gtk4::Align::End);
            inspector_toggle.set_valign(gtk4::Align::End);
            inspector_toggle.set_margin_bottom(20);
            inspector_toggle.set_margin_end(20);
            inspector_toggle.set_tooltip_text(Some("Toggle details"));
            center_overlay.add_overlay(&inspector_toggle);

            self.split_view.set_content(Some(&center_overlay));

            let details_box_outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            details_box_outer.add_css_class("background");

            let sidebar_header = libadwaita::HeaderBar::new();
            sidebar_header.set_show_end_title_buttons(false);
            let sidebar_close = gtk4::Button::from_icon_name("window-close-symbolic");
            sidebar_close.add_css_class("flat");
            sidebar_header.pack_end(&sidebar_close);
            details_box_outer.append(&sidebar_header);

            self.details_stack
                .set_transition_type(gtk4::StackTransitionType::Crossfade);

            let empty_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
            empty_box.set_margin_top(24);
            empty_box.set_margin_bottom(24);
            empty_box.set_margin_start(16);
            empty_box.set_margin_end(16);
            empty_box.set_valign(gtk4::Align::Center);
            let empty_lbl = gtk4::Label::new(Some("Select an item from the compare filmstrip"));
            empty_lbl.add_css_class("dim-label");
            empty_lbl.set_wrap(true);
            empty_lbl.set_justify(gtk4::Justification::Center);
            empty_box.append(&empty_lbl);
            self.details_stack.add_named(&empty_box, Some("empty"));

            let details_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
            details_box.set_margin_start(16);
            details_box.set_margin_end(16);
            details_box.set_margin_top(16);
            details_box.set_margin_bottom(16);

            self.filename_label.add_css_class("title-4");
            self.filename_label.set_halign(gtk4::Align::Start);
            self.filename_label
                .set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            details_box.append(&self.filename_label);

            self.date_label.add_css_class("dim-label");
            self.date_label.add_css_class("caption");
            self.date_label.set_halign(gtk4::Align::Start);
            details_box.append(&self.date_label);

            let details_heading = gtk4::Label::new(Some("Output Details"));
            details_heading.add_css_class("heading");
            details_heading.set_halign(gtk4::Align::Start);
            details_heading.set_margin_top(12);
            details_box.append(&details_heading);

            let grid = gtk4::Grid::new();
            grid.set_row_spacing(8);
            grid.set_column_spacing(12);
            grid.set_margin_top(8);

            let add_row =
                |grid: &gtk4::Grid, label: &str, value_lbl: &gtk4::Label, row: &mut i32| {
                    let l = gtk4::Label::new(Some(label));
                    l.set_halign(gtk4::Align::Start);
                    l.add_css_class("dim-label");
                    grid.attach(&l, 0, *row, 1, 1);

                    value_lbl.set_halign(gtk4::Align::End);
                    value_lbl.set_hexpand(true);
                    grid.attach(value_lbl, 1, *row, 1, 1);
                    *row += 1;
                };

            let mut row = 0;
            add_row(&grid, "Model", &self.model_value, &mut row);
            add_row(&grid, "Scale", &self.scale_value, &mut row);
            add_row(&grid, "Format", &self.format_value, &mut row);
            add_row(&grid, "Dimensions", &self.dims_value, &mut row);
            add_row(&grid, "File Size", &self.size_value, &mut row);
            details_box.append(&grid);

            let actions_heading = gtk4::Label::new(Some("Actions"));
            actions_heading.add_css_class("heading");
            actions_heading.set_halign(gtk4::Align::Start);
            actions_heading.set_margin_top(20);
            details_box.append(&actions_heading);

            let open_btn = gtk4::Button::with_label("Open Output");
            open_btn.set_halign(gtk4::Align::Fill);
            details_box.append(&open_btn);

            let tasks_btn = gtk4::Button::with_label("Back to Tasks");
            tasks_btn.set_halign(gtk4::Align::Fill);
            details_box.append(&tasks_btn);

            let spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            spacer.set_vexpand(true);
            details_box.append(&spacer);

            let remove_btn = gtk4::Button::with_label("Remove from Queue");
            remove_btn.add_css_class("destructive-action");
            remove_btn.add_css_class("flat");
            remove_btn.set_halign(gtk4::Align::Start);
            details_box.append(&remove_btn);

            self.details_stack.add_named(&details_box, Some("details"));
            details_box_outer.append(&self.details_stack);
            self.split_view.set_sidebar(Some(&details_box_outer));

            let split_view_c = self.split_view.clone();
            sidebar_close.connect_clicked(move |_| {
                split_view_c.set_show_sidebar(false);
            });

            let split_view_c = self.split_view.clone();
            inspector_toggle.connect_clicked(move |_| {
                let current = split_view_c.shows_sidebar();
                split_view_c.set_show_sidebar(!current);
            });

            let viewer_c = self.viewer.clone();
            fit_btn.connect_clicked(move |_| {
                viewer_c.set_zoom_mode(crate::ui::viewer::ZoomMode::Fit);
            });
            let viewer_c = self.viewer.clone();
            zoom_1_1_btn.connect_clicked(move |_| {
                viewer_c.set_zoom_mode(crate::ui::viewer::ZoomMode::OneToOne);
            });
            let viewer_c = self.viewer.clone();
            zoom_in_btn.connect_clicked(move |_| {
                viewer_c.apply_scroll_zoom(1.1);
            });
            let viewer_c = self.viewer.clone();
            zoom_out_btn.connect_clicked(move |_| {
                viewer_c.apply_scroll_zoom(1.0 / 1.1);
            });

            let obj_weak = obj.downgrade();
            open_btn.connect_clicked(move |_| {
                let Some(obj) = obj_weak.upgrade() else {
                    return;
                };
                let current_item = obj.imp().current_item.borrow();
                let Some(item) = current_item.as_ref() else {
                    return;
                };
                let uri = gio::File::for_path(&item.output_path).uri();
                let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
            });

            let obj_weak = obj.downgrade();
            tasks_btn.connect_clicked(move |_| {
                let Some(obj) = obj_weak.upgrade() else {
                    return;
                };
                if let Some(window) = obj.root().and_downcast::<crate::ui::window::SharprWindow>() {
                    if let Some(stack) = window.imp().content_stack.borrow().as_ref() {
                        stack.set_visible_child_name("tasks");
                    }
                }
            });

            let obj_weak = obj.downgrade();
            remove_btn.connect_clicked(move |_| {
                let Some(obj) = obj_weak.upgrade() else {
                    return;
                };
                let Some(item) = obj.imp().current_item.borrow().clone() else {
                    return;
                };
                let cb_borrow = obj.imp().compare_remove_cb.borrow();
                if let Some(cb) = cb_borrow.as_ref() {
                    cb(item.output_path);
                }
            });

            self.details_stack.set_visible_child_name("empty");
        }

        fn dispose(&self) {
            self.split_view.unparent();
        }
    }

    impl WidgetImpl for ComparePage {}
}
