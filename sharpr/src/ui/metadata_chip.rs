use std::sync::Once;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::metadata::ImageMetadata;
use crate::quality::QualityScore;

// ---------------------------------------------------------------------------
// MetadataChip — compact bottom-right OSD chip for metadata + quality
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    pub struct MetadataChip {
        pub card: gtk4::Box,
        pub metadata_label: gtk4::Label,
        pub quality_row: gtk4::Box,
        pub quality_score_label: gtk4::Label,
        pub quality_class_label: gtk4::Label,
        pub quality_segments: [gtk4::Box; 5],
        pub enabled: std::cell::Cell<bool>,
        pub has_metadata: std::cell::Cell<bool>,
        pub has_quality: std::cell::Cell<bool>,
    }

    impl Default for MetadataChip {
        fn default() -> Self {
            let card = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            card.add_css_class("osd");
            card.add_css_class("metadata-osd");
            card.set_valign(gtk4::Align::End);
            card.set_margin_top(12);
            card.set_margin_bottom(12);
            card.set_margin_start(12);
            card.set_margin_end(12);
            card.set_visible(false);

            let metadata_label = gtk4::Label::new(None);
            metadata_label.set_halign(gtk4::Align::Start);
            metadata_label.set_xalign(0.0);
            metadata_label.add_css_class("caption");
            metadata_label.add_css_class("metadata-osd-meta");
            metadata_label.set_wrap(false);
            metadata_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            metadata_label.set_max_width_chars(36);

            let quality_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            quality_row.set_halign(gtk4::Align::Start);
            quality_row.add_css_class("metadata-osd-quality-row");

            let quality_score_label = gtk4::Label::new(Some("IQ --"));
            quality_score_label.set_halign(gtk4::Align::Start);
            quality_score_label.set_xalign(0.0);
            quality_score_label.add_css_class("metadata-osd-quality-score");

            let separator = gtk4::Label::new(Some("·"));
            separator.add_css_class("metadata-osd-dot");

            let quality_class_label = gtk4::Label::new(None);
            quality_class_label.set_halign(gtk4::Align::Start);
            quality_class_label.set_xalign(0.0);
            quality_class_label.add_css_class("metadata-osd-quality-class");

            let segment_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
            segment_row.add_css_class("metadata-osd-segments");
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

            card.append(&metadata_label);
            card.append(&quality_row);

            Self {
                card,
                metadata_label,
                quality_row,
                quality_score_label,
                quality_class_label,
                quality_segments,
                enabled: std::cell::Cell::new(true),
                has_metadata: std::cell::Cell::new(false),
                has_quality: std::cell::Cell::new(false),
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

impl MetadataChip {
    pub fn new() -> Self {
        install_css();
        let widget: Self = glib::Object::new();
        widget.imp().card.set_parent(&widget);
        widget
    }

    pub fn update_metadata(&self, meta: &ImageMetadata) {
        let imp = self.imp();
        let mut parts = Vec::new();
        if let Some(dimensions) = meta.dimensions_display() {
            parts.push(dimensions.replace(" × ", "×"));
        }
        if !meta.format.is_empty() {
            parts.push(meta.format.clone());
        }
        let size = meta.file_size_display();
        if !size.is_empty() {
            parts.push(size);
        }

        let metadata_text = parts.join(" · ");
        imp.metadata_label.set_text(&metadata_text);
        let has_metadata = !metadata_text.is_empty();
        imp.has_metadata.set(has_metadata);
        imp.metadata_label.set_visible(has_metadata);
        self.sync_visibility();
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.imp().enabled.set(enabled);
        self.sync_visibility();
    }

    pub fn update_quality(&self, quality: Option<&QualityScore>) {
        let imp = self.imp();
        let Some(quality) = quality else {
            imp.has_quality.set(false);
            imp.quality_score_label.set_text("IQ --");
            imp.quality_class_label.set_text("");
            imp.quality_row.set_visible(false);
            for class_name in ["success", "warning", "error"] {
                imp.quality_class_label.remove_css_class(class_name);
            }
            for segment in &imp.quality_segments {
                segment.remove_css_class("active");
            }
            self.sync_visibility();
            return;
        };

        imp.has_quality.set(true);
        imp.quality_score_label
            .set_text(&format!("IQ {}%", quality.score));
        imp.quality_class_label.set_text(quality.class.label());
        imp.quality_class_label
            .set_tooltip_text(Some(&quality.tooltip()));
        imp.quality_row.set_visible(true);

        for class_name in ["success", "warning", "error"] {
            imp.quality_class_label.remove_css_class(class_name);
        }
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
        self.sync_visibility();
    }

    /// Hide and clear all labels (e.g. before a new image loads).
    pub fn clear(&self) {
        let imp = self.imp();
        imp.metadata_label.set_text("");
        imp.has_metadata.set(false);
        imp.metadata_label.set_visible(false);
        imp.quality_score_label.set_text("IQ --");
        imp.quality_class_label.set_text("");
        imp.quality_class_label.set_tooltip_text(None);
        imp.has_quality.set(false);
        imp.quality_row.set_visible(false);
        for class_name in ["success", "warning", "error"] {
            imp.quality_class_label.remove_css_class(class_name);
        }
        for segment in &imp.quality_segments {
            segment.remove_css_class("active");
        }
        imp.card.set_visible(false);
    }

    fn sync_visibility(&self) {
        let imp = self.imp();
        let has_content = imp.has_metadata.get() || imp.has_quality.get();
        let visible = imp.enabled.get() && has_content;
        imp.card.set_visible(visible);
    }
}

impl Default for MetadataChip {
    fn default() -> Self {
        Self::new()
    }
}

fn install_css() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(
            "
            .metadata-osd {
                padding: 10px 12px 8px 12px;
                border-radius: 16px;
                background-color: rgba(28, 28, 30, 0.72);
                color: white;
                box-shadow: 0 6px 18px rgba(0, 0, 0, 0.18);
            }
            .metadata-osd-meta {
                color: rgba(255, 255, 255, 0.78);
                font-size: 0.92em;
            }
            .metadata-osd-quality-row {
                margin-top: 1px;
            }
            .metadata-osd-quality-score,
            .metadata-osd-quality-class {
                font-weight: 600;
            }
            .metadata-osd-dot {
                color: rgba(255, 255, 255, 0.62);
            }
            .metadata-osd-segments {
                margin-left: 4px;
            }
            .metadata-osd-segment {
                min-width: 11px;
                min-height: 6px;
                border-radius: 999px;
                background-color: rgba(255, 255, 255, 0.20);
            }
            .metadata-osd-segment.active {
                background-color: rgba(255, 255, 255, 0.90);
            }
            ",
        );
        if let Some(display) = gdk4::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
}
