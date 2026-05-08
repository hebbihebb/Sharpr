use std::cell::RefCell;
use std::path::{Path, PathBuf};

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;

use crate::ui::compare_item::{CompareAssetInfo, CompareItem};
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
            .output_asset
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        imp.filename_label.set_text(&filename);

        if let Ok(date_str) = item.date_added.format("%B %e, %Y • %H:%M") {
            imp.date_label.set_text(&format!("Added {}", date_str));
        }

        imp.model_row.set_subtitle(&item.model);
        imp.scale_row.set_subtitle(&item.scale);

        update_asset_row(&imp.original_row, &item.original_asset);
        update_asset_row(&imp.output_row, &item.output_asset);
        update_sidecar_row(&imp.sidecar_row, item.preserved_png_asset.as_ref());
        imp.show_original_row
            .set_subtitle(&folder_display(&item.original_asset.path));
        imp.show_output_row
            .set_subtitle(&folder_display(&item.output_asset.path));

        imp.image_details_expander.set_expanded(false);
        imp.split_view.set_show_sidebar(false);
        imp.details_stack.set_visible_child_name("details");
    }

    pub fn clear(&self) {
        let imp = self.imp();
        *imp.current_item.borrow_mut() = None;
        imp.viewer.clear();
        imp.image_details_expander.set_expanded(false);
        imp.split_view.set_show_sidebar(false);
        imp.details_stack.set_visible_child_name("empty");
    }
}

fn update_asset_row(row: &libadwaita::ActionRow, asset: &CompareAssetInfo) {
    row.set_title(&asset.title);
    row.set_subtitle(&format_compare_asset_summary(asset));
}

fn update_sidecar_row(row: &libadwaita::ActionRow, asset: Option<&CompareAssetInfo>) {
    match asset {
        Some(asset) => update_asset_row(row, asset),
        None => {
            row.set_title("PNG Sidecar");
            row.set_subtitle("Not generated for this result");
        }
    }
}

fn format_compare_asset_summary(asset: &CompareAssetInfo) -> String {
    let filename = asset
        .path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| asset.path.display().to_string());
    format!(
        "{} · {} · {} × {} · {}",
        filename,
        asset.format,
        asset.dimensions.0,
        asset.dimensions.1,
        format_file_size(asset.file_size)
    )
}

fn format_file_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.2} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn folder_display(path: &Path) -> String {
    path.parent()
        .map(|parent| parent.display().to_string())
        .unwrap_or_else(|| "Unknown location".to_string())
}

fn reveal_in_file_manager(path: &Path) {
    let uri = gio::File::for_path(path).uri().to_string();
    let parent_uri = path
        .parent()
        .map(|parent| gio::File::for_path(parent).uri().to_string());
    glib::MainContext::default().spawn_local(async move {
        let shown = async {
            let conn = gio::bus_get_future(gio::BusType::Session).await.ok()?;
            let uris = glib::Variant::array_from_iter::<String>(std::iter::once(uri.to_variant()));
            let params = glib::Variant::tuple_from_iter([uris, "".to_variant()]);
            conn.call_future(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1",
                "ShowItems",
                Some(&params),
                None,
                gio::DBusCallFlags::NONE,
                5000,
            )
            .await
            .ok()
        }
        .await;
        if shown.is_none() {
            if let Some(uri) = parent_uri {
                let _ =
                    gio::AppInfo::launch_default_for_uri_future(&uri, gio::AppLaunchContext::NONE)
                        .await;
            }
        }
    });
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
        pub model_row: libadwaita::ActionRow,
        pub scale_row: libadwaita::ActionRow,
        pub image_details_expander: libadwaita::ExpanderRow,
        pub original_row: libadwaita::ActionRow,
        pub output_row: libadwaita::ActionRow,
        pub sidecar_row: libadwaita::ActionRow,
        pub show_original_row: libadwaita::ActionRow,
        pub show_output_row: libadwaita::ActionRow,

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
                model_row: libadwaita::ActionRow::new(),
                scale_row: libadwaita::ActionRow::new(),
                image_details_expander: libadwaita::ExpanderRow::new(),
                original_row: libadwaita::ActionRow::new(),
                output_row: libadwaita::ActionRow::new(),
                sidecar_row: libadwaita::ActionRow::new(),
                show_original_row: libadwaita::ActionRow::new(),
                show_output_row: libadwaita::ActionRow::new(),
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
            self.split_view.set_min_sidebar_width(320.0);
            self.split_view.set_max_sidebar_width(360.0);
            self.split_view.set_show_sidebar(false);

            let center_overlay = gtk4::Overlay::new();
            center_overlay.set_child(Some(&self.viewer));

            let original_lbl = gtk4::Label::new(Some("Original"));
            original_lbl.add_css_class("compare-corner-label");
            original_lbl.set_halign(gtk4::Align::Start);
            original_lbl.set_valign(gtk4::Align::Start);
            original_lbl.set_margin_top(16);
            original_lbl.set_margin_start(16);
            center_overlay.add_overlay(&original_lbl);

            let upscaled_lbl = gtk4::Label::new(Some("Upscaled"));
            upscaled_lbl.add_css_class("compare-corner-label");
            upscaled_lbl.set_halign(gtk4::Align::End);
            upscaled_lbl.set_valign(gtk4::Align::Start);
            upscaled_lbl.set_margin_top(16);
            upscaled_lbl.set_margin_end(16);
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

            let inspector_toggle = gtk4::Button::from_icon_name("info-outline-symbolic");
            inspector_toggle.add_css_class("osd");
            inspector_toggle.set_halign(gtk4::Align::End);
            inspector_toggle.set_valign(gtk4::Align::End);
            inspector_toggle.set_margin_bottom(20);
            inspector_toggle.set_margin_end(20);
            inspector_toggle.set_tooltip_text(Some("Toggle image details"));
            center_overlay.add_overlay(&inspector_toggle);

            self.split_view.set_content(Some(&center_overlay));

            let details_box_outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            details_box_outer.add_css_class("background");

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

            self.filename_label.add_css_class("title-4");
            self.filename_label.set_halign(gtk4::Align::Start);
            self.filename_label
                .set_ellipsize(gtk4::pango::EllipsizeMode::Middle);

            self.date_label.add_css_class("dim-label");
            self.date_label.add_css_class("caption");
            self.date_label.set_halign(gtk4::Align::Start);

            let details_scrolled = gtk4::ScrolledWindow::new();
            details_scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
            details_scrolled.set_vexpand(true);

            let details_container = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
            details_container.set_margin_start(16);
            details_container.set_margin_end(16);
            details_container.set_margin_top(16);
            details_container.set_margin_bottom(16);
            details_container.append(&self.filename_label);
            details_container.append(&self.date_label);

            let details_page = libadwaita::PreferencesPage::new();
            let summary_group = libadwaita::PreferencesGroup::new();

            self.model_row.set_title("Model");
            summary_group.add(&self.model_row);

            self.scale_row.set_title("Scale");
            summary_group.add(&self.scale_row);

            details_page.add(&summary_group);

            let details_group = libadwaita::PreferencesGroup::new();
            self.image_details_expander.set_title("Image Details");
            self.image_details_expander
                .set_subtitle("Original, output, and preserved PNG sidecar");
            self.image_details_expander.set_expanded(false);

            self.original_row.set_activatable(false);
            self.output_row.set_activatable(false);
            self.sidecar_row.set_activatable(false);
            self.image_details_expander.add_row(&self.original_row);
            self.image_details_expander.add_row(&self.output_row);
            self.image_details_expander.add_row(&self.sidecar_row);
            details_group.add(&self.image_details_expander);
            details_page.add(&details_group);

            let actions_group = libadwaita::PreferencesGroup::new();
            actions_group.set_title("Actions");

            self.show_original_row.set_title("Show Original in Files");
            let show_original_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
            show_original_btn.add_css_class("flat");
            self.show_original_row.add_suffix(&show_original_btn);
            actions_group.add(&self.show_original_row);

            self.show_output_row.set_title("Show Output in Files");
            let show_output_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
            show_output_btn.add_css_class("flat");
            self.show_output_row.add_suffix(&show_output_btn);
            actions_group.add(&self.show_output_row);

            let tasks_row = libadwaita::ActionRow::new();
            tasks_row.set_title("Back to Tasks");
            tasks_row.set_subtitle("Return to the task queue");
            let tasks_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
            tasks_btn.add_css_class("flat");
            tasks_row.add_suffix(&tasks_btn);
            actions_group.add(&tasks_row);

            let remove_row = libadwaita::ActionRow::new();
            remove_row.set_title("Remove from Compare");
            remove_row.set_subtitle("Keep the files, remove this item from the compare queue");
            let remove_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
            remove_btn.add_css_class("flat");
            remove_btn.add_css_class("destructive-action");
            remove_row.add_suffix(&remove_btn);
            actions_group.add(&remove_row);

            details_page.add(&actions_group);
            details_container.append(&details_page);
            details_scrolled.set_child(Some(&details_container));
            self.details_stack
                .add_named(&details_scrolled, Some("details"));

            details_box_outer.append(&self.details_stack);
            self.split_view.set_sidebar(Some(&details_box_outer));

            let split_view = self.split_view.clone();
            inspector_toggle.connect_clicked(move |_| {
                let visible = split_view.shows_sidebar();
                split_view.set_show_sidebar(!visible);
            });

            let viewer = self.viewer.clone();
            fit_btn.connect_clicked(move |_| {
                viewer.set_zoom_mode(crate::ui::viewer::ZoomMode::Fit);
            });

            let viewer = self.viewer.clone();
            zoom_1_1_btn.connect_clicked(move |_| {
                viewer.set_zoom_mode(crate::ui::viewer::ZoomMode::OneToOne);
            });

            let viewer = self.viewer.clone();
            zoom_in_btn.connect_clicked(move |_| {
                viewer.apply_scroll_zoom(1.1);
            });

            let viewer = self.viewer.clone();
            zoom_out_btn.connect_clicked(move |_| {
                viewer.apply_scroll_zoom(1.0 / 1.1);
            });

            let obj_weak = obj.downgrade();
            show_original_btn.connect_clicked(move |_| {
                let Some(obj) = obj_weak.upgrade() else {
                    return;
                };
                let current_item = obj.imp().current_item.borrow();
                let Some(item) = current_item.as_ref() else {
                    return;
                };
                reveal_in_file_manager(&item.original_asset.path);
            });

            let obj_weak = obj.downgrade();
            show_output_btn.connect_clicked(move |_| {
                let Some(obj) = obj_weak.upgrade() else {
                    return;
                };
                let current_item = obj.imp().current_item.borrow();
                let Some(item) = current_item.as_ref() else {
                    return;
                };
                reveal_in_file_manager(&item.output_asset.path);
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
