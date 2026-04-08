use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::tags::TagDatabase;

type TagActivatedCallback = Box<dyn Fn(&str) + 'static>;

mod imp {
    use super::*;

    pub struct TagBrowser {
        pub scroll: gtk4::ScrolledWindow,
        pub content_box: gtk4::Box,
        pub tags: RefCell<Option<Arc<TagDatabase>>>,
        pub tag_activated_cb: RefCell<Option<TagActivatedCallback>>,
    }

    impl Default for TagBrowser {
        fn default() -> Self {
            let scroll = gtk4::ScrolledWindow::new();
            scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
            scroll.set_vexpand(true);
            Self {
                scroll,
                content_box: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
                tags: RefCell::new(None),
                tag_activated_cb: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TagBrowser {
        const NAME: &'static str = "SharprTagBrowser";
        type Type = super::TagBrowser;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for TagBrowser {
        fn dispose(&self) {
            self.scroll.unparent();
        }
    }

    impl WidgetImpl for TagBrowser {}
}

glib::wrapper! {
    pub struct TagBrowser(ObjectSubclass<imp::TagBrowser>)
        @extends gtk4::Widget;
}

impl TagBrowser {
    pub fn new(tags: Arc<TagDatabase>) -> Self {
        let widget: Self = glib::Object::new();
        *widget.imp().tags.borrow_mut() = Some(tags);
        widget.build_ui();
        widget.refresh();
        widget
    }

    fn build_ui(&self) {
        let imp = self.imp();
        imp.scroll.set_child(Some(&imp.content_box));
        imp.scroll.set_parent(self);
    }

    pub fn refresh(&self) {
        let imp = self.imp();
        while let Some(child) = imp.content_box.first_child() {
            imp.content_box.remove(&child);
        }

        let tags = imp
            .tags
            .borrow()
            .as_ref()
            .map(|db| db.all_tags())
            .unwrap_or_default();

        if tags.is_empty() {
            imp.content_box.set_valign(gtk4::Align::Center);
            imp.content_box.set_halign(gtk4::Align::Center);
            imp.content_box.set_vexpand(true);

            let label = gtk4::Label::new(Some(
                "No tags yet.\nSelect an image and press T to add tags.",
            ));
            label.set_justify(gtk4::Justification::Center);
            label.set_wrap(true);
            label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            label.add_css_class("dim-label");
            imp.content_box.append(&label);
            return;
        }

        imp.content_box.set_valign(gtk4::Align::Fill);
        imp.content_box.set_halign(gtk4::Align::Fill);
        imp.content_box.set_vexpand(false);

        let mut grouped: BTreeMap<char, Vec<(String, usize)>> = BTreeMap::new();
        for (tag, count) in tags {
            let first = tag
                .chars()
                .next()
                .map(|ch| ch.to_ascii_uppercase())
                .filter(|ch| ch.is_ascii_alphabetic())
                .unwrap_or('#');
            grouped.entry(first).or_default().push((tag, count));
        }

        for (letter, entries) in grouped {
            let heading = gtk4::Label::new(Some(&letter.to_string()));
            heading.add_css_class("caption-heading");
            heading.set_halign(gtk4::Align::Start);
            heading.set_margin_start(12);
            heading.set_margin_top(8);
            heading.set_margin_bottom(4);
            imp.content_box.append(&heading);

            let list = gtk4::ListBox::new();
            list.add_css_class("navigation-sidebar");
            list.set_selection_mode(gtk4::SelectionMode::None);

            for (tag, count) in entries {
                let row = gtk4::ListBoxRow::new();
                let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                hbox.set_margin_start(8);
                hbox.set_margin_end(8);
                hbox.set_margin_top(6);
                hbox.set_margin_bottom(6);

                let name_button = gtk4::Button::with_label(&tag);
                name_button.add_css_class("flat");
                name_button.set_hexpand(true);
                name_button.set_halign(gtk4::Align::Start);
                name_button.set_tooltip_text(Some(&format!("{count} image(s)")));

                let delete_button = gtk4::Button::from_icon_name("edit-delete-symbolic");
                delete_button.add_css_class("flat");
                delete_button.add_css_class("destructive-action");

                let widget_weak = self.downgrade();
                let tag_for_activate = tag.clone();
                name_button.connect_clicked(move |_| {
                    let Some(widget) = widget_weak.upgrade() else {
                        return;
                    };
                    widget.emit_tag_activated(&tag_for_activate);
                });

                let widget_weak = self.downgrade();
                let tag_for_delete = tag.clone();
                delete_button.connect_clicked(move |_| {
                    let Some(widget) = widget_weak.upgrade() else {
                        return;
                    };
                    let Some(db) = widget.imp().tags.borrow().as_ref().cloned() else {
                        return;
                    };
                    db.delete_tag_globally(&tag_for_delete);
                    widget.refresh();
                });

                hbox.append(&name_button);
                hbox.append(&delete_button);
                row.set_child(Some(&hbox));
                list.append(&row);
            }

            imp.content_box.append(&list);
        }
    }

    pub fn connect_tag_activated<F: Fn(&str) + 'static>(&self, f: F) {
        *self.imp().tag_activated_cb.borrow_mut() = Some(Box::new(f));
    }

    fn emit_tag_activated(&self, tag: &str) {
        if let Some(cb) = self.imp().tag_activated_cb.borrow().as_ref() {
            cb(tag);
        }
    }
}
