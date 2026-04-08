use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use gdk4::Paintable;
use glib::prelude::*;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::model::ImageEntry;
use crate::thumbnails::worker::WorkerRequest;
use crate::ui::window::AppState;

type ImageSelectedCallback = Box<dyn Fn(u32) + 'static>;
type SearchChangedCallback = Box<dyn Fn(&str) + 'static>;
type SearchActivateCallback = Box<dyn Fn(&str) + 'static>;
type SearchDismissedCallback = Box<dyn Fn() + 'static>;

const ESTIMATED_ROW_HEIGHT: f64 = 220.0;
const BUFFER_ROWS: u32 = 2000;
const FALLBACK_VISIBLE_ROWS: u32 = 40;

mod imp {
    use super::*;
    use async_channel::Sender;

    pub struct FilmstripPane {
        pub toolbar_view: libadwaita::ToolbarView,
        pub root_box: gtk4::Box,
        pub search_bar: gtk4::SearchBar,
        pub search_entry: gtk4::SearchEntry,
        pub suggestions_popover: gtk4::Popover,
        pub suggestions_list: gtk4::ListBox,
        pub scroll: gtk4::ScrolledWindow,
        pub list_view: gtk4::ListView,
        pub selection_model: gtk4::SingleSelection,
        pub image_selected_cb: RefCell<Option<ImageSelectedCallback>>,
        pub search_changed_cb: RefCell<Option<SearchChangedCallback>>,
        pub search_activate_cb: RefCell<Option<SearchActivateCallback>>,
        pub search_dismissed_cb: RefCell<Option<SearchDismissedCallback>>,
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
        pub visible_thumbnail_tx: RefCell<Option<Sender<WorkerRequest>>>,
        pub preload_thumbnail_tx: RefCell<Option<Sender<WorkerRequest>>>,
        pub thumbnail_gen: RefCell<Option<Arc<std::sync::atomic::AtomicU64>>>,
        pub pending_thumbnails: RefCell<Option<Arc<Mutex<std::collections::HashSet<PathBuf>>>>>,
        pub pending_notify_count: std::sync::atomic::AtomicU32,
    }

    impl Default for FilmstripPane {
        fn default() -> Self {
            let selection_model = gtk4::SingleSelection::new(None::<gio::ListStore>);
            selection_model.set_can_unselect(false);

            let list_view =
                gtk4::ListView::new(Some(selection_model.clone()), None::<gtk4::ListItemFactory>);
            list_view.set_orientation(gtk4::Orientation::Vertical);
            list_view.add_css_class("navigation-sidebar");

            let search_bar = gtk4::SearchBar::new();
            let search_entry = gtk4::SearchEntry::new();
            let suggestions_list = gtk4::ListBox::new();
            let suggestions_popover = gtk4::Popover::new();

            Self {
                toolbar_view: libadwaita::ToolbarView::new(),
                root_box: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
                search_bar,
                search_entry,
                suggestions_popover,
                suggestions_list,
                scroll: gtk4::ScrolledWindow::new(),
                list_view,
                selection_model,
                image_selected_cb: RefCell::new(None),
                search_changed_cb: RefCell::new(None),
                search_activate_cb: RefCell::new(None),
                search_dismissed_cb: RefCell::new(None),
                state: RefCell::new(None),
                visible_thumbnail_tx: RefCell::new(None),
                preload_thumbnail_tx: RefCell::new(None),
                thumbnail_gen: RefCell::new(None),
                pending_thumbnails: RefCell::new(None),
                pending_notify_count: std::sync::atomic::AtomicU32::new(0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FilmstripPane {
        const NAME: &'static str = "SharprFilmstripPane";
        type Type = super::FilmstripPane;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for FilmstripPane {
        fn dispose(&self) {
            self.toolbar_view.unparent();
        }
    }

    impl WidgetImpl for FilmstripPane {}
}

glib::wrapper! {
    pub struct FilmstripPane(ObjectSubclass<imp::FilmstripPane>)
        @extends gtk4::Widget;
}

impl FilmstripPane {
    pub fn new(state: Rc<RefCell<AppState>>) -> Self {
        let widget: Self = glib::Object::new();
        *widget.imp().state.borrow_mut() = Some(state);
        widget.build_ui();
        widget
    }

    fn build_ui(&self) {
        let imp = self.imp();

        let header = libadwaita::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        imp.toolbar_view.add_top_bar(&header);

        self.install_css();

        imp.search_entry.set_placeholder_text(Some("Search tags…"));
        imp.search_bar.set_child(Some(&imp.search_entry));
        imp.search_bar.set_show_close_button(true);
        imp.search_bar.connect_entry(&imp.search_entry);

        imp.suggestions_list.add_css_class("boxed-list");
        imp.suggestions_list
            .set_selection_mode(gtk4::SelectionMode::None);
        imp.suggestions_popover.set_has_arrow(false);
        imp.suggestions_popover.set_autohide(true);
        imp.suggestions_popover
            .set_position(gtk4::PositionType::Bottom);
        imp.suggestions_popover
            .set_child(Some(&imp.suggestions_list));
        imp.suggestions_popover.set_parent(&imp.search_entry);

        imp.root_box.append(&imp.search_bar);

        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, obj| {
            let list_item = obj
                .downcast_ref::<gtk4::ListItem>()
                .expect("factory object must be ListItem");

            let item_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            item_box.set_margin_top(4);
            item_box.set_margin_bottom(4);
            item_box.set_margin_start(4);
            item_box.set_margin_end(4);

            let overlay = gtk4::Overlay::new();
            let picture = gtk4::Picture::new();
            picture.set_content_fit(gtk4::ContentFit::Cover);
            picture.set_size_request(-1, 160);
            picture.add_css_class("thumbnail-frame");

            let index_label = gtk4::Label::new(None);
            index_label.set_halign(gtk4::Align::Start);
            index_label.set_valign(gtk4::Align::End);
            index_label.add_css_class("filmstrip-index-label");

            overlay.set_child(Some(&picture));
            overlay.add_overlay(&index_label);

            let filename_label = gtk4::Label::new(None);
            filename_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            filename_label.set_max_width_chars(18);
            filename_label.add_css_class("caption");

            // Append content children before parenting the popover — in GTK4,
            // set_parent() inserts at the front of the child list so doing it
            // first would make the popover the first_child(), breaking traversal.
            item_box.append(&overlay);
            item_box.append(&filename_label);

            let popover = gtk4::Popover::new();
            popover.set_has_arrow(true);
            popover.set_autohide(true);
            popover.set_position(gtk4::PositionType::Bottom);
            popover.set_parent(&item_box);

            // Store widget refs by key so bind/unbind don't rely on child ordering.
            unsafe {
                list_item.set_data("fs-picture", picture.clone());
                list_item.set_data("fs-index-label", index_label.clone());
                list_item.set_data("fs-filename-label", filename_label.clone());
            }

            let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            popover_box.set_margin_top(6);
            popover_box.set_margin_bottom(6);
            popover_box.set_margin_start(6);
            popover_box.set_margin_end(6);

            let open_button = gtk4::Button::with_label("Open in Default Viewer");
            let reveal_button = gtk4::Button::with_label("Show in File Manager");
            open_button.set_halign(gtk4::Align::Fill);
            reveal_button.set_halign(gtk4::Align::Fill);
            popover_box.append(&open_button);
            popover_box.append(&reveal_button);
            popover.set_child(Some(&popover_box));

            let gesture_right = gtk4::GestureClick::new();
            gesture_right.set_button(3);
            item_box.add_controller(gesture_right.clone());

            // Double-click is handled by ListView::activate at the pane level.

            let list_item_weak = list_item.downgrade();
            open_button.connect_clicked(move |_| {
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                if let Some(path) = list_item_path(&list_item) {
                    launch_default_for_path(&path);
                }
            });

            let list_item_weak = list_item.downgrade();
            reveal_button.connect_clicked(move |_| {
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                if let Some(path) = list_item_path(&list_item) {
                    reveal_in_file_manager(&path);
                }
            });

            let list_item_weak = list_item.downgrade();
            let popover_weak = popover.downgrade();
            gesture_right.connect_released(move |_, _, _, _| {
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                if list_item.item().is_none() {
                    return;
                }
                if let Some(popover) = popover_weak.upgrade() {
                    popover.popup();
                }
            });

            list_item.set_child(Some(&item_box));
        });

        factory.connect_bind(move |_, obj| {
            let list_item = obj
                .downcast_ref::<gtk4::ListItem>()
                .expect("factory object must be ListItem");

            let entry: ImageEntry = match list_item.item().and_downcast::<ImageEntry>() {
                Some(entry) => entry,
                None => return,
            };
            let (picture, index_label, filename_label) = unsafe {
                let picture = list_item
                    .data::<gtk4::Picture>("fs-picture")
                    .map(|p| p.as_ref().clone());
                let index_label = list_item
                    .data::<gtk4::Label>("fs-index-label")
                    .map(|p| p.as_ref().clone());
                let filename_label = list_item
                    .data::<gtk4::Label>("fs-filename-label")
                    .map(|p| p.as_ref().clone());
                match (picture, index_label, filename_label) {
                    (Some(p), Some(i), Some(f)) => (p, i, f),
                    _ => return,
                }
            };

            filename_label.set_text(&entry.filename());
            index_label.set_text(&(list_item.position() + 1).to_string());

            match entry.thumbnail() {
                Some(ref texture) => picture.set_paintable(Some(texture.upcast_ref::<Paintable>())),
                None => picture.set_paintable(None::<&Paintable>),
            }

            let picture_weak = picture.downgrade();
            let entry_clone = entry.clone();
            let handler_id = entry.connect_notify_local(Some("thumbnail"), move |_, _| {
                if let Some(picture) = picture_weak.upgrade() {
                    match entry_clone.thumbnail() {
                        Some(ref texture) => {
                            picture.set_paintable(Some(texture.upcast_ref::<Paintable>()))
                        }
                        None => picture.set_paintable(None::<&Paintable>),
                    }
                }
            });

            unsafe {
                list_item.set_data("thumbnail-notify-id", handler_id);
            }
        });

        factory.connect_unbind(|_, obj| {
            let list_item = obj
                .downcast_ref::<gtk4::ListItem>()
                .expect("factory object must be ListItem");
            if let Some(entry) = list_item.item().and_downcast::<ImageEntry>() {
                let handler_id =
                    unsafe { list_item.steal_data::<glib::SignalHandlerId>("thumbnail-notify-id") };
                if let Some(handler_id) = handler_id {
                    entry.disconnect(handler_id);
                }
            }
            unsafe {
                if let Some(picture) = list_item
                    .data::<gtk4::Picture>("fs-picture")
                    .map(|p| p.as_ref().clone())
                {
                    picture.set_paintable(None::<&Paintable>);
                }
                if let Some(label) = list_item
                    .data::<gtk4::Label>("fs-index-label")
                    .map(|p| p.as_ref().clone())
                {
                    label.set_text("");
                }
                if let Some(label) = list_item
                    .data::<gtk4::Label>("fs-filename-label")
                    .map(|p| p.as_ref().clone())
                {
                    label.set_text("");
                }
            }
        });

        imp.list_view.set_factory(Some(&factory));
        // ListView::activate fires on double-click or Enter — use it for open-in-viewer.
        // Single-click still changes selection via SingleSelection; this is double-click only.
        imp.list_view.set_single_click_activate(false);

        imp.scroll
            .set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        imp.scroll.set_vexpand(true);
        imp.scroll.set_hexpand(true);
        imp.scroll.set_child(Some(&imp.list_view));

        imp.root_box.append(&imp.scroll);
        imp.toolbar_view.set_content(Some(&imp.root_box));
        imp.toolbar_view.set_parent(self);

        // Throttle scroll-driven scheduling: fire at most once per 80 ms to
        // avoid saturating the main thread during fast scrolls.
        let scroll_pending = std::rc::Rc::new(std::cell::Cell::new(false));
        let widget_weak = self.downgrade();
        imp.scroll.vadjustment().connect_value_changed(move |_| {
            if scroll_pending.get() {
                return;
            }
            scroll_pending.set(true);
            let pending_c = scroll_pending.clone();
            let widget_weak_c = widget_weak.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
                pending_c.set(false);
                if let Some(widget) = widget_weak_c.upgrade() {
                    widget.schedule_visible_thumbnails();
                }
                glib::ControlFlow::Break
            });
        });

        let widget_weak = self.downgrade();
        self.connect_map(move |_| {
            if let Some(widget) = widget_weak.upgrade() {
                widget.schedule_visible_thumbnails();
            }
        });

        let widget_weak = self.downgrade();
        imp.selection_model
            .connect_selected_notify(move |selection| {
                let index = selection.selected();
                if index == gtk4::INVALID_LIST_POSITION {
                    return;
                }
                if let Some(widget) = widget_weak.upgrade() {
                    widget.emit_image_selected(index);
                }
            });

        let widget_weak = self.downgrade();
        imp.search_entry.connect_search_changed(move |entry| {
            if let Some(widget) = widget_weak.upgrade() {
                widget.emit_search_changed(entry.text().as_ref());
            }
        });

        let widget_weak = self.downgrade();
        imp.search_entry.connect_activate(move |entry| {
            if let Some(widget) = widget_weak.upgrade() {
                widget.emit_search_activate(entry.text().as_ref());
            }
        });

        let widget_weak = self.downgrade();
        imp.search_bar
            .connect_search_mode_enabled_notify(move |search_bar| {
                if !search_bar.is_search_mode() {
                    if let Some(widget) = widget_weak.upgrade() {
                        widget.show_autocomplete(vec![]);
                        widget.emit_search_dismissed();
                    }
                }
            });

        let search_entry = imp.search_entry.clone();
        imp.suggestions_list.connect_row_activated(move |_, row| {
            let Some(child) = row.child() else { return };
            let Ok(label) = child.downcast::<gtk4::Label>() else {
                return;
            };
            search_entry.set_text(label.text().as_ref());
            search_entry.activate();
        });
    }

    fn install_css(&self) {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(
            "
            .filmstrip-index-label {
                font-size: 10px;
                padding: 2px 4px;
                background-color: rgba(0, 0, 0, 0.45);
                color: white;
                border-radius: 0 3px 0 0;
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
    }

    pub fn refresh(&self) {
        let imp = self.imp();
        let state_opt = imp.state.borrow();
        let Some(ref state_rc) = *state_opt else {
            return;
        };
        let store = state_rc.borrow().library.store.clone();
        imp.selection_model.set_model(None::<&gio::ListStore>);
        imp.selection_model.set_model(Some(&store));
        self.schedule_visible_thumbnails();
        let widget = self.downgrade();
        glib::idle_add_local(move || {
            let Some(filmstrip) = widget.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let page_size = filmstrip.imp().scroll.vadjustment().page_size();
            filmstrip.schedule_visible_thumbnails();
            if page_size > 0.0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    pub fn refresh_virtual(&self) {
        self.refresh();
    }

    pub fn set_thumbnail_sender(
        &self,
        visible_tx: async_channel::Sender<WorkerRequest>,
        preload_tx: async_channel::Sender<WorkerRequest>,
        gen: Arc<std::sync::atomic::AtomicU64>,
        pending_set: Arc<Mutex<std::collections::HashSet<PathBuf>>>,
    ) {
        *self.imp().visible_thumbnail_tx.borrow_mut() = Some(visible_tx);
        *self.imp().preload_thumbnail_tx.borrow_mut() = Some(preload_tx);
        *self.imp().thumbnail_gen.borrow_mut() = Some(gen);
        *self.imp().pending_thumbnails.borrow_mut() = Some(pending_set);
        self.schedule_visible_thumbnails();
    }

    pub fn mark_thumbnail_ready(&self, path: &Path) {
        let imp = self.imp();
        if let Some(pending) = imp.pending_thumbnails.borrow().as_ref() {
            if let Ok(mut pending) = pending.lock() {
                pending.remove(path);
            }
        }

        // Throttle UI updates: increment counter and only spawn the idle
        // task if it's the first pending notification.
        if imp.pending_notify_count.fetch_add(1, Ordering::Relaxed) == 0 {
            let widget_weak = self.downgrade();
            glib::idle_add_local(move || {
                if let Some(widget) = widget_weak.upgrade() {
                    widget
                        .imp()
                        .pending_notify_count
                        .store(0, Ordering::Relaxed);
                    widget.schedule_visible_thumbnails();
                }
                glib::ControlFlow::Break
            });
        }
    }

    pub fn schedule_visible_thumbnails(&self) {
        let imp = self.imp();
        let Some(visible_tx) = imp.visible_thumbnail_tx.borrow().as_ref().cloned() else {
            return;
        };
        let Some(preload_tx) = imp.preload_thumbnail_tx.borrow().as_ref().cloned() else {
            return;
        };
        let Some(pending_set) = imp.pending_thumbnails.borrow().as_ref().cloned() else {
            return;
        };
        let gen = imp
            .thumbnail_gen
            .borrow()
            .as_ref()
            .map_or(0, |a| a.load(Ordering::Relaxed));
        let Some(state_rc) = imp.state.borrow().as_ref().cloned() else {
            return;
        };

        let adjustment = imp.scroll.vadjustment();
        let visible_start = (adjustment.value() / ESTIMATED_ROW_HEIGHT).floor().max(0.0) as u32;
        let page_size = adjustment.page_size();
        let visible_rows = if page_size > 0.0 {
            (page_size / ESTIMATED_ROW_HEIGHT).ceil() as u32
        } else {
            0
        };

        let mut visible_range_start = visible_start;
        let mut visible_range_end = visible_start.saturating_add(visible_rows);
        let mut preload_range_start = visible_start.saturating_sub(BUFFER_ROWS);
        let mut preload_range_end = visible_start
            .saturating_add(visible_rows)
            .saturating_add(BUFFER_ROWS);

        let image_count = state_rc.borrow().library.image_count();
        let mut visible_capped_end = visible_range_end.min(image_count);
        let mut preload_capped_end = preload_range_end.min(image_count);

        if visible_rows == 0 || visible_capped_end <= visible_range_start {
            visible_range_start = 0;
            visible_range_end = FALLBACK_VISIBLE_ROWS;
            preload_range_start = 0;
            preload_range_end = FALLBACK_VISIBLE_ROWS.saturating_add(BUFFER_ROWS);
            visible_capped_end = visible_range_end.min(image_count);
            preload_capped_end = preload_range_end.min(image_count);
        }

        let mut visible_worker_paths: Vec<PathBuf> = Vec::new();
        let mut preload_worker_paths: Vec<PathBuf> = Vec::new();

        {
            let state = state_rc.borrow();
            for index in preload_range_start..preload_capped_end {
                let Some(entry) = state.library.entry_at(index) else {
                    continue;
                };
                if entry.thumbnail().is_some() {
                    continue;
                }
                let path = entry.path();
                // Workers check the disk cache first (fast path in generate_thumbnail),
                // so we just enqueue and let them handle it off the main thread.
                let is_visible =
                    index >= visible_range_start && index < visible_capped_end && visible_rows > 0;
                if is_visible {
                    visible_worker_paths.push(path);
                } else {
                    preload_worker_paths.push(path);
                }
            }
        }

        for path in visible_worker_paths {
            let should_enqueue = {
                let Ok(mut pending) = pending_set.lock() else {
                    continue;
                };
                pending.insert(path.clone())
            };
            if !should_enqueue {
                continue;
            }
            if visible_tx
                .try_send(WorkerRequest::Thumbnail {
                    path: path.clone(),
                    gen,
                })
                .is_err()
            {
                if let Ok(mut pending) = pending_set.lock() {
                    pending.remove(&path);
                }
                break;
            }
        }

        for path in preload_worker_paths {
            let should_enqueue = {
                let Ok(mut pending) = pending_set.lock() else {
                    continue;
                };
                pending.insert(path.clone())
            };
            if !should_enqueue {
                continue;
            }
            if preload_tx
                .try_send(WorkerRequest::Thumbnail {
                    path: path.clone(),
                    gen,
                })
                .is_err()
            {
                if let Ok(mut pending) = pending_set.lock() {
                    pending.remove(&path);
                }
                break;
            }
        }
    }

    pub fn connect_item_activated<F: Fn(u32) + 'static>(&self, f: F) {
        self.imp()
            .list_view
            .connect_activate(move |_, position| f(position));
    }

    pub fn navigate_to(&self, index: u32) {
        self.imp().list_view.scroll_to(
            index,
            gtk4::ListScrollFlags::SELECT | gtk4::ListScrollFlags::FOCUS,
            None,
        );
    }

    pub fn scroll_to_index(&self, index: u32) {
        let adjustment = self.imp().scroll.vadjustment();
        // If the layout hasn't happened yet (upper not yet computed), defer until
        // the scroll window knows its content size.
        if adjustment.upper() <= adjustment.page_size() + 1.0 {
            let widget_weak = self.downgrade();
            glib::idle_add_local(move || {
                let Some(widget) = widget_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let adj = widget.imp().scroll.vadjustment();
                if adj.upper() <= adj.page_size() + 1.0 {
                    return glib::ControlFlow::Continue;
                }
                let upper_bound = (adj.upper() - adj.page_size()).max(0.0);
                let offset = (index as f64 * ESTIMATED_ROW_HEIGHT).min(upper_bound);
                adj.set_value(offset);
                glib::ControlFlow::Break
            });
            return;
        }
        let upper_bound = (adjustment.upper() - adjustment.page_size()).max(0.0);
        let offset = (index as f64 * ESTIMATED_ROW_HEIGHT).min(upper_bound);
        adjustment.set_value(offset);
    }

    pub fn select_index(&self, index: u32) {
        self.imp().selection_model.set_selected(index);
    }

    pub fn activate_search(&self) {
        let imp = self.imp();
        imp.search_bar.set_search_mode(true);
        imp.search_entry.grab_focus();
    }

    pub fn is_search_active(&self) -> bool {
        self.imp().search_bar.is_search_mode()
    }

    pub fn deactivate_search(&self) {
        let imp = self.imp();
        imp.search_bar.set_search_mode(false);
        imp.search_entry.set_text("");
        self.show_autocomplete(vec![]);
    }

    pub fn set_search_capture_widget<W: IsA<gtk4::Widget>>(&self, widget: &W) {
        self.imp().search_bar.set_key_capture_widget(Some(widget));
    }

    pub fn connect_search_changed<F: Fn(&str) + 'static>(&self, f: F) {
        *self.imp().search_changed_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_search_activate<F: Fn(&str) + 'static>(&self, f: F) {
        *self.imp().search_activate_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_search_dismissed<F: Fn() + 'static>(&self, f: F) {
        *self.imp().search_dismissed_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn show_autocomplete(&self, suggestions: Vec<String>) {
        let imp = self.imp();
        while let Some(child) = imp.suggestions_list.first_child() {
            imp.suggestions_list.remove(&child);
        }

        if suggestions.is_empty() {
            imp.suggestions_popover.popdown();
            return;
        }

        for suggestion in suggestions {
            let row = gtk4::ListBoxRow::new();
            let label = gtk4::Label::new(Some(&suggestion));
            label.set_halign(gtk4::Align::Start);
            label.set_margin_top(6);
            label.set_margin_bottom(6);
            label.set_margin_start(10);
            label.set_margin_end(10);
            row.set_child(Some(&label));
            imp.suggestions_list.append(&row);
        }

        imp.suggestions_popover.popup();
    }

    pub fn connect_image_selected<F: Fn(u32) + 'static>(&self, f: F) {
        *self.imp().image_selected_cb.borrow_mut() = Some(Box::new(f));
    }

    fn emit_image_selected(&self, index: u32) {
        if let Some(cb) = self.imp().image_selected_cb.borrow().as_ref() {
            cb(index);
        }
    }

    fn emit_search_changed(&self, text: &str) {
        if let Some(cb) = self.imp().search_changed_cb.borrow().as_ref() {
            cb(text);
        }
    }

    pub fn emit_search_activate(&self, text: &str) {
        if let Some(cb) = self.imp().search_activate_cb.borrow().as_ref() {
            cb(text);
        }
    }

    fn emit_search_dismissed(&self) {
        if let Some(cb) = self.imp().search_dismissed_cb.borrow().as_ref() {
            cb();
        }
    }
}

fn list_item_path(list_item: &gtk4::ListItem) -> Option<PathBuf> {
    list_item
        .item()
        .and_downcast::<ImageEntry>()
        .map(|entry| entry.path())
}

fn launch_default_for_path(path: &Path) {
    let uri = gio::File::for_path(path).uri();
    let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
}

fn reveal_in_file_manager(path: &Path) {
    // Use org.freedesktop.FileManager1.ShowItems to highlight the specific file.
    // Nautilus, Thunar, Dolphin, and most GNOME/KDE file managers implement this.
    let uri = gio::File::for_path(path).uri().to_string();
    let parent = path.parent().map(|p| p.to_path_buf());
    std::thread::spawn(move || {
        let ok = std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{}", uri),
                "string:",
            ])
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            // Fallback: open the parent directory.
            if let Some(parent_path) = parent {
                let _ = std::process::Command::new("xdg-open")
                    .arg(&parent_path)
                    .spawn();
            }
        }
    });
}
