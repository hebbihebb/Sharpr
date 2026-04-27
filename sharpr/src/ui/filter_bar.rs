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
        pub quality_buttons: Vec<(gtk4::ToggleButton, QualityClass)>,
        pub tag_entry: gtk4::SearchEntry,
        pub reset_btn: gtk4::Button,
        pub tags_db: RefCell<Option<Arc<TagDatabase>>>,
        pub filters_changed_cb: RefCell<Option<FiltersChangedCallback>>,
        /// Prevents re-entrancy during reset().
        pub suppress_signals: Cell<bool>,
    }

    impl Default for FilterBar {
        fn default() -> Self {
            let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            container.set_margin_start(6);
            container.set_margin_end(6);
            container.set_margin_top(4);
            container.set_margin_bottom(4);

            let quality_buttons: Vec<(gtk4::ToggleButton, QualityClass)> = QualityClass::ALL
                .iter()
                .map(|&class| {
                    let btn = gtk4::ToggleButton::with_label(class.label());
                    btn.add_css_class("pill");
                    (btn, class)
                })
                .collect();

            let tag_entry = gtk4::SearchEntry::new();
            tag_entry.set_placeholder_text(Some("Filter by tag…"));
            tag_entry.set_hexpand(true);

            let reset_btn = gtk4::Button::with_label("Reset");
            reset_btn.add_css_class("flat");
            reset_btn.set_sensitive(false);

            Self {
                container,
                quality_buttons,
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
            for (btn, _) in &self.quality_buttons {
                self.container.append(btn);
            }
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

        for (btn, class) in self.imp().quality_buttons.clone() {
            let this_weak = this_weak.clone();
            btn.connect_toggled(move |_| {
                let Some(this) = this_weak.upgrade() else { return };
                let imp = this.imp();
                if imp.suppress_signals.get() {
                    return;
                }
                imp.suppress_signals.set(true);
                for (other_btn, other_class) in &imp.quality_buttons {
                    if *other_class != class {
                        other_btn.set_active(false);
                    }
                }
                imp.suppress_signals.set(false);
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
            .quality_buttons
            .iter()
            .find(|(btn, _)| btn.is_active())
            .map(|(_, class)| *class);
        let tag_text = imp.tag_entry.text().to_string();
        let tag = if tag_text.trim().is_empty() {
            None
        } else {
            Some(tag_text.trim().to_string())
        };
        ActiveFilters { quality, tag }
    }

    /// Clears all chips and the tag entry without emitting filters_changed.
    pub fn reset(&self) {
        let imp = self.imp();
        imp.suppress_signals.set(true);
        for (btn, _) in &imp.quality_buttons {
            btn.set_active(false);
        }
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
