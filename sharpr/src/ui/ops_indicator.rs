//! Header-bar indicator for transient background operations.

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

const BUSY_ICON_CANDIDATES: &[&str] = &[
    "spinner-symbolic",
    "process-working-symbolic",
    "emblem-synchronizing-symbolic",
];

mod imp {
    use std::cell::RefCell;
    use std::collections::HashSet;

    use gtk4::glib;
    use gtk4::prelude::*;
    use gtk4::subclass::prelude::*;

    #[derive(Default)]
    pub struct OpsIndicator {
        pub(super) icon: RefCell<Option<gtk4::Image>>,
        pub(super) revealer: RefCell<Option<gtk4::Revealer>>,
        pub(super) active_ops: RefCell<HashSet<u64>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OpsIndicator {
        const NAME: &'static str = "SharprOpsIndicator";
        type Type = super::OpsIndicator;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for OpsIndicator {
        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();

            let busy_icon = super::resolve_icon_name(super::BUSY_ICON_CANDIDATES);
            let icon = gtk4::Image::from_icon_name(&busy_icon);
            icon.set_pixel_size(16);
            icon.add_css_class("ops-indicator-busy");
            icon.set_tooltip_text(Some("Working in the background"));

            // Hidden while idle; slides in only while work runs.
            let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            content.append(&icon);
            let revealer = gtk4::Revealer::new();
            revealer.set_transition_type(gtk4::RevealerTransitionType::SlideLeft);
            revealer.set_transition_duration(200);
            revealer.set_child(Some(&content));
            revealer.set_parent(&*widget);
            *self.icon.borrow_mut() = Some(icon);
            *self.revealer.borrow_mut() = Some(revealer);
        }

        fn dispose(&self) {
            if let Some(revealer) = self.revealer.borrow().as_ref() {
                revealer.unparent();
            }
        }
    }

    impl WidgetImpl for OpsIndicator {}
}

glib::wrapper! {
    pub struct OpsIndicator(ObjectSubclass<imp::OpsIndicator>)
        @extends gtk4::Widget,
                 @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl OpsIndicator {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn push_op(&self, id: u64, _title: &str) {
        self.imp().active_ops.borrow_mut().insert(id);
        self.refresh_icon();
    }

    pub fn update_op(&self, _id: u64, _fraction: Option<f32>) {}

    pub fn complete_op(&self, id: u64) {
        self.imp().active_ops.borrow_mut().remove(&id);
        self.refresh_icon();
    }

    pub fn fail_op(&self, id: u64, _msg: &str) {
        self.imp().active_ops.borrow_mut().remove(&id);
        self.refresh_icon();
    }

    pub fn remove_op(&self, id: u64) {
        self.imp().active_ops.borrow_mut().remove(&id);
        self.refresh_icon();
    }

    /// Shows the indicator only for work that lasts longer than a moment, so
    /// quick operations do not make the header flicker.
    fn refresh_icon(&self) {
        let ops = !self.imp().active_ops.borrow().is_empty();
        if let Some(icon) = self.imp().icon.borrow().as_ref() {
            icon.set_visible(ops);
        }
        let busy = ops;
        if !busy {
            if let Some(revealer) = self.imp().revealer.borrow().as_ref() {
                revealer.set_reveal_child(false);
            }
            return;
        }
        let weak = self.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            let Some(indicator) = weak.upgrade() else {
                return;
            };
            let still_busy = !indicator.imp().active_ops.borrow().is_empty();
            let revealer = indicator.imp().revealer.borrow().clone();
            if let Some(revealer) = revealer {
                revealer.set_reveal_child(still_busy);
            }
        });
    }
}

fn resolve_icon_name(candidates: &[&str]) -> String {
    if let Some(display) = gdk4::Display::default() {
        let theme = gtk4::IconTheme::for_display(&display);
        for candidate in candidates {
            if theme.has_icon(candidate) {
                return (*candidate).to_string();
            }
        }
    }
    candidates
        .first()
        .copied()
        .unwrap_or("emblem-system-symbolic")
        .to_string()
}

impl Default for OpsIndicator {
    fn default() -> Self {
        Self::new()
    }
}
