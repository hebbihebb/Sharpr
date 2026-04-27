use std::cell::{Cell, RefCell};
use std::sync::Arc;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::quality::QualityClass;
use crate::tags::TagDatabase;

#[derive(Clone, Debug, Default)]
pub struct ActiveFilters {
    pub quality: Option<QualityClass>,
    pub tag: Option<String>,
}

impl ActiveFilters {
    pub fn is_empty(&self) -> bool {
        self.quality.is_none() && self.tag.is_none()
    }
}

type FiltersChangedCallback = Box<dyn Fn(ActiveFilters) + 'static>;

mod imp {
    use super::*;

    pub struct FilterBar {
        pub container: gtk4::Box,
        pub quality_btn: gtk4::MenuButton,
        /// (radio, None = "All Quality", Some(class) = specific class)
        pub quality_radios: Vec<(gtk4::CheckButton, Option<QualityClass>)>,
        pub tag_entry: gtk4::SearchEntry,
        pub reset_btn: gtk4::Button,
        pub tags_db: RefCell<Option<Arc<TagDatabase>>>,
        pub filters_changed_cb: RefCell<Option<FiltersChangedCallback>>,
        pub suppress_signals: Cell<bool>,
    }

    impl Default for FilterBar {
        fn default() -> Self {
            let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            container.set_margin_start(8);
            container.set_margin_end(8);
            container.set_margin_top(3);
            container.set_margin_bottom(3);

            // Quality menu button — compact, flat, opens a popover.
            let quality_btn = gtk4::MenuButton::new();
            quality_btn.set_label("Quality");
            quality_btn.add_css_class("flat");

            // Popover contents: "All" + one radio per QualityClass.
            let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            popover_box.set_margin_start(6);
            popover_box.set_margin_end(6);
            popover_box.set_margin_top(6);
            popover_box.set_margin_bottom(6);

            let all_radio = gtk4::CheckButton::with_label("All");
            all_radio.set_active(true);
            popover_box.append(&all_radio);

            let mut quality_radios: Vec<(gtk4::CheckButton, Option<QualityClass>)> = Vec::new();
            quality_radios.push((all_radio.clone(), None));

            for &class in QualityClass::ALL.iter() {
                let radio = gtk4::CheckButton::with_label(class.label());
                radio.set_group(Some(&all_radio));
                popover_box.append(&radio);
                quality_radios.push((radio, Some(class)));
            }

            let popover = gtk4::Popover::new();
            popover.set_child(Some(&popover_box));
            quality_btn.set_popover(Some(&popover));

            let tag_entry = gtk4::SearchEntry::new();
            tag_entry.set_placeholder_text(Some("Filter by tag…"));
            tag_entry.set_hexpand(true);

            let reset_btn = gtk4::Button::with_label("Reset");
            reset_btn.add_css_class("flat");
            reset_btn.set_sensitive(false);

            Self {
                container,
                quality_btn,
                quality_radios,
                tag_entry,
                reset_btn,
                tags_db: RefCell::new(None),
                filters_changed_cb: RefCell::new(None),
                suppress_signals: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FilterBar {
        const NAME: &'static str = "SharprFilterBar";
        type Type = super::FilterBar;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for FilterBar {
        fn constructed(&self) {
            self.parent_constructed();
            self.container.append(&self.quality_btn);
            self.container.append(&self.tag_entry);
            self.container.append(&self.reset_btn);
            self.container.set_parent(&*self.obj());
        }

        fn dispose(&self) {
            self.container.unparent();
        }
    }

    impl WidgetImpl for FilterBar {}
}

glib::wrapper! {
    pub struct FilterBar(ObjectSubclass<imp::FilterBar>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl FilterBar {
    pub fn new(tags_db: Option<Arc<TagDatabase>>) -> Self {
        let this: Self = glib::Object::new();
        *this.imp().tags_db.borrow_mut() = tags_db;
        this.wire_signals();
        this
    }

    fn wire_signals(&self) {
        let this_weak = self.downgrade();

        for (radio, class) in self.imp().quality_radios.clone() {
            let this_weak = this_weak.clone();
            radio.connect_toggled(move |btn| {
                if !btn.is_active() {
                    return;
                }
                let Some(this) = this_weak.upgrade() else { return };
                let imp = this.imp();
                if imp.suppress_signals.get() {
                    return;
                }
                // Update button label to reflect active selection.
                let label = match class {
                    Some(c) => c.label(),
                    None => "Quality",
                };
                imp.quality_btn.set_label(label);
                imp.reset_btn.set_sensitive(!this.current_filters().is_empty());
                this.emit_filters_changed();
            });
        }

        {
            let this_weak = this_weak.clone();
            self.imp().tag_entry.connect_search_changed(move |_| {
                let Some(this) = this_weak.upgrade() else { return };
                if this.imp().suppress_signals.get() {
                    return;
                }
                this.imp()
                    .reset_btn
                    .set_sensitive(!this.current_filters().is_empty());
                this.emit_filters_changed();
            });
        }

        {
            self.imp().reset_btn.connect_clicked(move |_| {
                let Some(this) = this_weak.upgrade() else { return };
                this.reset();
                this.emit_filters_changed();
            });
        }
    }

    pub fn current_filters(&self) -> ActiveFilters {
        let imp = self.imp();
        let quality = imp
            .quality_radios
            .iter()
            .find(|(btn, _)| btn.is_active())
            .and_then(|(_, class)| *class);
        let tag_text = imp.tag_entry.text().to_string();
        let tag = if tag_text.trim().is_empty() {
            None
        } else {
            Some(tag_text.trim().to_string())
        };
        ActiveFilters { quality, tag }
    }

    /// Clears quality selection back to "All" and empties tag entry, without emitting filters_changed.
    pub fn reset(&self) {
        let imp = self.imp();
        imp.suppress_signals.set(true);
        if let Some((all_radio, _)) = imp.quality_radios.first() {
            all_radio.set_active(true);
        }
        imp.quality_btn.set_label("Quality");
        imp.tag_entry.set_text("");
        imp.reset_btn.set_sensitive(false);
        imp.suppress_signals.set(false);
    }

    pub fn connect_filters_changed<F: Fn(ActiveFilters) + 'static>(&self, f: F) {
        *self.imp().filters_changed_cb.borrow_mut() = Some(Box::new(f));
    }

    fn emit_filters_changed(&self) {
        if let Some(cb) = self.imp().filters_changed_cb.borrow().as_ref() {
            cb(self.current_filters());
        }
    }
}
