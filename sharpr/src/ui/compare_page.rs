use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::metadata::orientation::apply_exif_orientation;
use crate::ui::compare_item::{CompareAssetInfo, CompareItem};

type CloseCallback = Box<dyn Fn()>;

glib::wrapper! {
    pub struct ComparePage(ObjectSubclass<imp::ComparePage>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl ComparePage {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_close_cb<F: Fn() + 'static>(&self, callback: F) {
        *self.imp().close_cb.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_comparison(&self, item: CompareItem) {
        let generation = self.imp().load_generation.get().wrapping_add(1);
        self.imp().load_generation.set(generation);
        set_asset_summary(&self.imp().left_summary, &item.left);
        set_asset_summary(&self.imp().right_summary, &item.right);
        self.imp().left_stack.set_visible_child_name("loading");
        self.imp().right_stack.set_visible_child_name("loading");

        let (tx, rx) = async_channel::bounded(2);
        spawn_decode(false, item.left.path, tx.clone());
        spawn_decode(true, item.right.path, tx);

        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            for _ in 0..2 {
                let Ok((right, result)) = rx.recv().await else {
                    break;
                };
                let Some(page) = weak.upgrade() else {
                    break;
                };
                if page.imp().load_generation.get() != generation {
                    break;
                }
                page.apply_decode_result(right, result);
            }
        });
    }

    pub fn clear(&self) {
        self.imp()
            .load_generation
            .set(self.imp().load_generation.get().wrapping_add(1));
        self.imp()
            .left_picture
            .set_paintable(None::<&gdk4::Paintable>);
        self.imp()
            .right_picture
            .set_paintable(None::<&gdk4::Paintable>);
    }

    fn apply_decode_result(&self, right: bool, result: Result<DecodedImage, String>) {
        let (picture, stack, error) = if right {
            (
                &self.imp().right_picture,
                &self.imp().right_stack,
                &self.imp().right_error,
            )
        } else {
            (
                &self.imp().left_picture,
                &self.imp().left_stack,
                &self.imp().left_error,
            )
        };
        match result {
            Ok(decoded) => {
                let bytes = glib::Bytes::from_owned(decoded.pixels);
                let texture: gdk4::Texture = gdk4::MemoryTexture::new(
                    decoded.width,
                    decoded.height,
                    gdk4::MemoryFormat::R8g8b8a8,
                    &bytes,
                    decoded.width as usize * 4,
                )
                .upcast();
                picture.set_paintable(Some(&texture));
                stack.set_visible_child_name("image");
            }
            Err(message) => {
                error.set_description(Some(&message));
                stack.set_visible_child_name("error");
            }
        }
    }
}

impl Default for ComparePage {
    fn default() -> Self {
        Self::new()
    }
}

struct DecodedImage {
    pixels: Vec<u8>,
    width: i32,
    height: i32,
}

fn spawn_decode(
    right: bool,
    path: PathBuf,
    tx: async_channel::Sender<(bool, Result<DecodedImage, String>)>,
) {
    std::thread::spawn(move || {
        let result = decode_image(&path);
        let _ = tx.send_blocking((right, result));
    });
}

fn decode_image(path: &Path) -> Result<DecodedImage, String> {
    let image = if crate::jxl::is_jxl_path(path) {
        crate::jxl::decode_path(path)
    } else {
        image::open(path).map_err(|error| error.to_string())
    }
    .map_err(|error| format!("Could not load {}: {error}", path.display()))?;
    let rgba = apply_exif_orientation(image, path).to_rgba8();
    Ok(DecodedImage {
        width: rgba.width() as i32,
        height: rgba.height() as i32,
        pixels: rgba.into_raw(),
    })
}

fn set_asset_summary(label: &gtk4::Label, asset: &CompareAssetInfo) {
    let name = asset
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| asset.path.display().to_string());
    let dimensions = asset
        .dimensions
        .map(|(width, height)| format!("{width} × {height}"))
        .unwrap_or_else(|| "Unknown size".to_string());
    label.set_text(&format!("{name} · {} · {dimensions}", asset.format));
}

fn image_stack() -> (gtk4::Stack, gtk4::Picture, libadwaita::StatusPage) {
    let stack = gtk4::Stack::new();
    let picture = gtk4::Picture::new();
    picture.set_content_fit(gtk4::ContentFit::Contain);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    let loading = gtk4::Spinner::new();
    loading.set_spinning(true);
    let error = libadwaita::StatusPage::builder()
        .icon_name("image-missing-symbolic")
        .title("Image unavailable")
        .build();
    stack.add_named(&picture, Some("image"));
    stack.add_named(&loading, Some("loading"));
    stack.add_named(&error, Some("error"));
    (stack, picture, error)
}

mod imp {
    use super::*;

    pub struct ComparePage {
        pub root: gtk4::Overlay,
        pub left_stack: gtk4::Stack,
        pub right_stack: gtk4::Stack,
        pub left_picture: gtk4::Picture,
        pub right_picture: gtk4::Picture,
        pub left_error: libadwaita::StatusPage,
        pub right_error: libadwaita::StatusPage,
        pub left_summary: gtk4::Label,
        pub right_summary: gtk4::Label,
        pub close_cb: RefCell<Option<CloseCallback>>,
        pub load_generation: Cell<u64>,
    }

    impl Default for ComparePage {
        fn default() -> Self {
            let (left_stack, left_picture, left_error) = image_stack();
            let (right_stack, right_picture, right_error) = image_stack();
            Self {
                root: gtk4::Overlay::new(),
                left_stack,
                right_stack,
                left_picture,
                right_picture,
                left_error,
                right_error,
                left_summary: gtk4::Label::new(None),
                right_summary: gtk4::Label::new(None),
                close_cb: RefCell::new(None),
                load_generation: Cell::new(0),
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
            let obj = self.obj();
            self.root.set_parent(&*obj);

            let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
            paned.set_wide_handle(true);
            paned.set_start_child(Some(&self.left_stack));
            paned.set_end_child(Some(&self.right_stack));
            paned.set_position(600);
            self.root.set_child(Some(&paned));

            let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
            toolbar.add_css_class("osd");
            toolbar.set_halign(gtk4::Align::Center);
            toolbar.set_valign(gtk4::Align::Start);
            toolbar.set_margin_top(16);
            toolbar.set_margin_start(16);
            toolbar.set_margin_end(16);
            self.left_summary
                .set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            self.right_summary
                .set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            toolbar.append(&self.left_summary);
            toolbar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
            toolbar.append(&self.right_summary);
            let close = gtk4::Button::from_icon_name("window-close-symbolic");
            close.add_css_class("flat");
            close.set_tooltip_text(Some("Close Compare"));
            let weak = obj.downgrade();
            close.connect_clicked(move |_| {
                if let Some(page) = weak.upgrade() {
                    if let Some(callback) = page.imp().close_cb.borrow().as_ref() {
                        callback();
                    }
                }
            });
            toolbar.append(&close);
            self.root.add_overlay(&toolbar);
        }

        fn dispose(&self) {
            self.root.unparent();
        }
    }

    impl WidgetImpl for ComparePage {}
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
