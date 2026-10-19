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
        let control = modifiers.contains(gdk4::ModifierType::CONTROL_MASK);
        let plain = !modifiers.intersects(
            gdk4::ModifierType::CONTROL_MASK
                | gdk4::ModifierType::ALT_MASK
                | gdk4::ModifierType::SUPER_MASK
                | gdk4::ModifierType::META_MASK
                | gdk4::ModifierType::SHIFT_MASK,
        );
        let candidates = &self.imp().candidates;
        if candidates.has_keyboard_focus() {
            // L and R send the focused thumbnail to that side (D45).
            let side = match key {
                gdk4::Key::l if plain => Some(Side::Left),
                gdk4::Key::r if plain => Some(Side::Right),
                _ => None,
            };
            if let Some(side) = side {
                if let Some(number) = candidates.focused_number() {
                    self.choose(side, number);
                }
                return glib::Propagation::Stop;
            }
            if matches!(key, gdk4::Key::Left | gdk4::Key::Right | gdk4::Key::space) {
                return glib::Propagation::Proceed;
            }
        }
        let view = &self.imp().view;
        let failed = self.imp().failed.get();
        match key {
            // A side that couldn't be shown can't be kept.
            gdk4::Key::Left if plain => {
                if !failed[0] {
                    self.keep(Side::Left)
                }
            }
            gdk4::Key::Right if plain => {
                if !failed[1] {
                    self.keep(Side::Right)
                }
            }
            gdk4::Key::space if plain => {
                if view.mode() != Mode::Flip {
                    self.imp().mode_buttons.borrow()[2].set_active(true);
                } else {
                    view.flip();
                    self.update_sides();
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
        let mut failed = imp.failed.get();
        failed[side] = false;
        imp.failed.set(failed);
        let mut sizes = imp.sizes.get();
        sizes[side] = None;
        imp.sizes.set(sizes);
        let caption = &imp.captions[side];
        caption.error.set_visible(false);
        imp.keep[side].set_sensitive(true);
        self.mark_better();
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
        // Two images: the Keep bar; the last one: a line saying so.
        imp.keep_bar.set_visible(pair[1].is_some());
        imp.last_line.set_visible(pair[1].is_none());
        for (side, image) in pair.iter().enumerate() {
            let caption = &imp.captions[side];
            if let Some(path) = image {
                if imp.shown.borrow()[side].as_ref() != Some(path) {
                    caption.resolution.set_text("…");
                    caption.details.set_text("");
                }
                caption
                    .name
                    .set_text(&path.file_name().unwrap_or_default().to_string_lossy());
                let key = if side == 0 { "←" } else { "→" };
                imp.keep[side].set_tooltip_text(Some(&format!(
                    "Keep {} and dismiss the other ({key}); no files change",
                    path.display()
                )));
            }
            if imp.shown.borrow()[side] == *image {
                continue;
            }
            imp.shown.borrow_mut()[side] = image.clone();
            self.forget(side);
            if let Some(path) = image.clone() {
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
        self.update_sides();
    }

    /// Each image on screen shows its caption, and its error in its own
    /// half; in Flip only the shown image's.
    fn update_sides(&self) {
        let imp = self.imp();
        let view = &imp.view;
        let flipped = usize::from(view.flipped());
        let shown = imp.shown.borrow();
        for (side, caption) in imp.captions.iter().enumerate() {
            let on_screen = shown[side].is_some() && (view.mode() != Mode::Flip || side == flipped);
            caption.root.set_visible(on_screen);
            caption.column.set_visible(on_screen);
        }
    }

    /// The side with more pixels has its resolution marked (D45); equal or
    /// unknown sizes, or a side that couldn't be shown, mark nothing.
    fn mark_better(&self) {
        let imp = self.imp();
        let sizes = imp.sizes.get();
        let failed = imp.failed.get();
        let pixels = |side: usize| {
            sizes[side]
                .filter(|_| !failed[side])
                .map(|(w, h)| u64::from(w) * u64::from(h))
        };
        let better = match (pixels(0), pixels(1)) {
            (Some(a), Some(b)) if a > b => Some(0),
            (Some(a), Some(b)) if b > a => Some(1),
            _ => None,
        };
        for (side, caption) in imp.captions.iter().enumerate() {
            if better == Some(side) {
                caption.resolution.add_css_class("marked");
            } else {
                caption.resolution.remove_css_class("marked");
            }
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
            let decoded = decode_image(&worker_path, tier);
            // The summary reads the header and EXIF: off the GTK thread too.
            let text = (tier == Tier::Preview)
                .then(|| summary(&worker_path, decoded.as_ref().ok().map(|image| image.image)));
            let _ = tx.send_blocking((text, decoded));
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
            let caption = &imp.captions[side];
            if let Some((resolution, details, size)) = text {
                caption.resolution.set_text(&resolution);
                caption.details.set_text(&details);
                let mut sizes = imp.sizes.get();
                sizes[side] = size;
                imp.sizes.set(sizes);
            }
            match (result, job.tier) {
                (Ok(decoded), tier) => page.show(side, decoded, tier),
                (Err(message), Tier::Preview) => {
                    caption.resolution.set_text("Could not load");
                    caption.details.set_text("");
                    caption.error.set_text(&message);
                    caption.error.set_visible(true);
                    imp.held.borrow_mut()[side] = Held::Failed;
                    let mut failed = imp.failed.get();
                    failed[side] = true;
                    imp.failed.set(failed);
                    imp.keep[side].set_sensitive(false);
                }
                // The preview stays, and the full image is not asked for again.
                (Err(_), Tier::Full) => imp.held.borrow_mut()[side] = Held::Failed,
            }
            page.mark_better();
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

/// The caption under the filename: the resolution, then format and file
/// size, and the size to compare. The decoded size stands in when the
/// header can't be read (AVIF and HEIC).
fn summary(path: &Path, decoded: Option<(u32, u32)>) -> (String, String, Option<(u32, u32)>) {
    let size = crate::export::oriented_source_dimensions(path)
        .ok()
        .or(decoded);
    let resolution = size
        .map(|(w, h)| format!("{w} × {h}"))
        .unwrap_or_else(|| "size unknown".into());
    let mut details = format!(" · {}", compare_asset_format(path));
    if let Ok(metadata) = std::fs::metadata(path) {
        details.push_str(" · ");
        details.push_str(&crate::metadata::exif::file_size_text(metadata.len()));
    }
    (resolution, details, size)
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

/// One image's caption in its outer corner of the canvas (D45), and its
/// error, centred in its half.
pub(crate) struct Caption {
    pub root: gtk4::Box,
    pub name: gtk4::Label,
    /// Marked on the side with more pixels.
    pub resolution: gtk4::Label,
    pub details: gtk4::Label,
    /// This side's half of the canvas, holding the error.
    pub column: gtk4::Box,
    pub error: gtk4::Label,
}

impl Default for Caption {
    fn default() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
        root.add_css_class("sharpr-compare-caption");
        let name = gtk4::Label::new(None);
        name.add_css_class("sharpr-compare-caption-name");
        name.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        name.set_max_width_chars(36);
        let resolution = gtk4::Label::new(None);
        resolution.add_css_class("sharpr-compare-caption-resolution");
        let details = gtk4::Label::new(None);
        details.add_css_class("sharpr-compare-caption-details");
        let line = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        line.append(&resolution);
        line.append(&details);
        root.append(&name);
        root.append(&line);
        let error = gtk4::Label::new(None);
        error.add_css_class("sharpr-compare-error");
        error.set_wrap(true);
        error.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        error.set_max_width_chars(48);
        error.set_justify(gtk4::Justification::Center);
        error.set_visible(false);
        let column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        column.set_hexpand(true);
        column.set_valign(gtk4::Align::Center);
        column.set_halign(gtk4::Align::Center);
        column.append(&error);
        Self {
            root,
            name,
            resolution,
            details,
            column,
            error,
        }
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
        /// Per side: the caption in its outer corner, and its error.
        pub(crate) captions: [Caption; 2],
        pub keep: [gtk4::Button; 2],
        /// Keep Left and Keep Right, mirrored on the centre line.
        pub keep_bar: gtk4::Box,
        /// In the Keep bar's place once one image is left.
        pub last_line: gtk4::Label,
        /// Per side: the image couldn't be shown, so it can't be kept.
        pub failed: Cell<[bool; 2]>,
        /// Per side: the image's size, to mark the larger.
        pub sizes: Cell<[Option<(u32, u32)>; 2]>,
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
    }

    impl Default for ComparePage {
        fn default() -> Self {
            Self {
                root: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
                header: ModeControls::new(),
                view: CompareView::default(),
                knockout: RefCell::new(None),
                shown: RefCell::new([None, None]),
                captions: Default::default(),
                keep: [gtk4::Button::new(), gtk4::Button::new()],
                keep_bar: gtk4::Box::new(gtk4::Orientation::Horizontal, 0),
                last_line: gtk4::Label::new(Some("Last image left · Done (Esc) keeps it selected")),
                failed: Cell::new([false; 2]),
                sizes: Cell::new([None; 2]),
                counter: gtk4::Label::new(None),
                undo: gtk4::Button::from_icon_name("edit-undo-symbolic"),
                mode_buttons: RefCell::new(Vec::new()),
                close_cb: RefCell::new(None),
                epochs: Cell::new([0; 2]),
                gates: RefCell::new(Default::default()),
                held: RefCell::new([Held::Preview; 2]),
                previews: RefCell::new([None, None]),
                candidates: CandidateStrip::default(),
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
    // Each image's caption sits in its outer corner and its error in its
    // own half, over the canvas but never in the way of pan and zoom.
    let canvas = gtk4::Overlay::new();
    canvas.set_child(Some(&imp.view));
    let columns = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    columns.set_homogeneous(true);
    columns.set_can_target(false);
    for (side, caption) in imp.captions.iter().enumerate() {
        columns.append(&caption.column);
        let corner = if side == 0 {
            gtk4::Align::Start
        } else {
            gtk4::Align::End
        };
        caption.root.set_halign(corner);
        caption.root.set_valign(gtk4::Align::End);
        caption.root.set_margin_start(12);
        caption.root.set_margin_end(12);
        caption.root.set_margin_bottom(12);
        caption.root.set_can_target(false);
        caption.root.set_visible(false);
        caption.name.set_xalign(if side == 0 { 0.0 } else { 1.0 });
        if side == 1 {
            caption
                .root
                .last_child()
                .unwrap()
                .set_halign(gtk4::Align::End);
        }
    }
    canvas.add_overlay(&columns);
    for caption in &imp.captions {
        canvas.add_overlay(&caption.root);
    }
    imp.root.append(&canvas);
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
        let button = gtk4::ToggleButton::new();
        if mode == Mode::Flip {
            // Space flips: the key shows on the control it triggers.
            let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            content.append(&gtk4::Label::new(Some(label)));
            content.append(&crate::ui::look::keycap("Space"));
            button.set_child(Some(&content));
            button.update_property(&[gtk4::accessible::Property::Label(label)]);
        } else {
            button.set_label(label);
        }
        button.set_tooltip_text(Some(tooltip));
        if let Some(first) = buttons.first() {
            button.set_group(Some(first));
        }
        let weak = page.downgrade();
        button.connect_toggled(move |button| {
            if let (true, Some(page)) = (button.is_active(), weak.upgrade()) {
                page.imp().view.set_mode(mode);
                page.update_sides();
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

    // Keep Left and Keep Right mirrored on the canvas's centre line, each
    // showing its key (D45).
    let bar = &imp.keep_bar;
    bar.set_homogeneous(true);
    bar.set_margin_top(8);
    bar.set_margin_bottom(6);
    for (side, (key, text)) in [("←", "Keep Left"), ("→", "Keep Right")]
        .into_iter()
        .enumerate()
    {
        let keep = &imp.keep[side];
        let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let label = gtk4::Label::new(Some(text));
        if side == 0 {
            content.append(&crate::ui::look::keycap(key));
            content.append(&label);
        } else {
            content.append(&label);
            content.append(&crate::ui::look::keycap(key));
        }
        keep.set_child(Some(&content));
        keep.add_css_class("sharpr-compare-keep");
        keep.update_property(&[gtk4::accessible::Property::Label(text)]);
        let weak = page.downgrade();
        let which = if side == 0 { Side::Left } else { Side::Right };
        keep.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.keep(which);
            }
        });
        let cell = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        if side == 0 {
            keep.set_halign(gtk4::Align::End);
            keep.set_margin_end(6);
        } else {
            keep.set_halign(gtk4::Align::Start);
            keep.set_margin_start(6);
        }
        keep.set_hexpand(true);
        cell.append(keep);
        bar.append(&cell);
    }
    imp.root.append(bar);
    let last = &imp.last_line;
    last.add_css_class("sharpr-compare-last");
    last.set_margin_top(8);
    last.set_margin_bottom(6);
    last.set_visible(false);
    imp.root.append(last);
    imp.candidates.build(page, &imp.counter);
    imp.root.append(&imp.candidates.root);
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

    /// Work Order item 32: AVIF and HEIC open in Compare through glycin,
    /// and the summary gives their size though the header can't be read.
    #[test]
    fn avif_and_heic_decode_with_their_size() {
        for (mime, name) in [("image/avif", "avif"), ("image/heif", "heic")] {
            let path = test_path("green").with_extension(name);
            crate::visual_search::prepare::encode_with_glycin(
                mime,
                &path,
                (1600, 900),
                [20, 200, 40],
            );
            let full = decode_image(&path, Tier::Full).unwrap();
            assert_eq!((full.width, full.height), (1600, 900), "{name}");
            let middle = ((450 * 1600 + 800) * 4) as usize;
            let [r, g, b, _] = full.pixels[middle..middle + 4] else {
                unreachable!()
            };
            assert!(g > 150 && r < 80 && b < 90, "{name}: {r},{g},{b}");
            let preview = decode_image(&path, Tier::Preview).unwrap();
            assert_eq!(preview.image, (1600, 900), "{name}");
            let (resolution, details, size) = summary(&path, Some(preview.image));
            assert_eq!(resolution, "1600 × 900", "{name}");
            assert_eq!(size, Some((1600, 900)), "{name}");
            assert!(details.starts_with(" · "), "{name}: {details}");
            std::fs::remove_file(path).unwrap();
        }
    }
}
