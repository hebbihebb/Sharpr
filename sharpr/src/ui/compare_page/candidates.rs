//! The virtualized candidate row and its visible-thumbnail requests (D29).
use super::*;
use crate::model::ImageEntry;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub(crate) struct CandidateStrip {
    pub root: gtk4::Box,
    pub list: gtk4::ListView,
    pub store: gtk4::gio::ListStore,
    pub scroll: gtk4::ScrolledWindow,
    pub replace: [gtk4::ToggleButton; 2],
    pub target: Cell<Side>,
    entries: RefCell<Vec<(usize, PathBuf)>>,
    rows: RefCell<Vec<glib::WeakRef<gtk4::ListItem>>>,
    textures: RefCell<HashMap<PathBuf, gdk4::Texture>>,
    pending: RefCell<HashSet<PathBuf>>,
    requests: RefCell<Option<async_channel::Sender<PathBuf>>>,
    generation: Arc<AtomicU64>,
    visible_stamp: Cell<Option<(u64, i32, u32)>>,
    visible_dirty: Cell<bool>,
}

impl Default for CandidateStrip {
    fn default() -> Self {
        let store = gtk4::gio::ListStore::new::<ImageEntry>();
        let selection = gtk4::NoSelection::new(Some(store.clone()));
        let list = gtk4::ListView::new(Some(selection), None::<gtk4::ListItemFactory>);
        list.set_orientation(gtk4::Orientation::Horizontal);
        list.set_single_click_activate(true);
        Self {
            root: gtk4::Box::new(gtk4::Orientation::Horizontal, 12),
            list,
            store,
            scroll: gtk4::ScrolledWindow::new(),
            replace: [
                gtk4::ToggleButton::with_label("Left"),
                gtk4::ToggleButton::with_label("Right"),
            ],
            target: Cell::new(Side::Right),
            entries: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            textures: RefCell::new(HashMap::new()),
            pending: RefCell::new(HashSet::new()),
            requests: RefCell::new(None),
            generation: Arc::new(AtomicU64::new(0)),
            visible_stamp: Cell::new(None),
            visible_dirty: Cell::new(true),
        }
    }
}

impl CandidateStrip {
    pub fn build(&self, page: &ComparePage, counter: &gtk4::Label) {
        self.root.add_css_class("compare-candidates");
        self.root.set_margin_start(12);
        self.root.set_margin_end(12);
        self.root.set_margin_top(4);
        self.root.set_margin_bottom(4);
        let controls = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        controls.set_valign(gtk4::Align::Center);
        controls.append(&gtk4::Label::new(Some("Replace")));
        let targets = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        targets.add_css_class("linked");
        self.replace[1].set_group(Some(&self.replace[0]));
        for (button, side) in self.replace.iter().zip([Side::Left, Side::Right]) {
            button.set_tooltip_text(Some(match side {
                Side::Left => "Choose a thumbnail to replace the left image",
                Side::Right => "Choose a thumbnail to replace the right image",
            }));
            let weak = page.downgrade();
            button.connect_toggled(move |button| {
                if button.is_active() {
                    if let Some(page) = weak.upgrade() {
                        page.imp().candidates.target.set(side);
                    }
                }
            });
            targets.append(button);
        }
        controls.append(&targets);
        counter.add_css_class("caption");
        counter.add_css_class("dim-label");
        controls.append(counter);
        self.root.append(&controls);
        let scroll = &self.scroll;
        scroll.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Never);
        scroll.set_hexpand(true);
        scroll.set_min_content_height(94);
        scroll.set_child(Some(&self.list));
        self.root.append(scroll);
        self.list
            .update_property(&[gtk4::accessible::Property::Label("Compare candidates")]);

        let factory = gtk4::SignalListItemFactory::new();
        let weak = page.downgrade();
        factory.connect_setup(move |_, item| {
            let (Some(page), Some(item)) = (weak.upgrade(), item.downcast_ref::<gtk4::ListItem>())
            else {
                return;
            };
            let card = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
            card.add_css_class("compare-candidate");
            let overlay = gtk4::Overlay::new();
            let picture = gtk4::Picture::new();
            picture.set_size_request(96, 60);
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk4::ContentFit::Contain);
            overlay.set_child(Some(&picture));
            let badge = gtk4::Label::new(None);
            badge.add_css_class("compare-side");
            badge.set_halign(gtk4::Align::End);
            badge.set_valign(gtk4::Align::Start);
            overlay.add_overlay(&badge);
            card.append(&overlay);
            let label = gtk4::Label::new(None);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            label.set_width_chars(12);
            label.set_max_width_chars(12);
            label.add_css_class("caption");
            card.append(&label);
            item.set_child(Some(&card));
            page.imp()
                .candidates
                .rows
                .borrow_mut()
                .push(item.downgrade());
        });
        let weak = page.downgrade();
        factory.connect_bind(move |_, item| {
            let (Some(page), Some(item)) = (weak.upgrade(), item.downcast_ref::<gtk4::ListItem>())
            else {
                return;
            };
            page.imp().candidates.update_row(&page, item);
            page.imp().candidates.visible_dirty.set(true);
        });
        self.list.set_factory(Some(&factory));
        let weak = page.downgrade();
        self.list.connect_activate(move |_, position| {
            let Some(page) = weak.upgrade() else { return };
            let strip = &page.imp().candidates;
            let number = strip
                .entries
                .borrow()
                .get(position as usize)
                .map(|(i, _)| *i);
            if let Some(number) = number {
                page.choose(strip.target.get(), number);
            }
        });
    }

    /// One serial worker per Compare session, fed only by visible rows. It uses
    /// the existing cache/decoder and never changes the filmstrip's scheduler.
    pub fn start(&self, page: &ComparePage) {
        self.clear();
        self.replace[1].set_active(true);
        self.target.set(Side::Right);
        let generation = self.generation.load(Ordering::Relaxed);
        let current = self.generation.clone();
        let (request_tx, request_rx) = async_channel::unbounded::<PathBuf>();
        let (tx, rx) = async_channel::bounded(8);
        *self.requests.borrow_mut() = Some(request_tx);
        std::thread::spawn(move || {
            while let Ok(path) = request_rx.recv_blocking() {
                if current.load(Ordering::Relaxed) != generation {
                    return;
                }
                let result = crate::thumbnails::worker::generate_thumbnail(&path);
                if current.load(Ordering::Relaxed) != generation
                    || tx.send_blocking((path, result)).is_err()
                {
                    return;
                }
            }
        });
        let weak = page.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(40), move || {
            let Some(page) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let strip = &page.imp().candidates;
            if strip.generation.load(Ordering::Relaxed) != generation {
                return glib::ControlFlow::Break;
            }
            strip.request_visible();
            glib::ControlFlow::Continue
        });
        let weak = page.downgrade();
        glib::MainContext::default().spawn_local(async move {
            while let Ok((path, result)) = rx.recv().await {
                let Some(page) = weak.upgrade() else { return };
                let strip = &page.imp().candidates;
                if strip.generation.load(Ordering::Relaxed) != generation {
                    return;
                }
                strip.pending.borrow_mut().remove(&path);
                if let Some(result) = result {
                    let texture = gdk4::MemoryTexture::new(
                        result.width as i32,
                        result.height as i32,
                        gdk4::MemoryFormat::R8g8b8a8,
                        &glib::Bytes::from_owned(result.rgba_bytes),
                        result.width as usize * 4,
                    )
                    .upcast::<gdk4::Texture>();
                    strip.textures.borrow_mut().insert(path, texture);
                    strip.update_rows(&page);
                }
            }
        });
    }

    pub fn clear(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.requests.borrow_mut().take();
        self.pending.borrow_mut().clear();
        self.textures.borrow_mut().clear();
        self.entries.borrow_mut().clear();
        self.visible_stamp.set(None);
        self.visible_dirty.set(true);
        self.store.remove_all();
    }

    pub fn sync(&self, page: &ComparePage, knockout: &Knockout) {
        let entries: Vec<_> = knockout
            .candidates()
            .into_iter()
            .map(|(i, p)| (i, p.to_path_buf()))
            .collect();
        if *self.entries.borrow() != entries {
            let rows: Vec<_> = entries
                .iter()
                .map(|(_, path)| ImageEntry::new(path.clone()))
                .collect();
            *self.entries.borrow_mut() = entries;
            self.store.splice(0, self.store.n_items(), &rows);
            self.visible_dirty.set(true);
        }
        for button in &self.replace {
            button.set_sensitive(knockout.remaining() > 1);
        }
        self.update_rows(page);
    }

    pub fn has_keyboard_focus(&self) -> bool {
        self.list
            .root()
            .and_downcast::<gtk4::Window>()
            .and_then(|window| gtk4::prelude::GtkWindowExt::focus(&window))
            .is_some_and(|focus| focus == self.list || focus.is_ancestor(&self.list))
    }

    fn request(&self, path: PathBuf) {
        if self.textures.borrow().contains_key(&path)
            || !self.pending.borrow_mut().insert(path.clone())
        {
            return;
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

    fn request_visible(&self) {
        let stamp = (
            self.scroll.hadjustment().value().to_bits(),
            self.scroll.width(),
            self.store.n_items(),
        );
        if self.visible_stamp.get() == Some(stamp) && !self.visible_dirty.replace(false) {
            return;
        }
        self.visible_stamp.set(Some(stamp));
        for weak in self.rows.borrow().iter() {
            let Some(item) = weak.upgrade() else { continue };
            let Some(card) = item.child() else { continue };
            let Some(bounds) = card.compute_bounds(&self.scroll) else {
                continue;
            };
            if bounds.width() > 0.0
                && bounds.x() + bounds.width() > 0.0
                && bounds.x() < self.scroll.width() as f32
            {
                if let Some(entry) = item.item().and_downcast::<ImageEntry>() {
                    self.request(entry.path());
                }
            }
        }
    }

    fn update_rows(&self, page: &ComparePage) {
        self.rows.borrow_mut().retain(|weak| {
            if let Some(item) = weak.upgrade() {
                self.update_row(page, &item);
                true
            } else {
                false
            }
        });
    }

    fn update_row(&self, page: &ComparePage, item: &gtk4::ListItem) {
        let Some(entry) = item.item().and_downcast::<ImageEntry>() else {
            return;
        };
        let Some(card) = item.child() else { return };
        let overlay = card.first_child().and_downcast::<gtk4::Overlay>().unwrap();
        let picture = overlay.child().and_downcast::<gtk4::Picture>().unwrap();
        let badge = overlay.last_child().and_downcast::<gtk4::Label>().unwrap();
        let label = card.last_child().and_downcast::<gtk4::Label>().unwrap();
        let path = entry.path();
        let shown = page.imp().shown.borrow();
        let side = if shown[0].as_ref() == Some(&path) {
            "L"
        } else if shown[1].as_ref() == Some(&path) {
            "R"
        } else {
            ""
        };
        badge.set_text(side);
        badge.set_visible(!side.is_empty());
        if side.is_empty() {
            card.remove_css_class("compare-current");
        } else {
            card.add_css_class("compare-current");
        }
        picture.set_paintable(self.textures.borrow().get(&path));
        label.set_text(&entry.filename());
        card.set_tooltip_text(Some(&path.display().to_string()));
        card.update_property(&[gtk4::accessible::Property::Label(&format!(
            "{} {}",
            side,
            entry.filename()
        ))]);
    }
}
