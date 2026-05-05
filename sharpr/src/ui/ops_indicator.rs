//! Header-bar tasks entry point.
//!
//! The button stays visible in the main header, routes users toward the Tasks
//! page, and reflects whether any background work or user pipeline is actively
//! running.

use std::sync::Once;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

const IDLE_ICON_CANDIDATES: &[&str] = &[
    "circle-outline-thick-symbolic",
    "circle-outline-symbolic",
    "emblem-system-symbolic",
];
const BUSY_ICON_CANDIDATES: &[&str] = &[
    "spinner-symbolic",
    "process-working-symbolic",
    "emblem-synchronizing-symbolic",
];

mod imp {
    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;

    use gtk4::glib;
    use gtk4::prelude::*;
    use gtk4::subclass::prelude::*;

    #[derive(Default)]
    pub struct OpsIndicator {
        pub(super) button: RefCell<Option<gtk4::Button>>,
        pub(super) icon: RefCell<Option<gtk4::Image>>,
        pub(super) idle_icon_name: RefCell<Option<String>>,
        pub(super) busy_icon_name: RefCell<Option<String>>,
        pub(super) active_ops: RefCell<HashSet<u64>>,
        pub(super) pipeline_active: Cell<bool>,
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
            super::install_css();

            let idle_icon = super::resolve_icon_name(super::IDLE_ICON_CANDIDATES);
            let busy_icon = super::resolve_icon_name(super::BUSY_ICON_CANDIDATES);

            let icon = gtk4::Image::from_icon_name(&idle_icon);
            icon.set_pixel_size(16);

            let button = gtk4::Button::new();
            button.set_child(Some(&icon));
            button.add_css_class("flat");
            button.add_css_class("ops-indicator-button");
            button.set_tooltip_text(Some("Tasks"));
            button.set_visible(true);
            button.set_parent(&*widget);

            *self.button.borrow_mut() = Some(button);
            *self.icon.borrow_mut() = Some(icon);
            *self.idle_icon_name.borrow_mut() = Some(idle_icon);
            *self.busy_icon_name.borrow_mut() = Some(busy_icon);
        }

        fn dispose(&self) {
            if let Some(btn) = self.button.borrow().as_ref() {
                btn.unparent();
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

    pub fn set_go_to_tasks_cb<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
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

    pub fn set_pipeline_active(&self, active: bool) {
        self.imp().pipeline_active.set(active);
        self.refresh_icon();
    }

    fn refresh_icon(&self) {
        let imp = self.imp();
        let busy = !imp.active_ops.borrow().is_empty() || imp.pipeline_active.get();
        let idle_icon = imp.idle_icon_name.borrow();
        let busy_icon = imp.busy_icon_name.borrow();

        if let Some(icon) = imp.icon.borrow().as_ref() {
            if busy {
                if let Some(name) = busy_icon.as_deref() {
                    icon.set_icon_name(Some(name));
                }
                icon.add_css_class("ops-indicator-busy");
            } else {
                if let Some(name) = idle_icon.as_deref() {
                    icon.set_icon_name(Some(name));
                }
                icon.remove_css_class("ops-indicator-busy");
            }
        }
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

fn install_css() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(
            "
            @keyframes ops-indicator-spin {
                from { -gtk-icon-transform: rotate(0deg); }
                to { -gtk-icon-transform: rotate(1turn); }
            }

            .ops-indicator-busy {
                animation: ops-indicator-spin 1s linear infinite;
            }
            ",
        );
        if let Some(display) = gdk4::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
}

impl Default for OpsIndicator {
    fn default() -> Self {
        Self::new()
    }
}
