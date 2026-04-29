use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use gdk4::Texture;
use glib::prelude::*;
use glib::Properties;
use gtk4::subclass::prelude::*;

// ---------------------------------------------------------------------------
// GObject subclass
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    #[derive(Properties)]
    #[properties(wrapper_type = super::ImageEntry)]
    pub struct ImageEntry {
        pub path: RefCell<PathBuf>,
        pub filename: RefCell<String>,
        #[property(get, set, nullable)]
        pub thumbnail: RefCell<Option<Texture>>,
        /// Width × height in pixels, populated lazily.
        pub width: Cell<u32>,
        pub height: Cell<u32>,
        /// File size in bytes, populated lazily.
        pub file_size: Cell<u64>,
        /// Raw Laplacian variance from the sharpness scorer.
        /// `f64::NAN` means not yet computed.
        pub sharpness_score: Cell<f64>,
    }

    impl Default for ImageEntry {
        fn default() -> Self {
            Self {
                path: RefCell::default(),
                filename: RefCell::default(),
                thumbnail: RefCell::default(),
                width: Cell::new(0),
                height: Cell::new(0),
                file_size: Cell::new(0),
                sharpness_score: Cell::new(f64::NAN),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImageEntry {
        const NAME: &'static str = "SharprImageEntry";
        type Type = super::ImageEntry;
        type ParentType = glib::Object;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ImageEntry {}
}

// ---------------------------------------------------------------------------
// Public type + convenience API
// ---------------------------------------------------------------------------

glib::wrapper! {
    pub struct ImageEntry(ObjectSubclass<imp::ImageEntry>);
}

impl ImageEntry {
    pub fn new(path: PathBuf) -> Self {
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let entry: Self = glib::Object::new();
        {
            let imp = entry.imp();
            *imp.path.borrow_mut() = path;
            *imp.filename.borrow_mut() = filename;
        }
        entry
    }

    pub fn path(&self) -> PathBuf {
        self.imp().path.borrow().clone()
    }

    pub fn filename(&self) -> String {
        self.imp().filename.borrow().clone()
    }

    pub fn dimensions(&self) -> Option<(u32, u32)> {
        let w = self.imp().width.get();
        let h = self.imp().height.get();
        if w > 0 && h > 0 {
            Some((w, h))
        } else {
            None
        }
    }

    pub fn set_dimensions(&self, width: u32, height: u32) {
        self.imp().width.set(width);
        self.imp().height.set(height);
    }

    pub fn file_size(&self) -> u64 {
        self.imp().file_size.get()
    }

    pub fn set_file_size(&self, size: u64) {
        self.imp().file_size.set(size);
    }

    /// Returns the raw Laplacian variance if it has been computed, or `None`.
    pub fn sharpness_score(&self) -> Option<f64> {
        let v = self.imp().sharpness_score.get();
        if v.is_nan() {
            None
        } else {
            Some(v)
        }
    }

    pub fn set_sharpness_score(&self, score: f64) {
        self.imp().sharpness_score.set(score);
    }
}
