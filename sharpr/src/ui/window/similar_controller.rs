//! Show Similar (Work Order item 28, D44): a search for the images like one
//! image, from the image menu. It runs through the search bar as a chip
//! with an Exact / Close / Similar switch, over the folder or bucket in view
//! or the whole library as the box says, so the results, Compare, buckets,
//! filter chips and Escape work as for any search.
use super::*;
use crate::visual_search::similar::Level;

impl SharprWindow {
    pub(super) fn setup_show_similar(&self) {
        let action = gio::SimpleAction::new("show-similar", Some(glib::VariantTy::STRING));
        let weak = self.downgrade();
        action.connect_activate(move |_, parameter| {
            let Some(window) = weak.upgrade() else { return };
            let Some(path) = parameter.and_then(|p| p.get::<String>()) else {
                return;
            };
            window.show_similar(PathBuf::from(path));
        });
        self.add_action(&action);
    }

    /// Lists the images like `path`, keeping the level last chosen in this
    /// search.
    pub(crate) fn show_similar(&self, path: PathBuf) {
        if file_actions_blocked(self.upcast_ref()) {
            return;
        }
        {
            let mut options = self.imp().browse_options.borrow_mut();
            let level = options
                .similar
                .as_ref()
                .map(|(_, c)| *c)
                .unwrap_or_default();
            options.similar = Some((path, level));
        }
        // The search bar shows the chip; the keys stay with the filmstrip.
        if let Some(revealer) = self.imp().inline_search_revealer.borrow().as_ref() {
            revealer.set_reveal_child(true);
        }
        self.imp().inline_search_suppressed.set(true);
        if let Some(entry) = self.imp().inline_search_entry.borrow().as_ref() {
            entry.set_text("");
        }
        self.imp().inline_search_suppressed.set(false);
        self.rebuild_search_chips();
        self.run_scoped_search(String::new());
    }

    /// The "similar to <name>" chip: the switch changes the level, the close
    /// button ends Show Similar.
    pub(super) fn similar_chip(&self, path: &Path, level: Level) -> gtk4::Box {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let pill = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        pill.add_css_class("linked");
        pill.add_css_class("sharpr-filter-chip");
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let label = gtk4::Button::with_label(&format!("similar to {name}"));
        label.set_tooltip_text(Some("Show this image"));
        let remove = gtk4::Button::from_icon_name("window-close-symbolic");
        remove.set_tooltip_text(Some("Stop showing similar images"));
        remove.update_property(&[gtk4::accessible::Property::Label("Remove similar filter")]);
        pill.append(&label);
        pill.append(&remove);
        row.append(&pill);

        let switch = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        switch.add_css_class("linked");
        let mut group: Option<gtk4::ToggleButton> = None;
        for option in Level::ALL {
            let button = gtk4::ToggleButton::with_label(option.label());
            button.add_css_class("sharpr-filter-chip");
            button.set_tooltip_text(Some(option.tooltip()));
            button.set_group(group.as_ref());
            button.set_active(option == level);
            group.get_or_insert_with(|| button.clone());
            let weak = self.downgrade();
            button.connect_toggled(move |button| {
                if !button.is_active() {
                    return;
                }
                let Some(win) = weak.upgrade() else { return };
                let changed = {
                    let mut options = win.imp().browse_options.borrow_mut();
                    match options.similar.as_mut() {
                        Some((_, current)) if *current != option => {
                            *current = option;
                            true
                        }
                        _ => false,
                    }
                };
                if changed {
                    win.run_scoped_search(win.inline_search_text());
                }
            });
            switch.append(&button);
        }
        row.append(&switch);

        let weak = self.downgrade();
        let shown = path.to_path_buf();
        label.connect_clicked(move |_| {
            let Some(win) = weak.upgrade() else { return };
            let index = win.imp().state.borrow().library.index_of_path(&shown);
            let filmstrip = win.imp().filmstrip.borrow().clone();
            if let (Some(index), Some(filmstrip)) = (index, filmstrip) {
                filmstrip.navigate_to(index);
            }
        });
        let weak = self.downgrade();
        remove.connect_clicked(move |_| {
            let Some(win) = weak.upgrade() else { return };
            win.imp().browse_options.borrow_mut().similar = None;
            win.rebuild_search_chips();
            win.run_scoped_search(win.inline_search_text());
        });
        row
    }

    fn inline_search_text(&self) -> String {
        self.imp()
            .inline_search_entry
            .borrow()
            .as_ref()
            .map(|e| e.text().to_string())
            .unwrap_or_default()
    }
}
