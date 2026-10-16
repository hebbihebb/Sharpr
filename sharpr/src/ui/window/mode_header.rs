//! Compare and crop use the viewer's existing header. Browsing's search
//! widgets stay alive, with their text, chips and scope left intact (D28).
use super::*;

/// The native HeaderBar owns the positioning of these three groups.
#[derive(Clone)]
pub(crate) struct ModeControls {
    pub start: gtk4::Box,
    pub center: gtk4::Box,
    pub end: gtk4::Box,
}

impl ModeControls {
    pub fn new() -> Self {
        Self {
            start: gtk4::Box::new(gtk4::Orientation::Horizontal, 8),
            center: gtk4::Box::new(gtk4::Orientation::Horizontal, 8),
            end: gtk4::Box::new(gtk4::Orientation::Horizontal, 8),
        }
    }
}

pub(super) struct ModeHeader {
    header: libadwaita::HeaderBar,
    start: gtk4::Box,
    end: gtk4::Box,
    browse: gtk4::Widget,
    chips: gtk4::Revealer,
}

impl ModeHeader {
    pub fn new(header: &libadwaita::HeaderBar, chips: &gtk4::Revealer) -> Self {
        let browse = header.title_widget().expect("browse header content");
        let start = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        let end = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        start.set_visible(false);
        end.set_visible(false);
        header.pack_start(&start);
        header.pack_end(&end);
        Self {
            header: header.clone(),
            start,
            end,
            browse,
            chips: chips.clone(),
        }
    }

    fn set(&self, content: Option<&ModeControls>) {
        for slot in [&self.start, &self.end] {
            if let Some(old) = slot.first_child() {
                slot.remove(&old);
            }
        }
        if let Some(content) = content {
            self.start.append(&content.start);
            self.end.append(&content.end);
            self.header.set_title_widget(Some(&content.center));
        } else {
            self.header.set_title_widget(Some(&self.browse));
        }
        self.start.set_visible(content.is_some());
        self.end.set_visible(content.is_some());
        self.chips.set_visible(content.is_none());
    }

    pub fn active(&self) -> bool {
        self.start.is_visible()
    }
}

impl SharprWindow {
    pub(super) fn setup_mode_panel_guards(
        &self,
        outer: &libadwaita::OverlaySplitView,
        inner: &libadwaita::OverlaySplitView,
    ) {
        for split in [outer, inner] {
            let window = self.downgrade();
            split.connect_collapsed_notify(move |split| {
                let active = window.upgrade().is_some_and(|w| {
                    w.imp().presentation_mode.get()
                        || w.imp()
                            .mode_header
                            .borrow()
                            .as_ref()
                            .is_some_and(ModeHeader::active)
                });
                if active && !split.is_collapsed() {
                    split.set_collapsed(true);
                    split.set_show_sidebar(false);
                }
            });
        }
    }

    pub(crate) fn set_mode_header(&self, content: Option<&ModeControls>) {
        if content.is_some() {
            // Retire in-flight browse results, including ones that could
            // otherwise arrive after returning and clear the survivors.
            let generation = self.imp().inline_search_generation.get().wrapping_add(1);
            self.imp().inline_search_generation.set(generation);
            self.imp()
                .browse_generation
                .store(generation, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(mode_header) = self.imp().mode_header.borrow().as_ref() {
            mode_header.set(content);
        }
        if let Some(header) = self.imp().presentation_header.borrow().as_ref() {
            header.set_visible(content.is_some() || !self.imp().presentation_mode.get());
        }
    }

    /// Window-level capture also reaches controls moved outside the canvas.
    /// Entries, picker popups and modal sheets retain their own keys.
    pub(super) fn handle_mode_key(
        &self,
        key: gdk4::Key,
        modifiers: gdk4::ModifierType,
    ) -> glib::Propagation {
        if self.visible_dialog().is_some() || text_input_has_focus(self.upcast_ref()) {
            return glib::Propagation::Proceed;
        }
        let mut focus = gtk4::prelude::GtkWindowExt::focus(self);
        while let Some(widget) = focus {
            if widget.is::<gtk4::Popover>() {
                return glib::Propagation::Proceed;
            }
            focus = widget.parent();
        }
        if let Some(crop) = self.crop_session() {
            if !modifiers.intersects(
                gdk4::ModifierType::CONTROL_MASK
                    | gdk4::ModifierType::ALT_MASK
                    | gdk4::ModifierType::SUPER_MASK,
            ) {
                match key {
                    gdk4::Key::Escape => crop.close(),
                    gdk4::Key::Return | gdk4::Key::KP_Enter => crop.save(),
                    _ => return glib::Propagation::Proceed,
                }
                return glib::Propagation::Stop;
            }
        } else if super::survey_controller::is_surveying(self.upcast_ref()) {
            if let Some(page) = self.imp().survey_page.borrow().as_ref() {
                return page.handle_key(key, modifiers);
            }
        } else if is_comparing(self.upcast_ref()) {
            if let Some(page) = self.imp().compare_page.borrow().as_ref() {
                return page.handle_key(key, modifiers);
            }
        }
        glib::Propagation::Proceed
    }
}
