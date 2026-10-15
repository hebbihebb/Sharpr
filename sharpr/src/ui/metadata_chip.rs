use std::path::Path;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::metadata::ImageMetadata;
use crate::quality::QualityScore;

// ---------------------------------------------------------------------------
// MetadataChip — the quiet bottom-right OSD chip (decision D18): resolution,
// format, a below-4K hint and the image's buckets. Everything else lives in
// the expanded Info card, opened by clicking the chip, Ctrl+I or right-click → Info.
// ---------------------------------------------------------------------------

/// Long × short edge of 4K, the user's wallpaper target.
const FOUR_K: (u32, u32) = (3840, 2160);

mod imp {
    use super::*;

    pub struct MetadataChip {
        pub card: gtk4::Box,
        pub metadata_label: gtk4::Label,
        pub below_4k_label: gtk4::Label,
        pub buckets_label: gtk4::Label,
        pub details: gtk4::Box,
        pub pinned: std::cell::Cell<bool>,
        pub hover_token: std::cell::Cell<u64>,
        pub info_title: gtk4::Label,
        pub info_location: gtk4::Label,
        pub info_file: gtk4::Label,
        pub info_camera: gtk4::Label,
        pub quality_row: gtk4::Box,
        pub quality_score_label: gtk4::Label,
        pub quality_class_label: gtk4::Label,
        pub quality_segments: [gtk4::Box; 5],
        pub popover_quality_label: gtk4::Label,
        pub popover_collections_label: gtk4::Label,
        pub popover_duplicates_label: gtk4::Label,
        pub duplicate_count: std::cell::Cell<usize>,
        pub enabled: std::cell::Cell<bool>,
        pub has_metadata: std::cell::Cell<bool>,
    }

    fn info_label(css: Option<&str>) -> gtk4::Label {
        let label = gtk4::Label::new(None);
        label.set_halign(gtk4::Align::Start);
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_max_width_chars(44);
        if let Some(css) = css {
            label.add_css_class(css);
        }
        label
    }

    impl Default for MetadataChip {
        fn default() -> Self {
            let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            card.add_css_class("osd");
            card.add_css_class("metadata-osd");
            card.set_valign(gtk4::Align::End);
            card.set_margin_top(12);
            card.set_margin_bottom(12);
            card.set_margin_start(12);
            card.set_margin_end(12);
            card.set_visible(false);
            card.set_tooltip_text(Some("Image info (Ctrl+I)"));

            let metadata_label = gtk4::Label::new(None);
            metadata_label.set_halign(gtk4::Align::Start);
            metadata_label.set_xalign(0.0);
            metadata_label.add_css_class("metadata-osd-meta");
            metadata_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            metadata_label.set_max_width_chars(36);

            let below_4k_label = gtk4::Label::new(Some("Below 4K"));
            below_4k_label.add_css_class("metadata-osd-below");
            below_4k_label.set_visible(false);

            let first_line = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            first_line.append(&metadata_label);
            first_line.append(&below_4k_label);

            let buckets_label = gtk4::Label::new(None);
            buckets_label.set_halign(gtk4::Align::Start);
            buckets_label.set_xalign(0.0);
            buckets_label.add_css_class("caption");
            buckets_label.add_css_class("metadata-osd-buckets");
            buckets_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            buckets_label.set_max_width_chars(36);
            buckets_label.set_visible(false);

            card.append(&first_line);
            card.append(&buckets_label);

            // Details expand within this same card.
            let quality_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            quality_row.set_halign(gtk4::Align::Start);
            let quality_score_label = gtk4::Label::new(Some("Res --"));
            quality_score_label.add_css_class("metadata-osd-quality-score");
            let separator = gtk4::Label::new(Some("·"));
            separator.add_css_class("dim-label");
            let quality_class_label = gtk4::Label::new(None);
            quality_class_label.add_css_class("metadata-osd-quality-class");
            let segment_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
            segment_row.set_valign(gtk4::Align::Center);
            let quality_segments = std::array::from_fn(|_| {
                let segment = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
                segment.add_css_class("metadata-osd-segment");
                segment
            });
            for segment in &quality_segments {
                segment_row.append(segment);
            }
            quality_row.append(&quality_score_label);
            quality_row.append(&separator);
            quality_row.append(&quality_class_label);
            quality_row.append(&segment_row);
            quality_row.set_visible(false);

            let info_title = info_label(Some("heading"));
            let info_location = info_label(Some("dim-label"));
            info_location.add_css_class("caption");
            info_location.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            let info_file = info_label(None);
            let info_camera = info_label(Some("dim-label"));
            info_camera.set_visible(false);
            let popover_quality_label = info_label(Some("dim-label"));
            popover_quality_label.set_text("Quality: Not available");
            let popover_collections_label = info_label(None);
            popover_collections_label.set_text("In no buckets");
            let popover_duplicates_label = info_label(None);
            popover_duplicates_label.set_visible(false);

            let details = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            details.set_margin_top(12);
            details.set_margin_bottom(12);
            details.set_margin_start(12);
            details.set_margin_end(12);
            for widget in [
                info_title.upcast_ref::<gtk4::Widget>(),
                info_location.upcast_ref(),
                info_file.upcast_ref(),
                info_camera.upcast_ref(),
                quality_row.upcast_ref(),
                popover_collections_label.upcast_ref(),
                popover_duplicates_label.upcast_ref(),
            ] {
                details.append(widget);
            }

            details.set_margin_start(0);
            details.set_margin_end(0);
            details.set_margin_bottom(0);
            details.set_visible(false);
            details.add_css_class("metadata-details");
            card.append(&details);

            Self {
                card,
                metadata_label,
                below_4k_label,
                buckets_label,
                details,
                pinned: std::cell::Cell::new(false),
                hover_token: std::cell::Cell::new(0),
                info_title,
                info_location,
                info_file,
                info_camera,
                quality_row,
                quality_score_label,
                quality_class_label,
                quality_segments,
                popover_quality_label,
                popover_collections_label,
                popover_duplicates_label,
                duplicate_count: std::cell::Cell::new(0),
                enabled: std::cell::Cell::new(true),
                has_metadata: std::cell::Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MetadataChip {
        const NAME: &'static str = "SharprMetadataChip";
        type Type = super::MetadataChip;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for MetadataChip {
        fn dispose(&self) {
            self.card.unparent();
        }
    }

    impl WidgetImpl for MetadataChip {}
}

glib::wrapper! {
    pub struct MetadataChip(ObjectSubclass<imp::MetadataChip>)
        @extends gtk4::Widget,
                 @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

/// True when an image of `(w, h)` reaches 4K in either orientation.
fn reaches_4k((w, h): (u32, u32)) -> bool {
    w.max(h) >= FOUR_K.0 && w.min(h) >= FOUR_K.1
}

impl MetadataChip {
    pub fn new() -> Self {
        let widget: Self = glib::Object::new();
        widget.imp().card.set_parent(&widget);
        let weak = widget.downgrade();
        let motion = gtk4::EventControllerMotion::new();
        motion.connect_enter(move |_, _, _| {
            if let Some(chip) = weak.upgrade() {
                chip.set_hovered(true);
            }
        });
        let weak = widget.downgrade();
        motion.connect_leave(move |_| {
            if let Some(chip) = weak.upgrade() {
                chip.set_hovered(false);
            }
        });
        widget.add_controller(motion);
        let weak = widget.downgrade();
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(chip) = weak.upgrade() {
                if chip.imp().pinned.get() {
                    chip.hide_info();
                } else {
                    chip.show_info();
                }
            }
        });
        widget.add_controller(click);
        widget
    }

    pub fn update_metadata(&self, meta: &ImageMetadata) {
        let imp = self.imp();
        let mut chip = Vec::new();
        if let Some(dimensions) = meta.dimensions_display() {
            chip.push(dimensions.replace(" × ", "×"));
        }
        if !meta.format.is_empty() {
            chip.push(meta.format.clone());
        }
        let chip_text = chip.join(" · ");
        imp.metadata_label.set_text(&chip_text);
        let known = meta.width > 0 && meta.height > 0;
        imp.below_4k_label
            .set_visible(known && !reaches_4k((meta.width, meta.height)));

        imp.info_title.set_text(&meta.filename);
        let mut file = chip.clone();
        let megapixels = meta.width as f64 * meta.height as f64 / 1_000_000.0;
        if megapixels >= 0.1 {
            if let Some(mp) = meta.megapixels_display() {
                file.insert(1.min(file.len()), mp);
            }
        }
        let size = meta.file_size_display();
        if !size.is_empty() {
            file.push(size);
        }
        imp.info_file.set_text(&file.join(" · "));
        let camera: Vec<&str> = [
            &meta.camera,
            &meta.lens,
            &meta.focal_length,
            &meta.aperture,
            &meta.shutter_speed,
            &meta.iso,
            &meta.color_space,
        ]
        .into_iter()
        .filter_map(|value| value.as_deref())
        .collect();
        imp.info_camera.set_text(&camera.join(" · "));
        imp.info_camera.set_visible(!camera.is_empty());

        let has_metadata = !chip_text.is_empty();
        imp.has_metadata.set(has_metadata);
        imp.metadata_label.set_visible(has_metadata);
        self.sync_visibility();
    }

    /// The folder shown in the expanded Info card.
    pub fn update_location(&self, path: &Path) {
        let folder = path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.imp().info_location.set_text(&folder);
    }

    /// Opens the expanded Info card (Ctrl+I, right-click → Info, or a click).
    /// Returns false while no image info is loaded yet.
    pub fn show_info(&self) -> bool {
        let imp = self.imp();
        if !imp.card.is_visible() {
            return false;
        }
        imp.pinned.set(true);
        imp.details.set_visible(true);
        true
    }

    pub fn is_info_open(&self) -> bool {
        self.imp().details.is_visible()
    }

    pub fn hide_info(&self) -> bool {
        let imp = self.imp();
        let was_open = self.is_info_open();
        imp.pinned.set(false);
        imp.hover_token.set(imp.hover_token.get().wrapping_add(1));
        imp.details.set_visible(false);
        was_open
    }

    pub fn set_hovered(&self, hovered: bool) {
        let imp = self.imp();
        let token = imp.hover_token.get().wrapping_add(1);
        imp.hover_token.set(token);
        let weak = self.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
            let Some(chip) = weak.upgrade() else { return };
            let imp = chip.imp();
            if imp.hover_token.get() == token && !imp.pinned.get() && imp.card.is_visible() {
                imp.details.set_visible(hovered);
            }
        });
    }

    pub fn set_enabled(&self, enabled: bool) {
        if !enabled {
            self.hide_info();
        }
        self.imp().enabled.set(enabled);
        self.sync_visibility();
    }

    pub fn update_quality(&self, quality: Option<&QualityScore>) {
        let imp = self.imp();
        for class_name in ["success", "warning", "error"] {
            imp.quality_class_label.remove_css_class(class_name);
        }
        let Some(quality) = quality else {
            imp.quality_score_label.set_text("Res --");
            imp.quality_class_label.set_text("");
            imp.popover_quality_label.set_text("Quality: Not available");
            imp.quality_row.set_visible(false);
            for segment in &imp.quality_segments {
                segment.remove_css_class("active");
            }
            return;
        };
        imp.quality_score_label.set_text("Res");
        imp.quality_class_label.set_text(quality.class.label());
        imp.popover_quality_label.set_text(&format!(
            "Quality: {} · {} / 100",
            quality.class.label(),
            quality.score
        ));
        imp.quality_class_label
            .set_tooltip_text(Some(&quality.tooltip()));
        imp.quality_row
            .set_tooltip_text(Some(&format!("Resolution score {} / 100", quality.score)));
        imp.quality_row.set_visible(true);
        match quality.class {
            crate::quality::QualityClass::Excellent | crate::quality::QualityClass::Good => {
                imp.quality_class_label.add_css_class("success");
            }
            crate::quality::QualityClass::Fair => {
                imp.quality_class_label.add_css_class("warning");
            }
            crate::quality::QualityClass::Poor | crate::quality::QualityClass::NeedsUpscale => {
                imp.quality_class_label.add_css_class("error");
            }
        }
        let active_segments = (quality.score as usize).div_ceil(20);
        for (index, segment) in imp.quality_segments.iter().enumerate() {
            if index < active_segments {
                segment.add_css_class("active");
            } else {
                segment.remove_css_class("active");
            }
        }
    }

    /// Buckets the image belongs to: always visible on the chip (D18).
    pub fn update_collections(&self, names: &[String]) {
        let imp = self.imp();
        if names.is_empty() {
            imp.popover_collections_label.set_text("In no buckets");
            imp.buckets_label.set_text("");
            imp.buckets_label.set_visible(false);
        } else {
            let list = names.join(", ");
            imp.popover_collections_label
                .set_text(&format!("Buckets: {list}"));
            imp.buckets_label.set_text(&format!("In {list}"));
            imp.buckets_label.set_visible(true);
        }
    }

    pub fn update_duplicate_count(&self, count: usize) {
        let imp = self.imp();
        imp.duplicate_count.set(count);
        if count == 0 {
            imp.popover_duplicates_label.set_text("");
            imp.popover_duplicates_label.set_visible(false);
        } else {
            let copies = if count == 1 { "copy" } else { "copies" };
            imp.popover_duplicates_label
                .set_text(&format!("{count} identical {copies} elsewhere"));
            imp.popover_duplicates_label.set_visible(true);
        }
    }

    /// Hide and clear all labels (e.g. before a new image loads).
    pub fn clear(&self) {
        self.hide_info();
        let imp = self.imp();
        imp.metadata_label.set_text("");
        imp.has_metadata.set(false);
        imp.metadata_label.set_visible(false);
        imp.below_4k_label.set_visible(false);
        imp.info_title.set_text("");
        imp.info_location.set_text("");
        imp.info_file.set_text("");
        imp.info_camera.set_visible(false);
        self.update_quality(None);
        self.update_collections(&[]);
        self.update_duplicate_count(0);
        imp.card.set_visible(false);
    }

    fn sync_visibility(&self) {
        let imp = self.imp();
        imp.card
            .set_visible(imp.enabled.get() && imp.has_metadata.get());
    }
}

impl Default for MetadataChip {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_k_counts_either_orientation() {
        assert!(reaches_4k((3840, 2160)));
        assert!(reaches_4k((2160, 3840)));
        assert!(!reaches_4k((3840, 1600)));
        assert!(!reaches_4k((2560, 1440)));
    }
}
