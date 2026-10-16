//! A virtualized, non-destructive review grid for the selected images (D30).
use super::survey_selection::SurveySelection;
use super::window::mode_header::ModeControls;
use crate::model::ImageEntry;
use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

type Close = Box<dyn Fn(Vec<PathBuf>)>;

pub(crate) struct SurveyPage {
    pub root: gtk4::Box,
    pub header: ModeControls,
    pub grid: gtk4::GridView,
    pub scroll: gtk4::ScrolledWindow,
    pub store: gio::ListStore,
    selection: gtk4::SingleSelection,
    session: RefCell<SurveySelection>,
    entries: RefCell<Vec<usize>>,
    rows: RefCell<Vec<glib::WeakRef<gtk4::ListItem>>>,
    textures: RefCell<HashMap<PathBuf, gdk::Texture>>,
    failed: RefCell<HashSet<PathBuf>>,
    pending: RefCell<HashSet<PathBuf>>,
    requests: RefCell<Option<async_channel::Sender<PathBuf>>>,
    generation: Arc<AtomicU64>,
    stamp: Cell<Option<(u64, i32, i32, u32)>>,
    dirty: Cell<bool>,
    tile_height: Cell<i32>,
    counter: gtk4::Label,
    undo: gtk4::Button,
    empty: gtk4::Stack,
    close_cb: RefCell<Option<Close>>,
}

impl SurveyPage {
    pub fn new() -> Rc<Self> {
        let store = gio::ListStore::new::<ImageEntry>();
        let selection = gtk4::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let page = Rc::new(Self {
            root: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
            header: ModeControls::new(),
            grid: gtk4::GridView::new(Some(selection.clone()), None::<gtk4::ListItemFactory>),
            scroll: gtk4::ScrolledWindow::new(),
            store,
            selection,
            session: RefCell::new(SurveySelection::default()),
            entries: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            textures: RefCell::new(HashMap::new()),
            failed: RefCell::new(HashSet::new()),
            pending: RefCell::new(HashSet::new()),
            requests: RefCell::new(None),
            generation: Arc::new(AtomicU64::new(0)),
            stamp: Cell::new(None),
            dirty: Cell::new(true),
            tile_height: Cell::new(180),
            counter: gtk4::Label::new(None),
            undo: gtk4::Button::with_label("Undo"),
            empty: gtk4::Stack::new(),
            close_cb: RefCell::new(None),
        });
        page.build();
        page
    }

    pub fn set_close_cb(&self, close: impl Fn(Vec<PathBuf>) + 'static) {
        *self.close_cb.borrow_mut() = Some(Box::new(close));
    }
    pub fn survivors(&self) -> Vec<PathBuf> {
        self.session.borrow().survivors()
    }
    pub fn close(&self) {
        let survivors = self.survivors();
        self.clear();
        if let Some(close) = self.close_cb.borrow().as_ref() {
            close(survivors);
        }
    }
    pub fn clear(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.requests.borrow_mut().take();
        self.pending.borrow_mut().clear();
        self.textures.borrow_mut().clear();
        self.failed.borrow_mut().clear();
        self.session.replace(SurveySelection::default());
        self.entries.borrow_mut().clear();
        self.store.remove_all();
        self.stamp.set(None);
        self.dirty.set(true);
    }

    pub fn start(self: &Rc<Self>, paths: Vec<PathBuf>) {
        self.clear();
        self.session.replace(SurveySelection::new(paths));
        let generation = self.generation.load(Ordering::Relaxed);
        let (tx, rx) =
            crate::thumbnails::worker::visible_requests(self.generation.clone(), generation);
        *self.requests.borrow_mut() = Some(tx);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            while let Ok((path, result)) = rx.recv().await {
                let Some(page) = weak.upgrade() else { return };
                if page.generation.load(Ordering::Relaxed) != generation {
                    return;
                }
                page.pending.borrow_mut().remove(&path);
                if let Some(image) = result {
                    let texture = gdk::MemoryTexture::new(
                        image.width as i32,
                        image.height as i32,
                        gdk::MemoryFormat::R8g8b8a8,
                        &glib::Bytes::from_owned(image.rgba_bytes),
                        image.width as usize * 4,
                    )
                    .upcast();
                    page.textures.borrow_mut().insert(path, texture);
                } else {
                    page.failed.borrow_mut().insert(path);
                }
                page.update_rows();
            }
        });
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(std::time::Duration::from_millis(40), move || {
            let Some(page) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if page.generation.load(Ordering::Relaxed) != generation {
                return glib::ControlFlow::Break;
            }
            page.visible();
            glib::ControlFlow::Continue
        });
        self.refresh();
    }

    pub fn dismiss(&self, number: usize) {
        let old = self.selection.selected();
        if self.session.borrow_mut().dismiss(number) {
            self.refresh();
            if self.store.n_items() > 0 {
                self.selection
                    .set_selected(old.min(self.store.n_items() - 1));
            }
        }
    }
    pub fn undo(&self) {
        let number = self.session.borrow_mut().undo();
        if let Some(number) = number {
            self.refresh();
            if let Some(position) = self.entries.borrow().iter().position(|i| *i == number) {
                self.selection.set_selected(position as u32);
                self.grid
                    .scroll_to(position as u32, gtk4::ListScrollFlags::FOCUS, None);
            }
        }
    }
    pub fn handle_key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        let plain = !modifiers.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK
                | gdk::ModifierType::SHIFT_MASK,
        );
        match key {
            gdk::Key::Delete | gdk::Key::BackSpace if plain => {
                let number = self
                    .entries
                    .borrow()
                    .get(self.selection.selected() as usize)
                    .copied();
                if let Some(number) = number {
                    self.dismiss(number);
                }
            }
            gdk::Key::z | gdk::Key::Z if modifiers.contains(gdk::ModifierType::CONTROL_MASK) => {
                self.undo()
            }
            gdk::Key::Escape => self.close(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    fn refresh(&self) {
        let (numbers, rows): (Vec<_>, Vec<_>) = self
            .session
            .borrow()
            .candidates()
            .into_iter()
            .map(|(i, path)| (i, ImageEntry::new(path.to_path_buf())))
            .unzip();
        let count = rows.len();
        *self.entries.borrow_mut() = numbers;
        self.store.splice(0, self.store.n_items(), &rows);
        if count > 0 {
            self.selection.set_selected(0);
        }
        self.counter.set_text(&format!("{count} remaining"));
        self.undo.set_sensitive(self.session.borrow().can_undo());
        self.empty
            .set_visible_child_name(if count == 0 { "empty" } else { "grid" });
        self.dirty.set(true);
        self.stamp.set(None);
        self.update_rows();
    }

    fn build(self: &Rc<Self>) {
        self.root.add_css_class("background");
        self.header.start.append(&gtk4::Label::new(Some("Survey")));
        self.header.center.append(&self.counter);
        self.header.end.append(&self.undo);
        let weak = Rc::downgrade(self);
        self.undo.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.undo();
            }
        });
        let done = gtk4::Button::with_label("Done");
        done.add_css_class("suggested-action");
        done.set_tooltip_text(Some("Return with the remaining images selected (Escape)"));
        let weak = Rc::downgrade(self);
        done.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.close();
            }
        });
        self.header.end.append(&done);
        self.scroll
            .set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        self.grid.add_css_class("sharpr-survey");
        self.grid.add_css_class("sharpr-canvas");
        self.grid
            .update_property(&[gtk4::accessible::Property::Label("Survey selected images")]);
        self.scroll.set_child(Some(&self.grid));
        self.empty.add_named(&self.scroll, Some("grid"));
        let empty = libadwaita::StatusPage::builder()
            .title("No images remaining")
            .description("Undo restores the last dismissed image. No files were changed.")
            .build();
        self.empty.add_named(&empty, Some("empty"));
        self.empty.set_vexpand(true);
        self.root.append(&self.empty);
        let hints = gtk4::Label::new(Some("Delete  dismiss tile    Ctrl+Z  undo    Esc  done"));
        hints.add_css_class("sharpr-keys");
        hints.set_margin_top(4);
        hints.set_margin_bottom(6);
        hints.set_wrap(true);
        self.root.append(&hints);
        let factory = gtk4::SignalListItemFactory::new();
        let weak = Rc::downgrade(self);
        factory.connect_setup(move |_, item| {
            let (Some(page), Some(item)) = (weak.upgrade(), item.downcast_ref::<gtk4::ListItem>())
            else {
                return;
            };
            let card = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            card.add_css_class("survey-tile");
            let overlay = gtk4::Overlay::new();
            let picture = gtk4::Picture::new();
            crate::ui::look::thumbnail_feedback(&picture);
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk4::ContentFit::Contain);
            picture.set_size_request(200, page.tile_height.get());
            overlay.set_child(Some(&picture));
            let dismiss = gtk4::Button::from_icon_name("window-close-symbolic");
            dismiss.add_css_class("osd");
            dismiss.set_halign(gtk4::Align::End);
            dismiss.set_valign(gtk4::Align::Start);
            dismiss.set_tooltip_text(Some("Dismiss from Survey; no files change (Delete)"));
            overlay.add_overlay(&dismiss);
            let weak = Rc::downgrade(&page);
            let row = item.downgrade();
            dismiss.connect_clicked(move |_| {
                let (Some(page), Some(item)) = (weak.upgrade(), row.upgrade()) else {
                    return;
                };
                let number = page.entries.borrow().get(item.position() as usize).copied();
                if let Some(number) = number {
                    page.dismiss(number);
                }
            });
            card.append(&overlay);
            let label = gtk4::Label::new(None);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            label.set_max_width_chars(24);
            label.add_css_class("caption");
            card.append(&label);
            item.set_child(Some(&card));
            page.rows.borrow_mut().push(item.downgrade());
        });
        let weak = Rc::downgrade(self);
        factory.connect_bind(move |_, item| {
            let (Some(page), Some(item)) = (weak.upgrade(), item.downcast_ref::<gtk4::ListItem>())
            else {
                return;
            };
            page.update_row(item);
            page.dirty.set(true);
        });
        self.grid.set_factory(Some(&factory));
    }

    fn update_rows(&self) {
        self.rows.borrow_mut().retain(|weak| {
            if let Some(item) = weak.upgrade() {
                self.update_row(&item);
                true
            } else {
                false
            }
        });
    }
    fn update_row(&self, item: &gtk4::ListItem) {
        let Some(entry) = item.item().and_downcast::<ImageEntry>() else {
            return;
        };
        let Some(card) = item.child() else { return };
        let overlay = card.first_child().and_downcast::<gtk4::Overlay>().unwrap();
        let picture = overlay.child().and_downcast::<gtk4::Picture>().unwrap();
        picture.set_size_request(200, self.tile_height.get());
        picture.set_paintable(self.textures.borrow().get(&entry.path()));
        let number = self
            .entries
            .borrow()
            .get(item.position() as usize)
            .copied()
            .unwrap_or(0)
            + 1;
        let error = if self.failed.borrow().contains(&entry.path()) {
            " · unreadable"
        } else {
            ""
        };
        card.last_child()
            .and_downcast::<gtk4::Label>()
            .unwrap()
            .set_text(&format!("{number} · {}{error}", entry.filename()));
        card.set_tooltip_text(Some(&entry.path().display().to_string()));
    }
    fn visible(&self) {
        let stamp = (
            self.scroll.vadjustment().value().to_bits(),
            self.scroll.width(),
            self.scroll.height(),
            self.store.n_items(),
        );
        if self.stamp.get() == Some(stamp) && !self.dirty.replace(false) {
            return;
        }
        self.stamp.set(Some(stamp));
        if stamp.1 > 0 && stamp.2 > 0 && stamp.3 > 0 {
            let columns = ((f64::from(stamp.3) * f64::from(stamp.1) / (f64::from(stamp.2) * 1.5))
                .sqrt()
                .round() as u32)
                .clamp(1, 4)
                .min(stamp.3);
            let rows = stamp.3.div_ceil(columns);
            self.grid.set_min_columns(1);
            self.grid.set_max_columns(columns);
            self.grid.set_min_columns(columns);
            self.tile_height
                .set((stamp.2 / rows as i32 - 40).clamp(120, 600));
            self.update_rows();
        }
        for weak in self.rows.borrow().iter() {
            let Some(item) = weak.upgrade() else { continue };
            let (Some(card), Some(entry)) =
                (item.child(), item.item().and_downcast::<ImageEntry>())
            else {
                continue;
            };
            let Some(bounds) = card.compute_bounds(&self.scroll) else {
                continue;
            };
            if bounds.height() <= 0.0 {
                // Binding can precede allocation by a frame. Retry rather
                // than treating that first empty allocation as a viewport.
                self.dirty.set(true);
                continue;
            }
            if bounds.y() + bounds.height() <= 0.0 || bounds.y() >= self.scroll.height() as f32 {
                continue;
            }
            let path = entry.path();
            if self.textures.borrow().contains_key(&path)
                || self.failed.borrow().contains(&path)
                || !self.pending.borrow_mut().insert(path.clone())
            {
                continue;
            }
            if self
                .requests
                .borrow()
                .as_ref()
                .is_none_or(|tx| tx.try_send(path.clone()).is_err())
            {
                self.pending.borrow_mut().remove(&path);
            }
        }
    }
}
