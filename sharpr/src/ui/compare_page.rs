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
use crate::ui::window::mode_header::ModeControls;

mod candidates;
use candidates::CandidateStrip;

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
        *self.imp().knockout.borrow_mut() = Some(knockout);
        self.imp().shown.replace([None, None]);
        self.imp().candidates.start(self);
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

    pub(crate) fn header(&self) -> ModeControls {
        self.imp().header.clone()
    }

    /// Keep one side by dismissing its opponent; the next candidate arrives there.
    pub fn keep(&self, side: Side) {
        self.dismiss(match side {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        });
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

    /// Shows image `number` on `side`, as the candidate row does.
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
    /// focus is: arrows keep a side, Space flips, Ctrl+Z undoes, Escape closes.
    /// Brackets remain as unadvertised aliases for the old dismiss controls.
    pub fn handle_key(&self, key: gdk4::Key, modifiers: gdk4::ModifierType) -> glib::Propagation {
        if self.imp().candidates.has_keyboard_focus()
            && matches!(key, gdk4::Key::Left | gdk4::Key::Right | gdk4::Key::space)
        {
            return glib::Propagation::Proceed;
        }
        let view = &self.imp().view;
        let control = modifiers.contains(gdk4::ModifierType::CONTROL_MASK);
        let plain = !modifiers.intersects(
            gdk4::ModifierType::CONTROL_MASK
                | gdk4::ModifierType::ALT_MASK
                | gdk4::ModifierType::SUPER_MASK
                | gdk4::ModifierType::META_MASK
                | gdk4::ModifierType::SHIFT_MASK,
        );
        match key {
            gdk4::Key::Left if plain => self.keep(Side::Left),
            gdk4::Key::Right if plain => self.keep(Side::Right),
            gdk4::Key::space if plain => {
                if view.mode() != Mode::Flip {
                    self.imp().mode_buttons.borrow()[2].set_active(true);
                } else {
                    view.flip();
                    self.update_badges();
                }
            }
            gdk4::Key::bracketleft if plain => self.dismiss(Side::Left),
            gdk4::Key::bracketright if plain => self.dismiss(Side::Right),
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
        imp.candidates.clear();
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
        imp.counter.set_text(&format!("{remaining} remaining"));
        imp.counter.set_tooltip_text(Some(&format!(
            "Started with {total} images; choosing a side never changes files"
        )));
        imp.undo.set_sensitive(can_undo);
        for side in 0..2 {
            let label = &imp.labels[side];
            let dismiss = &imp.keep[side];
            match &pair[side] {
                Some(path) => {
                    if imp.shown.borrow()[side].as_ref() != Some(path) {
                        label.set_text("…");
                    }
                    imp.filenames[side]
                        .set_text(&path.file_name().unwrap_or_default().to_string_lossy());
                    imp.filenames[side].set_tooltip_text(Some(&path.display().to_string()));
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
        if let Some(knockout) = imp.knockout.borrow().as_ref() {
            imp.candidates.sync(self, knockout);
        }
        // A lone survivor fills the canvas.
        if pair[1].is_none() {
            if imp.view.flipped() {
                imp.view.flip();
            }
            imp.mode_buttons.borrow()[2].set_active(true);
        }
        self.update_badges();
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

/// Size and format; the filename above it names the image.
fn summary(path: &Path) -> String {
    let size = crate::export::oriented_source_dimensions(path)
        .map(|(w, h)| format!("{w} × {h}"))
        .unwrap_or_else(|_| "size unknown".into());
    format!("{size} · {}", compare_asset_format(path))
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
        pub root: gtk4::Box,
        pub(super) header: ModeControls,
        pub view: CompareView,
        pub knockout: RefCell<Option<Knockout>>,
        /// The image each side shows or is loading.
        pub shown: RefCell<[Option<PathBuf>; 2]>,
        pub labels: [gtk4::Label; 2],
        pub keep: [gtk4::Button; 2],
        pub badges: [gtk4::Box; 2],
        pub counter: gtk4::Label,
        pub undo: gtk4::Button,
        pub mode_buttons: RefCell<Vec<gtk4::ToggleButton>>,
        pub close_cb: RefCell<Option<CloseCallback>>,
        pub load_generation: Cell<u64>,
        pub(crate) candidates: CandidateStrip,
        pub filenames: [gtk4::Label; 2],
    }

    impl Default for ComparePage {
        fn default() -> Self {
            Self {
                root: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
                header: ModeControls::new(),
                view: CompareView::default(),
                knockout: RefCell::new(None),
                shown: RefCell::new([None, None]),
                labels: [gtk4::Label::new(None), gtk4::Label::new(None)],
                keep: [
                    gtk4::Button::with_label("Keep Left"),
                    gtk4::Button::with_label("Keep Right"),
                ],
                badges: [
                    gtk4::Box::new(gtk4::Orientation::Vertical, 3),
                    gtk4::Box::new(gtk4::Orientation::Vertical, 3),
                ],
                counter: gtk4::Label::new(None),
                undo: gtk4::Button::from_icon_name("edit-undo-symbolic"),
                mode_buttons: RefCell::new(Vec::new()),
                close_cb: RefCell::new(None),
                load_generation: Cell::new(0),
                candidates: CandidateStrip::default(),
                filenames: [gtk4::Label::new(None), gtk4::Label::new(None)],
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
    imp.view.set_vexpand(true);
    imp.root.append(&imp.view);
    // Look prototype: a neutral canvas in both themes (I14).
    imp.view.add_css_class("sharpr-canvas");
    imp.root.add_css_class("background");

    // These controls live in the window's existing top bar (D28).
    let toolbar = &imp.header.start;
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
    let toolbar = &imp.header.center;
    let lock = gtk4::ToggleButton::with_label("Linked zoom");
    lock.set_active(true);
    lock.set_tooltip_text(Some(
        "Zoom lock: both images zoom and pan together, showing the same region",
    ));
    {
        let weak = page.downgrade();
        lock.connect_toggled(move |button| {
            if let Some(page) = weak.upgrade() {
                page.imp().view.set_locked(button.is_active());
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
    let toolbar = &imp.header.end;
    imp.undo.set_label("Undo");
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

    // Look prototype: the keys that work here, in one quiet line (D26).
    let keys = gtk4::Label::new(None);
    keys.set_markup(
        "<b>←</b> keep left   <b>→</b> keep right   <b>Space</b> flip   \
         <b>Ctrl+Z</b> undo   <b>Esc</b> done",
    );
    keys.add_css_class("sharpr-keys");
    keys.set_hexpand(true);
    keys.set_wrap(true);
    keys.set_can_target(false);
    let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
    footer.set_homogeneous(true);
    footer.set_margin_start(12);
    footer.set_margin_end(12);
    footer.set_margin_top(6);
    footer.set_margin_bottom(6);
    imp.root.append(&footer);
    let hints = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    hints.set_margin_start(12);
    hints.set_margin_end(12);
    hints.set_margin_bottom(6);
    imp.candidates.build(page, &imp.counter);
    imp.root.append(&imp.candidates.root);
    hints.append(&keys);
    imp.root.append(&hints);

    // Each image's filename and keep action stay with its side below the canvas.
    for (side, key) in ["←", "→"].into_iter().enumerate() {
        let badge = &imp.badges[side];
        let label = &imp.labels[side];
        label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        label.set_max_width_chars(48);
        let keep = &imp.keep[side];
        keep.set_tooltip_text(Some(&format!(
            "Keep this image and dismiss the other ({key}); no files change"
        )));
        let weak = page.downgrade();
        let which = if side == 0 { Side::Left } else { Side::Right };
        keep.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.keep(which);
            }
        });
        let filename = &imp.filenames[side];
        filename.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        filename.set_max_width_chars(32);
        filename.set_hexpand(true);
        filename.set_xalign(0.0);
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        row.append(filename);
        row.append(keep);
        badge.append(&row);
        label.set_xalign(0.0);
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        badge.append(label);
        footer.append(badge);
    }
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
