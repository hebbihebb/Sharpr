//! Compare (`ROADMAP.md` Phase 8, decision D8): choose the best of several
//! images, two at a time. Dismissing one brings in the next; closing
//! returns to browsing with the survivors selected. Compare never changes
//! files.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::ui::compare_item::{compare_asset_format, Knockout, Side};
use crate::ui::compare_view::{CompareView, Mode};

type CloseCallback = Box<dyn Fn(Vec<PathBuf>)>;

glib::wrapper! {
    pub struct ComparePage(ObjectSubclass<imp::ComparePage>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl ComparePage {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// `callback` gets the survivors when Compare closes.
    pub fn set_close_cb<F: Fn(Vec<PathBuf>) + 'static>(&self, callback: F) {
        *self.imp().close_cb.borrow_mut() = Some(Box::new(callback));
    }

    /// Starts comparing `paths`, two at a time, in this order.
    pub fn start(&self, paths: Vec<PathBuf>) {
        let Some(knockout) = Knockout::new(paths) else {
            return;
        };
        let all: Vec<PathBuf> = knockout
            .candidates()
            .into_iter()
            .map(|(_, p)| p.to_path_buf())
            .collect();
        *self.imp().knockout.borrow_mut() = Some(knockout);
        self.imp().shown.replace([None, None]);
        self.imp().thumbs.borrow_mut().clear();
        self.load_thumbnails(all);
        let view = &self.imp().view;
        view.set_mode(Mode::SideBySide);
        view.set_zoom(None);
        view.set_split(0.5);
        if view.flipped() {
            view.flip();
        }
        self.imp().mode_buttons.borrow()[0].set_active(true);
        self.refresh();
        view.grab_focus();
    }

    /// Images not dismissed yet.
    pub fn survivors(&self) -> Vec<PathBuf> {
        self.imp()
            .knockout
            .borrow()
            .as_ref()
            .map(Knockout::survivors)
            .unwrap_or_default()
    }

    pub fn view(&self) -> CompareView {
        self.imp().view.clone()
    }

    /// Shows image `number` on `side`, as the side's picker does.
    pub fn choose(&self, side: Side, number: usize) {
        let changed = self
            .imp()
            .knockout
            .borrow_mut()
            .as_mut()
            .is_some_and(|k| k.choose(side, number));
        if changed {
            self.refresh();
        }
    }

    /// Dismisses one side; the next image takes its place.
    pub fn dismiss(&self, side: Side) {
        let changed = self
            .imp()
            .knockout
            .borrow_mut()
            .as_mut()
            .is_some_and(|k| k.dismiss(side));
        if changed {
            self.refresh();
        }
    }

    pub fn undo(&self) {
        let changed = self
            .imp()
            .knockout
            .borrow_mut()
            .as_mut()
            .is_some_and(Knockout::undo);
        if changed {
            self.refresh();
        }
    }

    /// Compare's keys, routed from the window so they work wherever the
    /// focus is: Space flips, [ and ] dismiss, Ctrl+Z undoes, Escape closes.
    pub fn handle_key(&self, key: gdk4::Key, modifiers: gdk4::ModifierType) -> glib::Propagation {
        let view = &self.imp().view;
        let control = modifiers.contains(gdk4::ModifierType::CONTROL_MASK);
        match key {
            gdk4::Key::space if !control => {
                if view.mode() != Mode::Flip {
                    self.imp().mode_buttons.borrow()[2].set_active(true);
                } else {
                    view.flip();
                    self.update_badges();
                }
            }
            gdk4::Key::bracketleft if !control => self.dismiss(Side::Left),
            gdk4::Key::bracketright if !control => self.dismiss(Side::Right),
            gdk4::Key::z | gdk4::Key::Z if control => self.undo(),
            gdk4::Key::Escape => self.close(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    pub fn close(&self) {
        let survivors = self.survivors();
        self.clear();
        if let Some(callback) = self.imp().close_cb.borrow().as_ref() {
            callback(survivors);
        }
    }

    pub fn clear(&self) {
        let imp = self.imp();
        imp.load_generation
            .set(imp.load_generation.get().wrapping_add(1));
        imp.knockout.replace(None);
        imp.shown.replace([None, None]);
        imp.view.set_texture(0, None);
        imp.view.set_texture(1, None);
    }

    /// Shows the current pair, decoding whichever image is new.
    fn refresh(&self) {
        let imp = self.imp();
        let (pair, remaining, total, can_undo) = {
            let knockout = imp.knockout.borrow();
            let Some(k) = knockout.as_ref() else { return };
            (
                [
                    Some(k.left().to_path_buf()),
                    k.right().map(Path::to_path_buf),
                ],
                k.remaining(),
                k.total(),
                k.remaining() < k.total(),
            )
        };
        imp.counter.set_text(&if remaining == 1 {
            format!("The best of {total}")
        } else {
            format!("{remaining} of {total} left")
        });
        imp.undo.set_sensitive(can_undo);
        for side in 0..2 {
            let label = &imp.labels[side];
            let dismiss = &imp.dismiss[side];
            match &pair[side] {
                Some(path) => {
                    if imp.shown.borrow()[side].as_ref() != Some(path) {
                        label.set_text("…");
                    }
                    label.set_tooltip_text(Some(&path.display().to_string()));
                    dismiss.set_visible(pair[1].is_some());
                    imp.badges[side].set_visible(true);
                }
                None => imp.badges[side].set_visible(false),
            }
            if imp.shown.borrow()[side] == pair[side] {
                continue;
            }
            imp.shown.borrow_mut()[side] = pair[side].clone();
            imp.view.set_texture(side, None);
            if let Some(path) = pair[side].clone() {
                self.load(side, path);
            }
        }
        self.sync_pickers();
        // A lone survivor fills the canvas.
        if pair[1].is_none() {
            if imp.view.flipped() {
                imp.view.flip();
            }
            imp.mode_buttons.borrow()[2].set_active(true);
        }
        self.update_badges();
    }

    /// Fills each side's picker with the images not dismissed, showing the
    /// one on that side.
    fn sync_pickers(&self) {
        let imp = self.imp();
        let knockout = imp.knockout.borrow();
        let Some(k) = knockout.as_ref() else { return };
        let entries: Vec<(usize, PathBuf)> = k
            .candidates()
            .into_iter()
            .map(|(i, p)| (i, p.to_path_buf()))
            .collect();
        let labels: Vec<String> = entries.iter().map(|(i, p)| picker_label(*i, p)).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        imp.syncing.set(true);
        for (side, which) in [Side::Left, Side::Right].into_iter().enumerate() {
            let picker = &imp.pickers[side];
            // Replace the list only when the images left changed; a new
            // choice just moves the selection.
            if imp.picker_entries.borrow()[side] != entries || picker.model().is_none() {
                imp.picker_entries.borrow_mut()[side] = entries.clone();
                picker.set_model(Some(&gtk4::StringList::new(&labels)));
            }
            if let Some(position) = k
                .shown(which)
                .and_then(|n| entries.iter().position(|(i, _)| *i == n))
            {
                picker.set_selected(position as u32);
            }
        }
        imp.syncing.set(false);
    }

    /// Cached thumbnails for the pickers, on one worker; images without
    /// one simply show none.
    fn load_thumbnails(&self, paths: Vec<PathBuf>) {
        let generation = self.imp().load_generation.get();
        let (tx, rx) = async_channel::bounded(8);
        std::thread::spawn(move || {
            for path in paths {
                let Some((rgba, w, h)) = crate::thumbnails::worker::load_cached_rgba(&path) else {
                    continue;
                };
                let Some(image) = image::RgbaImage::from_raw(w, h, rgba) else {
                    continue;
                };
                let small = image::DynamicImage::ImageRgba8(image)
                    .thumbnail(96, 64)
                    .to_rgba8();
                if tx.send_blocking((path, small)).is_err() {
                    return;
                }
            }
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            while let Ok((path, small)) = rx.recv().await {
                let Some(page) = weak.upgrade() else { return };
                let imp = page.imp();
                if imp.load_generation.get() != generation {
                    return;
                }
                let (w, h) = small.dimensions();
                let texture: gdk4::Texture = gdk4::MemoryTexture::new(
                    w as i32,
                    h as i32,
                    gdk4::MemoryFormat::R8g8b8a8,
                    &glib::Bytes::from_owned(small.into_raw()),
                    w as usize * 4,
                )
                .upcast();
                imp.bound
                    .borrow_mut()
                    .retain(|(bound, picture)| match picture.upgrade() {
                        Some(picture) => {
                            if *bound == path {
                                picture.set_paintable(Some(&texture));
                            }
                            true
                        }
                        None => false,
                    });
                imp.thumbs.borrow_mut().insert(path, texture);
            }
        });
    }

    /// In Flip mode, the badge of the image not on screen is dimmed.
    fn update_badges(&self) {
        let view = &self.imp().view;
        let shown = usize::from(view.flipped());
        for (side, badge) in self.imp().badges.iter().enumerate() {
            let hidden = view.mode() == Mode::Flip && side != shown;
            badge.set_opacity(if hidden { 0.45 } else { 1.0 });
        }
    }

    fn load(&self, side: usize, path: PathBuf) {
        let generation = self.imp().load_generation.get();
        let (tx, rx) = async_channel::bounded(1);
        let worker_path = path.clone();
        std::thread::spawn(move || {
            // The summary reads the header and EXIF: off the GTK thread too.
            let _ = tx.send_blocking((summary(&worker_path), decode_image(&worker_path)));
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok((text, result)) = rx.recv().await else {
                return;
            };
            let Some(page) = weak.upgrade() else { return };
            let imp = page.imp();
            if imp.load_generation.get() != generation
                || imp.shown.borrow()[side].as_ref() != Some(&path)
            {
                return;
            }
            imp.labels[side].set_text(&text);
            match result {
                Ok(decoded) => {
                    let texture: gdk4::Texture = gdk4::MemoryTexture::new(
                        decoded.width,
                        decoded.height,
                        gdk4::MemoryFormat::R8g8b8a8,
                        &glib::Bytes::from_owned(decoded.pixels),
                        decoded.width as usize * 4,
                    )
                    .upcast();
                    imp.view.set_texture(side, Some(texture));
                }
                Err(message) => imp.labels[side].set_text(&message),
            }
        });
    }
}

impl Default for ComparePage {
    fn default() -> Self {
        Self::new()
    }
}

/// Size and format; the picker beside it names the image.
fn summary(path: &Path) -> String {
    let size = crate::export::oriented_source_dimensions(path)
        .map(|(w, h)| format!("{w} × {h}"))
        .unwrap_or_else(|_| "size unknown".into());
    format!("{size} · {}", compare_asset_format(path))
}

/// "3 · name.jpg": the image's number in the selection, from 1.
fn picker_label(number: usize, path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    format!("{} · {name}", number + 1)
}

struct DecodedImage {
    pixels: Vec<u8>,
    width: i32,
    height: i32,
}

/// Decodes upright, within the export decoder's memory limits.
fn decode_image(path: &Path) -> Result<DecodedImage, String> {
    let image = crate::export::decode_oriented(path)
        .map_err(|error| format!("Could not load {}: {error}", path.display()))?;
    let rgba = image.to_rgba8();
    Ok(DecodedImage {
        width: rgba.width() as i32,
        height: rgba.height() as i32,
        pixels: rgba.into_raw(),
    })
}

mod imp {
    use super::*;

    pub struct ComparePage {
        pub root: gtk4::Overlay,
        pub view: CompareView,
        pub knockout: RefCell<Option<Knockout>>,
        /// The image each side shows or is loading.
        pub shown: RefCell<[Option<PathBuf>; 2]>,
        pub labels: [gtk4::Label; 2],
        pub dismiss: [gtk4::Button; 2],
        pub badges: [gtk4::Box; 2],
        pub counter: gtk4::Label,
        pub undo: gtk4::Button,
        pub mode_buttons: RefCell<Vec<gtk4::ToggleButton>>,
        pub close_cb: RefCell<Option<CloseCallback>>,
        pub load_generation: Cell<u64>,
        /// Each side's picker of the images not dismissed.
        pub pickers: [gtk4::DropDown; 2],
        /// What each picker lists: (number, path), in its order.
        pub picker_entries: RefCell<[Vec<(usize, PathBuf)>; 2]>,
        /// Set while the pickers are refilled, so that isn't a choice.
        pub syncing: Cell<bool>,
        pub thumbs: RefCell<std::collections::HashMap<PathBuf, gdk4::Texture>>,
        /// Picker rows on screen, to show thumbnails that arrive later.
        pub bound: RefCell<Vec<(PathBuf, glib::WeakRef<gtk4::Picture>)>>,
    }

    impl Default for ComparePage {
        fn default() -> Self {
            Self {
                root: gtk4::Overlay::new(),
                view: CompareView::default(),
                knockout: RefCell::new(None),
                shown: RefCell::new([None, None]),
                labels: [gtk4::Label::new(None), gtk4::Label::new(None)],
                dismiss: [
                    gtk4::Button::from_icon_name("window-close-symbolic"),
                    gtk4::Button::from_icon_name("window-close-symbolic"),
                ],
                badges: [
                    gtk4::Box::new(gtk4::Orientation::Horizontal, 6),
                    gtk4::Box::new(gtk4::Orientation::Horizontal, 6),
                ],
                counter: gtk4::Label::new(None),
                undo: gtk4::Button::from_icon_name("edit-undo-symbolic"),
                mode_buttons: RefCell::new(Vec::new()),
                close_cb: RefCell::new(None),
                load_generation: Cell::new(0),
                pickers: [
                    gtk4::DropDown::new(None::<gtk4::gio::ListModel>, None::<gtk4::Expression>),
                    gtk4::DropDown::new(None::<gtk4::gio::ListModel>, None::<gtk4::Expression>),
                ],
                picker_entries: RefCell::new([Vec::new(), Vec::new()]),
                syncing: Cell::new(false),
                thumbs: RefCell::new(std::collections::HashMap::new()),
                bound: RefCell::new(Vec::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ComparePage {
        const NAME: &'static str = "SharprComparePage";
        type Type = super::ComparePage;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for ComparePage {
        fn constructed(&self) {
            self.parent_constructed();
            super::build(&self.obj());
        }

        fn dispose(&self) {
            self.root.unparent();
        }
    }

    impl WidgetImpl for ComparePage {}
}

fn build(page: &ComparePage) {
    let imp = page.imp();
    imp.root.set_parent(page);
    imp.root.set_child(Some(&imp.view));
    // Look prototype: a neutral canvas in both themes (I14).
    imp.root.add_css_class("sharpr-canvas");

    // Top: modes, zoom lock, zoom, progress, undo, close.
    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    toolbar.add_css_class("osd");
    toolbar.add_css_class("toolbar");
    toolbar.set_halign(gtk4::Align::Center);
    toolbar.set_valign(gtk4::Align::Start);
    toolbar.set_margin_top(12);
    let modes = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    modes.add_css_class("linked");
    let mut buttons = Vec::new();
    for (label, tooltip, mode) in [
        (
            "Side by Side",
            "Both images next to each other",
            Mode::SideBySide,
        ),
        (
            "Split",
            "One image each side of a divider you can drag",
            Mode::Split,
        ),
        ("Flip", "One image at a time; Space switches", Mode::Flip),
    ] {
        let button = gtk4::ToggleButton::with_label(label);
        button.set_tooltip_text(Some(tooltip));
        if let Some(first) = buttons.first() {
            button.set_group(Some(first));
        }
        let weak = page.downgrade();
        button.connect_toggled(move |button| {
            if let (true, Some(page)) = (button.is_active(), weak.upgrade()) {
                page.imp().view.set_mode(mode);
                page.update_badges();
            }
        });
        modes.append(&button);
        buttons.push(button);
    }
    *imp.mode_buttons.borrow_mut() = buttons;
    toolbar.append(&modes);
    let lock = gtk4::ToggleButton::new();
    lock.set_icon_name("changes-prevent-symbolic");
    lock.set_active(true);
    lock.set_tooltip_text(Some(
        "Zoom lock: both images zoom and pan together, showing the same region",
    ));
    {
        let weak = page.downgrade();
        lock.connect_toggled(move |button| {
            if let Some(page) = weak.upgrade() {
                page.imp().view.set_locked(button.is_active());
                button.set_icon_name(if button.is_active() {
                    "changes-prevent-symbolic"
                } else {
                    "changes-allow-symbolic"
                });
            }
        });
    }
    toolbar.append(&lock);
    let fit = gtk4::Button::with_label("Fit");
    fit.set_tooltip_text(Some("Fit both images (or double-click)"));
    let actual = gtk4::Button::with_label("100%");
    actual.set_tooltip_text(Some("Actual pixels of the left image"));
    {
        let weak = page.downgrade();
        fit.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.imp().view.set_zoom(None);
            }
        });
        let weak = page.downgrade();
        actual.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.imp().view.set_zoom(Some(1.0));
            }
        });
    }
    toolbar.append(&fit);
    toolbar.append(&actual);
    imp.counter.add_css_class("numeric");
    imp.counter.set_margin_start(6);
    toolbar.append(&imp.counter);
    imp.undo
        .set_tooltip_text(Some("Bring back the last dismissed image (Ctrl+Z)"));
    {
        let weak = page.downgrade();
        imp.undo.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.undo();
            }
        });
    }
    toolbar.append(&imp.undo);
    let close = gtk4::Button::with_label("Done");
    close.add_css_class("suggested-action");
    close.set_tooltip_text(Some(
        "Back to browsing with the remaining images selected (Escape)",
    ));
    {
        let weak = page.downgrade();
        close.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.close();
            }
        });
    }
    toolbar.append(&close);
    imp.root.add_overlay(&toolbar);

    // Bottom corners: each image's name and its dismiss button.
    for (side, (align, key)) in [(gtk4::Align::Start, "["), (gtk4::Align::End, "]")]
        .into_iter()
        .enumerate()
    {
        let badge = &imp.badges[side];
        badge.add_css_class("osd");
        badge.add_css_class("toolbar");
        badge.set_halign(align);
        badge.set_valign(gtk4::Align::End);
        badge.set_margin_start(12);
        badge.set_margin_end(12);
        badge.set_margin_bottom(12);
        let label = &imp.labels[side];
        label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        label.set_max_width_chars(48);
        let dismiss = &imp.dismiss[side];
        dismiss.add_css_class("flat");
        dismiss.set_tooltip_text(Some(&format!("Dismiss this image ({key})")));
        let weak = page.downgrade();
        let which = if side == 0 { Side::Left } else { Side::Right };
        dismiss.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.dismiss(which);
            }
        });
        let picker = &imp.pickers[side];
        build_picker(page, picker, side, which);
        badge.append(picker);
        badge.append(label);
        badge.append(dismiss);
        imp.root.add_overlay(badge);
    }
}

/// A side's picker: the button names the image shown; the list shows
/// every image not dismissed, numbered, with its thumbnail. Typing filters.
fn build_picker(page: &ComparePage, picker: &gtk4::DropDown, side: usize, which: Side) {
    picker.set_tooltip_text(Some("Choose the image for this side"));
    picker.set_expression(Some(gtk4::PropertyExpression::new(
        gtk4::StringObject::static_type(),
        None::<gtk4::Expression>,
        "string",
    )));
    picker.set_enable_search(true);
    let button = gtk4::SignalListItemFactory::new();
    button.connect_setup(|_, item| {
        let label = gtk4::Label::new(None);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        label.set_max_width_chars(28);
        label.set_xalign(0.0);
        if let Some(item) = item.downcast_ref::<gtk4::ListItem>() {
            item.set_child(Some(&label));
        }
    });
    button.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk4::ListItem>() else {
            return;
        };
        let text = item
            .item()
            .and_downcast::<gtk4::StringObject>()
            .map(|o| o.string().to_string())
            .unwrap_or_default();
        if let Some(label) = item.child().and_downcast::<gtk4::Label>() {
            label.set_text(&text);
        }
    });
    picker.set_factory(Some(&button));
    let rows = gtk4::SignalListItemFactory::new();
    rows.connect_setup(|_, item| {
        let line = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let picture = gtk4::Picture::new();
        picture.set_size_request(64, 40);
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk4::ContentFit::Cover);
        let label = gtk4::Label::new(None);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        label.set_max_width_chars(36);
        label.set_xalign(0.0);
        line.append(&picture);
        line.append(&label);
        if let Some(item) = item.downcast_ref::<gtk4::ListItem>() {
            item.set_child(Some(&line));
        }
    });
    let weak = page.downgrade();
    rows.connect_bind(move |_, item| {
        let (Some(page), Some(item)) = (weak.upgrade(), item.downcast_ref::<gtk4::ListItem>())
        else {
            return;
        };
        let Some(line) = item.child() else { return };
        let (Some(picture), Some(label)) = (
            line.first_child().and_downcast::<gtk4::Picture>(),
            line.last_child().and_downcast::<gtk4::Label>(),
        ) else {
            return;
        };
        let text = item
            .item()
            .and_downcast::<gtk4::StringObject>()
            .map(|o| o.string().to_string())
            .unwrap_or_default();
        label.set_text(&text);
        let imp = page.imp();
        // The filtered list's rows are found by their label.
        let path = imp.picker_entries.borrow()[side]
            .iter()
            .find(|(i, p)| picker_label(*i, p) == text)
            .map(|(_, p)| p.clone());
        let Some(path) = path else { return };
        picture.set_paintable(imp.thumbs.borrow().get(&path));
        imp.bound.borrow_mut().push((path, picture.downgrade()));
    });
    picker.set_list_factory(Some(&rows));
    let weak = page.downgrade();
    picker.connect_selected_notify(move |picker| {
        let Some(page) = weak.upgrade() else { return };
        if page.imp().syncing.get() {
            return;
        }
        let number = page.imp().picker_entries.borrow()[side]
            .get(picker.selected() as usize)
            .map(|(i, _)| *i);
        // Applied after this signal: refilling a drop-down from inside its
        // own selection change makes GTK loop.
        if let Some(number) = number {
            glib::idle_add_local_once(move || page.choose(which, number));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST: AtomicU64 = AtomicU64::new(1);

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sharpr-compare-{name}-{}-{}",
            std::process::id(),
            NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn missing_file_returns_visible_error_text() {
        let path = test_path("missing.png");
        let error = decode_image(&path).err().unwrap();
        assert!(error.contains("Could not load"));
        assert!(error.contains("missing.png"));
    }

    #[test]
    fn corrupt_file_returns_visible_error_text() {
        let path = test_path("corrupt.png");
        std::fs::write(&path, b"not an image").unwrap();
        let error = decode_image(&path).err().unwrap();
        assert!(error.contains("Could not load"));
        std::fs::remove_file(path).unwrap();
    }
}
