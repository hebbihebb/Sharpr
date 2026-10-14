//! Header-bar indicator for transient background operations.

use std::sync::Once;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

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
        pub(super) icon: RefCell<Option<gtk4::Image>>,
        pub(super) revealer: RefCell<Option<gtk4::Revealer>>,
        pub(super) active_ops: RefCell<HashSet<u64>>,
        /// A progress ring for visual-search indexing.
        pub(super) ring: RefCell<Option<gtk4::DrawingArea>>,
        pub(super) indexing: Cell<Option<f64>>,
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

            let busy_icon = super::resolve_icon_name(super::BUSY_ICON_CANDIDATES);
            let icon = gtk4::Image::from_icon_name(&busy_icon);
            icon.set_pixel_size(16);
            icon.add_css_class("ops-indicator-busy");
            icon.set_tooltip_text(Some("Working in the background"));

            let ring = gtk4::DrawingArea::new();
            ring.set_content_width(16);
            ring.set_content_height(16);
            ring.set_valign(gtk4::Align::Center);
            ring.set_visible(false);
            {
                let indicator = widget.downgrade();
                ring.set_draw_func(move |area, cr, width, height| {
                    let Some(indicator) = indicator.upgrade() else {
                        return;
                    };
                    let fraction = indicator.imp().indexing.get().unwrap_or(0.0);
                    super::draw_ring(area, cr, width, height, fraction);
                });
            }

            // Hidden while idle; slides in only while work runs.
            let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            content.append(&ring);
            content.append(&icon);
            let revealer = gtk4::Revealer::new();
            revealer.set_transition_type(gtk4::RevealerTransitionType::SlideLeft);
            revealer.set_transition_duration(200);
            revealer.set_child(Some(&content));
            revealer.set_parent(&*widget);
            *self.icon.borrow_mut() = Some(icon);
            *self.ring.borrow_mut() = Some(ring);
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

    /// Shows a progress ring with `tooltip` while visual-search indexing
    /// runs (`fraction` done); `None` hides it.
    pub fn set_indexing(&self, progress: Option<(f64, &str)>) {
        let imp = self.imp();
        let fraction = progress.map(|(fraction, _)| fraction.clamp(0.0, 1.0));
        let tooltip = progress.map(|(_, tooltip)| tooltip.to_string());
        let shown = imp
            .ring
            .borrow()
            .as_ref()
            .and_then(|ring| ring.tooltip_text())
            .map(|t| t.to_string());
        // Called every second: change the widgets only when something changed.
        if fraction == imp.indexing.get() && tooltip == shown {
            return;
        }
        imp.indexing.set(fraction);
        if let Some(ring) = imp.ring.borrow().as_ref() {
            ring.set_visible(progress.is_some());
            ring.set_tooltip_text(progress.map(|(_, tooltip)| tooltip));
            ring.update_property(&[gtk4::accessible::Property::Label(
                progress.map(|(_, tooltip)| tooltip).unwrap_or_default(),
            )]);
            ring.queue_draw();
        }
        self.refresh_icon();
    }

    /// Shows the indicator only for work that lasts longer than a moment, so
    /// quick operations do not make the header flicker.
    fn refresh_icon(&self) {
        let ops = !self.imp().active_ops.borrow().is_empty();
        if let Some(icon) = self.imp().icon.borrow().as_ref() {
            icon.set_visible(ops);
        }
        let busy = ops || self.imp().indexing.get().is_some();
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
            let still_busy = !indicator.imp().active_ops.borrow().is_empty()
                || indicator.imp().indexing.get().is_some();
            let revealer = indicator.imp().revealer.borrow().clone();
            if let Some(revealer) = revealer {
                revealer.set_reveal_child(still_busy);
            }
        });
    }
}

/// A faint full circle with the done part drawn over it in the text colour.
fn draw_ring(
    area: &gtk4::DrawingArea,
    cr: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    fraction: f64,
) {
    let color = area.color();
    let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
    let radius = cx.min(cy) - 2.0;
    cr.set_line_width(2.5);
    cr.set_source_rgba(
        f64::from(color.red()),
        f64::from(color.green()),
        f64::from(color.blue()),
        0.25,
    );
    cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
    if fraction > 0.0 {
        let start = -std::f64::consts::FRAC_PI_2;
        cr.set_source_rgba(
            f64::from(color.red()),
            f64::from(color.green()),
            f64::from(color.blue()),
            f64::from(color.alpha()),
        );
        cr.arc(
            cx,
            cy,
            radius,
            start,
            start + fraction * std::f64::consts::TAU,
        );
        let _ = cr.stroke();
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
