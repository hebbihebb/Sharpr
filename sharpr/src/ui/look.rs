//! One application stylesheet/provider, plus validated user-colour rules.
use gtk4::{gdk, gio, prelude::*};
use std::cell::RefCell;
use std::collections::BTreeMap;

struct Style {
    provider: gtk4::CssProvider,
    rules: BTreeMap<String, String>,
}
thread_local! { static STYLE: RefCell<Option<Style>> = const { RefCell::new(None) }; }

fn with_style(run: impl FnOnce(&mut Style)) {
    STYLE.with(|state| {
        let mut state = state.borrow_mut();
        if state.is_none() {
            let Some(display) = gdk::Display::default() else {
                return;
            };
            let provider = gtk4::CssProvider::new();
            provider.load_from_resource("/io/github/hebbihebb/Sharpr/style.css");
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 10,
            );
            *state = Some(Style {
                provider,
                rules: BTreeMap::new(),
            });
        }
        run(state.as_mut().unwrap());
    });
}

/// Registers a persisted colour without allowing arbitrary CSS into the sheet.
/// The filmstrip class spelling is stable for existing badge checks.
pub fn color_class(color: &str, pill: bool) -> String {
    let Ok(rgba) = gdk::RGBA::parse(color) else {
        return "sharpr-invalid-color".into();
    };
    let mut key = format!(
        "{:02x}{:02x}{:02x}",
        (rgba.red() * 255.0).round() as u8,
        (rgba.green() * 255.0).round() as u8,
        (rgba.blue() * 255.0).round() as u8
    );
    if rgba.alpha() < 1.0 {
        key.push_str(&format!("{:02x}", (rgba.alpha() * 255.0).round() as u8));
    }
    let class = format!(
        "{}-{key}",
        if pill {
            "filmstrip-pill-color"
        } else {
            "sharpr-icon-color"
        }
    );
    with_style(|style| {
        if style.rules.contains_key(&class) {
            return;
        }
        let rule = if pill {
            format!(".{class} {{ background-color: alpha({rgba}, 0.92); }}")
        } else {
            format!(".{class} {{ color: {rgba}; }}")
        };
        style.rules.insert(class.clone(), rule);
        let bytes = gio::resources_lookup_data(
            "/io/github/hebbihebb/Sharpr/style.css",
            gio::ResourceLookupFlags::NONE,
        )
        .expect("embedded stylesheet");
        let mut css = std::str::from_utf8(bytes.as_ref())
            .expect("UTF-8 stylesheet")
            .to_owned();
        for rule in style.rules.values() {
            css.push('\n');
            css.push_str(rule);
        }
        style.provider.load_from_string(&css);
    });
    class
}

pub fn cairo_color(context: &gtk4::cairo::Context, (r, g, b, a): crate::palette::Ink) {
    context.set_source_rgba(r, g, b, a);
}
pub fn gdk_color((r, g, b, a): crate::palette::Ink) -> gdk::RGBA {
    gdk::RGBA::new(r as f32, g as f32, b as f32, a as f32)
}

pub fn install(widget: &impl IsA<gtk4::Widget>) {
    super::fonts::install(widget);
    with_style(|_| {});
    let style = libadwaita::StyleManager::default();
    let update = |widget: &gtk4::Widget, high: bool| {
        if high {
            widget.add_css_class("sharpr-high-contrast");
        } else {
            widget.remove_css_class("sharpr-high-contrast");
        }
    };
    update(widget.as_ref(), style.is_high_contrast());
    let weak = widget.as_ref().downgrade();
    style.connect_high_contrast_notify(move |style| {
        if let Some(widget) = weak.upgrade() {
            update(&widget, style.is_high_contrast());
        }
    });
    if let Some(display) = gdk::Display::default() {
        gtk4::IconTheme::for_display(&display)
            .add_resource_path("/io/github/hebbihebb/Sharpr/icons");
    }
}

/// Style empty thumbnail pictures using GTK-supported state. Paintable changes
/// include factory rebinds and async completions; no scheduler state is touched.
pub fn thumbnail_feedback(picture: &gtk4::Picture) {
    fn update(picture: &gtk4::Picture) {
        if picture.paintable().is_none() {
            picture.add_css_class("sharpr-thumbnail-loading");
        } else {
            picture.remove_css_class("sharpr-thumbnail-loading");
        }
    }
    update(picture);
    picture.connect_paintable_notify(update);
}
