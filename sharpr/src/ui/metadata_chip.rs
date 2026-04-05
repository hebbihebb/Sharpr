use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::metadata::ImageMetadata;

// ---------------------------------------------------------------------------
// MetadataChip — floating overlay at the bottom-right of the viewer
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    pub struct MetadataChip {
        pub card: gtk4::Box,
        pub filename_label: gtk4::Label,
        pub dims_label: gtk4::Label,
        pub size_label: gtk4::Label,
        pub camera_label: gtk4::Label,
        pub exif_label: gtk4::Label,
        pub lens_label: gtk4::Label,
    }

    impl Default for MetadataChip {
        fn default() -> Self {
            // Outer box with a card-style frame
            let card = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            card.add_css_class("card");
            card.set_margin_top(12);
            card.set_margin_bottom(12);
            card.set_margin_start(12);
            card.set_margin_end(12);

            let filename_label = gtk4::Label::new(None);
            filename_label.set_halign(gtk4::Align::Start);
            filename_label.add_css_class("caption-heading");
            filename_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            filename_label.set_max_width_chars(30);

            let dims_label = gtk4::Label::new(None);
            dims_label.set_halign(gtk4::Align::Start);
            dims_label.add_css_class("caption");
            dims_label.add_css_class("dim-label");

            let size_label = gtk4::Label::new(None);
            size_label.set_halign(gtk4::Align::Start);
            size_label.add_css_class("caption");
            size_label.add_css_class("dim-label");

            let camera_label = gtk4::Label::new(None);
            camera_label.set_halign(gtk4::Align::Start);
            camera_label.add_css_class("caption");
            camera_label.add_css_class("dim-label");

            let exif_label = gtk4::Label::new(None);
            exif_label.set_halign(gtk4::Align::Start);
            exif_label.add_css_class("caption");
            exif_label.add_css_class("dim-label");

            let lens_label = gtk4::Label::new(None);
            lens_label.set_halign(gtk4::Align::Start);
            lens_label.add_css_class("caption");
            lens_label.add_css_class("dim-label");
            lens_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            lens_label.set_max_width_chars(36);

            card.append(&filename_label);
            card.append(&dims_label);
            card.append(&size_label);
            card.append(&camera_label);
            card.append(&exif_label);
            card.append(&lens_label);

            Self {
                card,
                filename_label,
                dims_label,
                size_label,
                camera_label,
                exif_label,
                lens_label,
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
        @extends gtk4::Widget;
}

impl MetadataChip {
    pub fn new() -> Self {
        let widget: Self = glib::Object::new();
        widget.imp().card.set_parent(&widget);
        widget
    }

    /// Populate the chip with data from an `ImageMetadata` snapshot.
    pub fn update(&self, meta: &ImageMetadata) {
        let imp = self.imp();

        imp.filename_label.set_text(&meta.filename);

        // Dimensions + megapixels.
        let dims_text = match meta.dimensions_display() {
            Some(d) => match meta.megapixels_display() {
                Some(mp) => format!("{} ({}) • {}", d, mp, meta.format),
                None => format!("{} • {}", d, meta.format),
            },
            None => meta.format.clone(),
        };
        imp.dims_label.set_text(&dims_text);
        imp.dims_label.set_visible(!dims_text.is_empty());

        // File size.
        imp.size_label.set_text(&meta.file_size_display());
        imp.size_label.set_visible(meta.file_size_bytes > 0);

        // Camera model.
        if let Some(ref cam) = meta.camera {
            imp.camera_label.set_text(cam);
            imp.camera_label.set_visible(true);
        } else {
            imp.camera_label.set_text("");
            imp.camera_label.set_visible(false);
        }

        let exif_text = build_exif_text(meta);
        imp.exif_label.set_text(&exif_text);
        imp.exif_label.set_visible(!exif_text.is_empty());

        let lens_text = build_lens_text(meta);
        imp.lens_label.set_text(&lens_text);
        imp.lens_label.set_visible(!lens_text.is_empty());

        self.set_visible(true);
    }

    /// Hide and clear all labels (e.g. before a new image loads).
    pub fn clear(&self) {
        let imp = self.imp();
        for label in [
            &imp.filename_label,
            &imp.dims_label,
            &imp.size_label,
            &imp.camera_label,
            &imp.exif_label,
            &imp.lens_label,
        ] {
            label.set_text("");
            label.set_visible(false);
        }
    }
}

impl Default for MetadataChip {
    fn default() -> Self {
        Self::new()
    }
}

fn build_exif_text(meta: &ImageMetadata) -> String {
    let mut parts = Vec::new();
    if let Some(ref iso) = meta.iso {
        parts.push(format!("ISO {}", iso));
    }
    if let Some(ref ss) = meta.shutter_speed {
        parts.push(format!("{}s", ss));
    }
    if let Some(ref ap) = meta.aperture {
        parts.push(ap.clone());
    }
    if let Some(ref fl) = meta.focal_length {
        parts.push(fl.clone());
    }
    parts.join(" · ")
}

fn build_lens_text(meta: &ImageMetadata) -> String {
    let mut parts = Vec::new();
    if let Some(ref lens) = meta.lens {
        parts.push(lens.clone());
    }
    if let Some(ref cs) = meta.color_space {
        parts.push(cs.clone());
    }
    parts.join(" · ")
}
