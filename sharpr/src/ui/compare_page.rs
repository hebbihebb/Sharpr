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
mod decodes;
use decodes::{Gate, Held, Job, Step, Tier};

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
        imp.candidates.clear();
        imp.knockout.replace(None);
        imp.shown.replace([None, None]);
        for side in 0..2 {
            self.forget(side);
        }
    }

    /// `side` no longer shows what it did: results still coming for it are
    /// stale, and the decode waiting for it is dropped.
    fn forget(&self, side: usize) {
        let imp = self.imp();
        let mut epochs = imp.epochs.get();
        epochs[side] = epochs[side].wrapping_add(1);
        imp.epochs.set(epochs);
        imp.gates.borrow_mut()[side].clear();
        imp.previews.borrow_mut()[side] = None;
        imp.held.borrow_mut()[side] = Held::Preview;
        imp.view.clear_texture(side);
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
            self.forget(side);
            if let Some(path) = pair[side].clone() {
                self.submit(
                    side,
                    Job {
                        epoch: imp.epochs.get()[side],
                        path,
                        tier: Tier::Preview,
                    },
                );
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

    /// Decodes `job` for `side` now, or after the one running there; a job
    /// already waiting is dropped for it.
    fn submit(&self, side: usize, job: Job) {
        let start = self.imp().gates.borrow_mut()[side].submit(job);
        if let Some(job) = start {
            self.run(side, job);
        }
    }

    fn run(&self, side: usize, job: Job) {
        let (tx, rx) = async_channel::bounded(1);
        let worker_path = job.path.clone();
        let tier = job.tier;
        std::thread::spawn(move || {
            #[cfg(test)]
            let _probe = probe::enter();
            // The summary reads the header and EXIF: off the GTK thread too.
            let text = (tier == Tier::Preview).then(|| summary(&worker_path));
            let _ = tx.send_blocking((text, decode_image(&worker_path, tier)));
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok((text, result)) = rx.recv().await else {
                return;
            };
            let Some(page) = weak.upgrade() else { return };
            let imp = page.imp();
            // This side is free again: the newest waiting job starts.
            let next = imp.gates.borrow_mut()[side].finish();
            if let Some(next) = next {
                page.run(side, next);
            }
            if imp.epochs.get()[side] != job.epoch {
                return;
            }
            if let Some(text) = text {
                imp.labels[side].set_text(&text);
            }
            match (result, job.tier) {
                (Ok(decoded), tier) => page.show(side, decoded, tier),
                (Err(message), Tier::Preview) => {
                    imp.labels[side].set_text(&message);
                    imp.held.borrow_mut()[side] = Held::Failed;
                }
                // The preview stays, and the full image is not asked for again.
                (Err(_), Tier::Full) => imp.held.borrow_mut()[side] = Held::Failed,
            }
        });
    }

    fn show(&self, side: usize, decoded: DecodedImage, tier: Tier) {
        let imp = self.imp();
        let texture: gdk4::Texture = gdk4::MemoryTexture::new(
            decoded.width,
            decoded.height,
            gdk4::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(decoded.pixels),
            decoded.width as usize * 4,
        )
        .upcast();
        if tier == Tier::Preview {
            imp.previews.borrow_mut()[side] = Some(texture.clone());
        }
        imp.held.borrow_mut()[side] = match tier {
            Tier::Preview => Held::Preview,
            Tier::Full => Held::Full,
        };
        imp.view.set_texture(side, texture, decoded.image);
        self.check_resolution();
    }

    /// Asks for the full image of a side zoomed past its preview, and gives
    /// it up again once the view is back out.
    fn check_resolution(&self) {
        let imp = self.imp();
        let device_scale = f64::from(imp.view.scale_factor());
        for side in 0..2 {
            let (Some(image), Some(scale)) =
                (imp.view.image_size(side), imp.view.drawn_scale(side))
            else {
                continue;
            };
            let held = imp.held.borrow()[side];
            match decodes::next_step(held, image, scale, device_scale) {
                Step::Nothing => {}
                Step::LoadFull => {
                    let Some(path) = imp.shown.borrow()[side].clone() else {
                        continue;
                    };
                    imp.held.borrow_mut()[side] = Held::Loading;
                    let epoch = imp.epochs.get()[side];
                    self.submit(
                        side,
                        Job {
                            epoch,
                            path,
                            tier: Tier::Full,
                        },
                    );
                }
                Step::DropFull => {
                    let preview = imp.previews.borrow()[side].clone();
                    if let Some(preview) = preview {
                        imp.held.borrow_mut()[side] = Held::Preview;
                        imp.view.set_texture(side, preview, image);
                    }
                }
            }
        }
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
    /// The texture's size.
    width: i32,
    height: i32,
    /// The whole image's size, which a preview is smaller than.
    image: (u32, u32),
}

/// Decodes upright, within the export decoder's memory limits. A preview
/// holds at most `PREVIEW_EDGE` pixels on its long edge.
fn decode_image(path: &Path, tier: Tier) -> Result<DecodedImage, String> {
    let image = crate::export::decode_oriented(path)
        .map_err(|error| format!("Could not load {}: {error}", path.display()))?;
    let whole = (image.width(), image.height());
    let (width, height) = match tier {
        Tier::Full => whole,
        Tier::Preview => decodes::preview_size(whole),
    };
    let rgba = if (width, height) == whole {
        image.to_rgba8()
    } else {
        image
            .resize_exact(width, height, image::imageops::FilterType::Triangle)
            .to_rgba8()
    };
    Ok(DecodedImage {
        width: rgba.width() as i32,
        height: rgba.height() as i32,
        pixels: rgba.into_raw(),
        image: whole,
    })
}

/// Test-only count of the decodes running at once.
#[cfg(test)]
pub(crate) mod probe {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);
    static STARTED: AtomicUsize = AtomicUsize::new(0);

    pub struct Probe;

    /// Holds the decode open for a moment so overlapping ones are seen.
    pub fn enter() -> Probe {
        STARTED.fetch_add(1, Ordering::SeqCst);
        let live = LIVE.fetch_add(1, Ordering::SeqCst) + 1;
        PEAK.fetch_max(live, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(150));
        Probe
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }

    pub fn reset() {
        PEAK.store(0, Ordering::SeqCst);
        STARTED.store(0, Ordering::SeqCst);
    }

    /// (most decodes running at once, decodes started) since `reset`.
    pub fn counts() -> (usize, usize) {
        (PEAK.load(Ordering::SeqCst), STARTED.load(Ordering::SeqCst))
    }
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
        /// Per side: counts what the side has shown, so results for an
        /// earlier image are recognised and dropped.
        pub epochs: Cell<[u64; 2]>,
        /// Per side: one decode at a time, the newest request waiting.
        pub gates: RefCell<[Gate<Job>; 2]>,
        /// Per side: what the texture holds.
        pub held: RefCell<[Held; 2]>,
        /// Per side: the screen-sized preview, kept to return to.
        pub previews: RefCell<[Option<gdk4::Texture>; 2]>,
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
                epochs: Cell::new([0; 2]),
                gates: RefCell::new(Default::default()),
                held: RefCell::new([Held::Preview; 2]),
                previews: RefCell::new([None, None]),
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
    {
        let weak = page.downgrade();
        imp.view.connect_view_changed(move || {
            if let Some(page) = weak.upgrade() {
                page.check_resolution();
            }
        });
    }
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
        let error = decode_image(&path, Tier::Full).err().unwrap();
        assert!(error.contains("Could not load"));
        assert!(error.contains("missing.png"));
    }

    #[test]
    fn corrupt_file_returns_visible_error_text() {
        let path = test_path("corrupt.png");
        std::fs::write(&path, b"not an image").unwrap();
        let error = decode_image(&path, Tier::Full).err().unwrap();
        assert!(error.contains("Could not load"));
        std::fs::remove_file(path).unwrap();
    }
}
