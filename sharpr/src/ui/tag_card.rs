use std::sync::Once;

use gtk4::gdk;
use gtk4::prelude::*;

#[derive(Clone)]
pub struct TagCard {
    root: gtk4::Frame,
    accent: gtk4::Box,
    picture: gtk4::Picture,
    placeholder: gtk4::Box,
    icon: gtk4::Image,
    name_label: gtk4::Label,
    count_label: gtk4::Label,
    menu_button: gtk4::MenuButton,
}

impl TagCard {
    pub fn new() -> Self {
        install_css();

        let root = gtk4::Frame::new(None);
        root.add_css_class("tag-card");
        root.set_focusable(true);
        root.set_size_request(280, -1);

        let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let preview_overlay = gtk4::Overlay::new();
        preview_overlay.add_css_class("tag-card-preview");
        preview_overlay.set_overflow(gtk4::Overflow::Hidden);

        let preview_dummy = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        preview_dummy.set_size_request(-1, 136);
        preview_overlay.set_child(Some(&preview_dummy));

        let picture = gtk4::Picture::new();
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk4::ContentFit::Cover);
        picture.set_halign(gtk4::Align::Fill);
        picture.set_valign(gtk4::Align::Fill);
        preview_overlay.add_overlay(&picture);

        let placeholder = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        placeholder.set_halign(gtk4::Align::Center);
        placeholder.set_valign(gtk4::Align::Center);
        placeholder.add_css_class("tag-card-placeholder");
        let placeholder_icon = gtk4::Image::from_icon_name("image-x-generic-symbolic");
        placeholder_icon.set_pixel_size(28);
        let placeholder_label = gtk4::Label::new(Some("Preview pending"));
        placeholder_label.add_css_class("dim-label");
        placeholder.append(&placeholder_icon);
        placeholder.append(&placeholder_label);
        preview_overlay.add_overlay(&placeholder);

        let menu_button = gtk4::MenuButton::new();
        menu_button.set_icon_name("view-more-symbolic");
        menu_button.add_css_class("flat");
        menu_button.add_css_class("tag-card-menu");
        menu_button.set_halign(gtk4::Align::End);
        menu_button.set_valign(gtk4::Align::Start);
        menu_button.set_margin_top(8);
        menu_button.set_margin_end(8);
        preview_overlay.add_overlay(&menu_button);

        let accent = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        accent.add_css_class("tag-card-accent");
        accent.set_halign(gtk4::Align::Fill);
        accent.set_valign(gtk4::Align::End);
        accent.set_height_request(4);
        preview_overlay.add_overlay(&accent);

        outer.append(&preview_overlay);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let title_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let icon = gtk4::Image::from_icon_name("bookmark-new-symbolic");
        icon.set_pixel_size(16);
        icon.add_css_class("tag-card-icon");
        title_row.append(&icon);

        let title_col = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        title_col.set_hexpand(true);

        let name_label = gtk4::Label::new(None);
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_xalign(0.0);
        name_label.add_css_class("heading");
        name_label.add_css_class("tag-card-title");
        name_label.set_wrap(true);
        title_col.append(&name_label);

        let count_label = gtk4::Label::new(None);
        count_label.set_halign(gtk4::Align::Start);
        count_label.set_xalign(0.0);
        count_label.add_css_class("dim-label");
        title_col.append(&count_label);

        title_row.append(&title_col);
        content.append(&title_row);
        outer.append(&content);
        root.set_child(Some(&outer));

        Self {
            root,
            accent,
            picture,
            placeholder,
            icon,
            name_label,
            count_label,
            menu_button,
        }
    }

    pub fn widget(&self) -> &gtk4::Frame {
        &self.root
    }

    pub fn menu_button(&self) -> &gtk4::MenuButton {
        &self.menu_button
    }

    pub fn set_label(&self, label: &str) {
        self.name_label.set_label(label);
        self.root.set_tooltip_text(Some(label));
    }

    pub fn set_count(&self, count: usize) {
        let noun = if count == 1 { "image" } else { "images" };
        self.count_label.set_label(&format!("{count} {noun}"));
    }

    pub fn set_preview_texture(&self, texture: Option<&gdk::Texture>) {
        if let Some(texture) = texture {
            self.picture.set_paintable(Some(texture));
            self.placeholder.set_visible(false);
        } else {
            self.picture.set_paintable(None::<&gdk::Paintable>);
            self.placeholder.set_visible(true);
        }
    }

    pub fn set_accent_color(&self, color: Option<&str>) {
        if let Some(color) = color {
            let (accent_class, icon_class) = register_accent_classes(color);
            self.accent.add_css_class(&accent_class);
            self.icon.add_css_class(&icon_class);
            self.accent.set_visible(true);
        } else {
            self.accent.set_visible(false);
        }
    }

    pub fn set_selected(&self, selected: bool) {
        if selected {
            self.root.add_css_class("selected");
        } else {
            self.root.remove_css_class("selected");
        }
    }
}

fn install_css() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(
            "
            .tag-card {
                border-radius: 18px;
                background: alpha(@window_fg_color, 0.04);
                border: 1px solid alpha(@window_fg_color, 0.08);
                box-shadow: 0 10px 30px alpha(black, 0.08);
            }
            .tag-card:hover {
                background: alpha(@window_fg_color, 0.06);
                border-color: alpha(@accent_color, 0.35);
            }
            .tag-card:focus-visible {
                outline: 2px solid alpha(@accent_color, 0.75);
                outline-offset: 2px;
            }
            .tag-card.selected {
                border-color: alpha(@accent_color, 0.9);
                background: alpha(@accent_color, 0.10);
                box-shadow: 0 14px 30px alpha(@accent_color, 0.12);
            }
            .tag-card-preview {
                border-top-left-radius: 18px;
                border-top-right-radius: 18px;
                background: linear-gradient(160deg, alpha(@accent_color, 0.14), alpha(@window_fg_color, 0.03));
            }
            .tag-card-placeholder {
                color: alpha(@window_fg_color, 0.7);
            }
            .tag-card-menu {
                background: alpha(black, 0.25);
                color: white;
                border-radius: 999px;
            }
            .tag-card-menu:hover {
                background: alpha(black, 0.4);
            }
            .tag-card-accent {
                min-height: 4px;
            }
            .tag-card-icon {
                color: alpha(@window_fg_color, 0.72);
            }
            .tag-card-title {
                font-weight: 600;
            }
            ",
        );
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
}

fn register_accent_classes(color: &str) -> (String, String) {
    use std::sync::{LazyLock, Mutex};

    static REGISTERED: LazyLock<Mutex<std::collections::HashSet<String>>> =
        LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

    let key = color
        .replace([' ', '(', ')', ',', '#', '.'], "_")
        .to_lowercase();
    let accent_class = format!("tag-card-accent-{key}");
    let icon_class = format!("tag-card-icon-{key}");

    if let Ok(mut registered) = REGISTERED.lock() {
        if registered.insert(key) {
            let provider = gtk4::CssProvider::new();
            provider.load_from_string(&format!(
                ".{accent_class} {{ background-color: {color}; }} .{icon_class} {{ color: {color}; }}"
            ));
            if let Some(display) = gdk::Display::default() {
                gtk4::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
        }
    }

    (accent_class, icon_class)
}
