//! Bottom-left background-operations indicator.
//!
//! Collapsed: a floating pill button (GtkButton, "osd" CSS class) with a
//! spinner and summary label.  Expanded: a GtkPopover listing each op with
//! its own GtkProgressBar or status text.

use std::rc::Rc;
use std::sync::Once;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

// ---------------------------------------------------------------------------
// Per-row data kept in the HashMap
// ---------------------------------------------------------------------------

pub type ActionCallback = Rc<std::cell::RefCell<Option<Rc<dyn Fn()>>>>;

struct OpRowWidgets {
    row: gtk4::ListBoxRow,
    progress_bar: gtk4::ProgressBar,
    status_label: gtk4::Label,
    action_button: gtk4::Button,
    action: ActionCallback,
    persistent: std::cell::Cell<bool>,
}

// ---------------------------------------------------------------------------
// imp
// ---------------------------------------------------------------------------

mod imp {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use gtk4::glib;
    use gtk4::prelude::*;
    use gtk4::subclass::prelude::*;

    use super::OpRowWidgets;

    #[derive(Default)]
    pub struct OpsIndicator {
        // The single child widget owned by this widget (BinLayout).
        pub(super) button: RefCell<Option<gtk4::Button>>,
        pub(super) spinner: RefCell<Option<gtk4::Spinner>>,
        pub(super) summary_label: RefCell<Option<gtk4::Label>>,
        pub(super) popover: RefCell<Option<gtk4::Popover>>,
        pub(super) list_box: RefCell<Option<gtk4::ListBox>>,
        pub(super) clear_btn: RefCell<Option<gtk4::Button>>,
        pub(super) rows: RefCell<HashMap<u64, OpRowWidgets>>,
        pub(super) active_count: RefCell<u32>,
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

            // ---- Build the summary button (collapsed indicator) ----
            let spinner = gtk4::Spinner::new();
            spinner.set_size_request(16, 16);
            let summary_label = gtk4::Label::new(Some("Operations in progress…"));
            summary_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            summary_label.set_xalign(0.5);
            summary_label.set_hexpand(true);
            summary_label.set_halign(gtk4::Align::Fill);
            summary_label.add_css_class("ops-indicator-summary");
            let end_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            end_spacer.set_size_request(16, 16);

            let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            hbox.set_spacing(8);
            hbox.set_margin_start(12);
            hbox.set_margin_end(12);
            hbox.set_margin_top(6);
            hbox.set_margin_bottom(6);
            hbox.set_hexpand(true);
            hbox.append(&spinner);
            hbox.append(&summary_label);
            hbox.append(&end_spacer);

            let button = gtk4::Button::new();
            button.set_child(Some(&hbox));
            button.add_css_class("flat");
            button.add_css_class("ops-indicator-pill");
            button.set_width_request(220);
            button.set_halign(gtk4::Align::Fill);
            button.set_hexpand(true);
            button.set_visible(false); // hidden until first op

            button.set_parent(&*widget);

            // ---- Build the popover ----
            let heading = gtk4::Label::new(Some("Background Operations"));
            heading.add_css_class("heading");
            heading.set_halign(gtk4::Align::Start);
            heading.set_margin_bottom(6);

            let list_box = gtk4::ListBox::new();
            list_box.set_selection_mode(gtk4::SelectionMode::None);
            list_box.add_css_class("boxed-list");

            let scrolled = gtk4::ScrolledWindow::new();
            scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
            scrolled.set_max_content_height(300);
            scrolled.set_propagate_natural_height(true);
            scrolled.set_child(Some(&list_box));

            let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);

            let clear_btn = gtk4::Button::with_label("Clear completed");
            clear_btn.set_halign(gtk4::Align::Center);

            let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            vbox.set_margin_top(12);
            vbox.set_margin_bottom(12);
            vbox.set_margin_start(12);
            vbox.set_margin_end(12);
            vbox.set_spacing(6);
            vbox.append(&heading);
            vbox.append(&scrolled);
            vbox.append(&sep);
            vbox.append(&clear_btn);

            let popover = gtk4::Popover::new();
            popover.set_child(Some(&vbox));
            popover.set_has_arrow(false);
            popover.set_position(gtk4::PositionType::Right);
            popover.add_css_class("background");
            popover.set_parent(&button);

            // Toggle popover on button click
            {
                let popover_c = popover.clone();
                button.connect_clicked(move |_| {
                    if popover_c.is_visible() {
                        popover_c.popdown();
                    } else {
                        popover_c.popup();
                    }
                });
            }

            // "Clear completed" removes rows that are done/failed
            {
                let list_box_c = list_box.clone();
                let clear_btn_obj = self.obj().clone();
                clear_btn.connect_clicked(move |_| {
                    clear_btn_obj.imp().clear_completed(&list_box_c);
                });
            }

            *self.button.borrow_mut() = Some(button);
            *self.spinner.borrow_mut() = Some(spinner);
            *self.summary_label.borrow_mut() = Some(summary_label);
            *self.popover.borrow_mut() = Some(popover);
            *self.list_box.borrow_mut() = Some(list_box);
            *self.clear_btn.borrow_mut() = Some(clear_btn);
        }

        fn dispose(&self) {
            if let Some(btn) = self.button.borrow().as_ref() {
                btn.unparent();
            }
        }
    }

    impl WidgetImpl for OpsIndicator {}

    impl OpsIndicator {
        pub fn clear_completed(&self, list_box: &gtk4::ListBox) {
            let mut rows = self.rows.borrow_mut();
            let done_ids: Vec<u64> = rows
                .iter()
                .filter(|(_, w)| !w.progress_bar.is_visible())
                .map(|(id, _)| *id)
                .collect();
            for id in done_ids {
                if let Some(w) = rows.remove(&id) {
                    list_box.remove(&w.row);
                }
            }
            // Update button visibility
            let button = self.button.borrow();
            if let Some(btn) = button.as_ref() {
                btn.set_visible(!rows.is_empty() || *self.active_count.borrow() > 0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public wrapper
// ---------------------------------------------------------------------------

glib::wrapper! {
    pub struct OpsIndicator(ObjectSubclass<imp::OpsIndicator>)
        @extends gtk4::Widget;
}

impl OpsIndicator {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Register a new operation — called when `OpEvent::Added` is received.
    pub fn push_op(&self, id: u64, title: &str) {
        let imp = self.imp();

        // Build the row
        let title_label = gtk4::Label::new(Some(title));
        title_label.set_halign(gtk4::Align::Start);
        title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        title_label.set_max_width_chars(35);

        let progress_bar = gtk4::ProgressBar::new();
        progress_bar.set_pulse_step(0.1);
        progress_bar.pulse(); // start indeterminate

        // Status label (shown instead of progress bar when done/failed)
        let status_label = gtk4::Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_visible(false);

        let action_button = gtk4::Button::with_label("Open");
        action_button.add_css_class("flat");
        action_button.add_css_class("accent");
        action_button.set_halign(gtk4::Align::Start);
        action_button.set_visible(false);

        let row_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        row_box.set_margin_top(8);
        row_box.set_margin_bottom(8);
        row_box.set_margin_start(12);
        row_box.set_margin_end(12);
        row_box.append(&title_label);
        row_box.append(&progress_bar);
        row_box.append(&status_label);
        row_box.append(&action_button);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&row_box));
        row.set_activatable(false);

        if let Some(lb) = imp.list_box.borrow().as_ref() {
            lb.append(&row);
        }

        let action: ActionCallback = Rc::new(std::cell::RefCell::new(None));
        {
            let action = action.clone();
            action_button.connect_clicked(move |_| {
                if let Some(cb) = action.borrow().as_ref() {
                    cb();
                }
            });
        }

        imp.rows.borrow_mut().insert(
            id,
            OpRowWidgets {
                row,
                progress_bar,
                status_label,
                action_button,
                action,
                persistent: std::cell::Cell::new(false),
            },
        );

        *imp.active_count.borrow_mut() += 1;
        self.refresh_summary();
    }

    /// Update progress — called when `OpEvent::Progress` is received.
    pub fn update_op(&self, id: u64, fraction: Option<f32>) {
        let rows = self.imp().rows.borrow();
        if let Some(w) = rows.get(&id) {
            match fraction {
                Some(f) => w.progress_bar.set_fraction(f as f64),
                None => w.progress_bar.pulse(),
            }
        }
    }

    /// Mark an operation as complete — called when `OpEvent::Completed` is received.
    pub fn complete_op(&self, id: u64) {
        let imp = self.imp();
        let rows = imp.rows.borrow();
        if let Some(w) = rows.get(&id) {
            w.progress_bar.set_visible(false);
            w.status_label.set_text("✓ Done");
            w.status_label.add_css_class("dim-label");
            w.status_label.set_visible(true);
        }
        drop(rows);
        let count = imp.active_count.borrow().saturating_sub(1);
        *imp.active_count.borrow_mut() = count;
        self.refresh_summary();
        {
            let widget = self.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                if !widget.op_is_persistent(id) {
                    widget.remove_op(id);
                }
            });
        }
    }

    /// Mark an operation as failed — called when `OpEvent::Failed` is received.
    pub fn fail_op(&self, id: u64, msg: &str) {
        let imp = self.imp();
        let rows = imp.rows.borrow();
        if let Some(w) = rows.get(&id) {
            w.progress_bar.set_visible(false);
            w.status_label.set_text(&format!("✗ {}", msg));
            w.status_label.add_css_class("error");
            w.status_label.set_visible(true);
        }
        drop(rows);
        let count = imp.active_count.borrow().saturating_sub(1);
        *imp.active_count.borrow_mut() = count;
        self.refresh_summary();
        {
            let widget = self.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                if !widget.op_is_persistent(id) {
                    widget.remove_op(id);
                }
            });
        }
    }

    /// Remove an op row entirely — called when `OpEvent::Dismissed` is received.
    pub fn remove_op(&self, id: u64) {
        let imp = self.imp();
        let removed = imp.rows.borrow_mut().remove(&id);
        if let Some(w) = removed {
            if let Some(lb) = imp.list_box.borrow().as_ref() {
                lb.remove(&w.row);
            }
        }
        self.refresh_summary();
    }

    // ---- Private helpers ----

    fn refresh_summary(&self) {
        let imp = self.imp();
        let active = *imp.active_count.borrow();
        let total_rows = imp.rows.borrow().len();

        let button = imp.button.borrow();
        let Some(btn) = button.as_ref() else { return };

        if total_rows == 0 {
            if let Some(sp) = imp.spinner.borrow().as_ref() {
                sp.stop();
            }
            let btn_clone = btn.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                btn_clone.set_visible(false);
            });
            return;
        }

        btn.set_visible(true);

        if let Some(sp) = imp.spinner.borrow().as_ref() {
            if active > 0 {
                sp.start();
            } else {
                sp.stop();
            }
        }

        if let Some(lbl) = imp.summary_label.borrow().as_ref() {
            if active == 0 {
                lbl.set_text("All operations complete");
            } else if active == 1 {
                lbl.set_text("1 operation running");
            } else {
                lbl.set_text(&format!("{} operations running", active));
            }
        }
    }

    fn op_is_persistent(&self, id: u64) -> bool {
        self.imp()
            .rows
            .borrow()
            .get(&id)
            .map(|row| row.persistent.get())
            .unwrap_or(false)
    }

    pub fn set_op_action<F>(&self, id: u64, label: &str, action: F)
    where
        F: Fn() + 'static,
    {
        let rows = self.imp().rows.borrow();
        if let Some(row) = rows.get(&id) {
            row.action_button.set_label(label);
            row.action_button.set_visible(true);
            row.persistent.set(true);
            *row.action.borrow_mut() = Some(Rc::new(action));
        }
    }

    pub fn clear_op_action(&self, id: u64) {
        let rows = self.imp().rows.borrow();
        if let Some(row) = rows.get(&id) {
            row.action_button.set_visible(false);
            row.persistent.set(false);
            row.action.borrow_mut().take();
        }
    }
}

fn install_css() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(
            "
            .ops-indicator-pill {
                min-height: 0;
                min-width: 0;
                padding: 0;
                border-radius: 999px;
                background-color: rgba(28, 28, 30, 0.72);
                color: white;
                box-shadow: 0 6px 18px rgba(0, 0, 0, 0.18);
            }
            .ops-indicator-pill:hover {
                background-color: rgba(40, 40, 43, 0.82);
            }
            .ops-indicator-pill:active {
                background-color: rgba(52, 52, 56, 0.88);
            }
            .ops-indicator-summary {
                font-weight: 600;
                font-size: 0.92em;
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
