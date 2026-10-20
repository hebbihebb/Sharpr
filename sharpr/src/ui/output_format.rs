//! Shared plain-GTK format picker and honest format-specific guidance.
use crate::export::OutputFormat;
use gtk4::prelude::*;

pub fn selected(picker: &gtk4::DropDown) -> OutputFormat {
    match picker.selected() {
        1 => OutputFormat::Webp,
        2 => OutputFormat::Original,
        _ => OutputFormat::Png,
    }
}

pub fn picker(format: OutputFormat) -> gtk4::DropDown {
    let picker = gtk4::DropDown::from_strings(&["PNG", "WebP", "Original"]);
    picker.set_widget_name("output-format");
    picker.set_tooltip_text(Some("Output format"));
    picker.set_selected(match format {
        OutputFormat::Png => 0,
        OutputFormat::Webp => 1,
        OutputFormat::Original => 2,
    });
    picker
}

pub fn note(picker: &gtk4::DropDown, special_source: bool) -> gtk4::Label {
    let label = gtk4::Label::new(None);
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_max_width_chars(64);
    let update = move |picker: &gtk4::DropDown, label: &gtk4::Label| {
        let text = match selected(picker) {
            OutputFormat::Png if special_source => "Lossless PNG keeps decoded 8/16-bit precision and transparency. JPEG XL, AVIF and HEIC color-profile metadata is not copied; AVIF/HEIC decode at 8-bit. Check colors in your editor. PNG cannot preserve floating-point/HDR samples.",
            OutputFormat::Png => "Lossless PNG keeps decoded 8/16-bit precision, transparency and embedded ICC profiles and PNG color metadata. PNG cannot preserve floating-point/HDR samples.",
            OutputFormat::Webp => "WebP uses 8-bit color: higher-depth samples are converted. Color profiles are not copied. Crop Copy always uses lossless WebP.",
            OutputFormat::Original => "Each image keeps its source format. JPEG, AVIF and HEIC are re-encoded at maximum quality; AVIF/HEIC output is 8-bit. Other metadata and color profiles may not be retained.",
        };
        label.set_text(text);
    };
    update(picker, &label);
    let weak = label.downgrade();
    picker.connect_selected_notify(move |picker| {
        if let Some(label) = weak.upgrade() {
            update(picker, &label);
        }
    });
    label
}

pub fn has_special_source(sources: &[std::path::PathBuf]) -> bool {
    sources.iter().any(|p| {
        crate::jxl::is_jxl_path(p) || crate::visual_search::prepare::heif_format(p).is_some()
    })
}

pub fn row(title: &str, control: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let label = gtk4::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_mnemonic_widget(Some(control));
    row.append(&label);
    row.append(control);
    row
}
