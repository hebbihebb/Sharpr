use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::glib;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use crate::upscale::BeforeAfterViewer;

pub(crate) struct SlotData {
    id: u64,
    source: PathBuf,
    output: PathBuf,
    #[allow(dead_code)]
    label: String,
    container: gtk4::Box, // the outer widget in the filmstrip
}

glib::wrapper! {
    pub struct ComparePage(ObjectSubclass<imp::ComparePage>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl ComparePage {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn push_pair(&self, source: PathBuf, output: PathBuf, label: String) {
        let id = self.imp().next_id.get();
        self.imp().next_id.set(id + 1);

        let widget = self.build_slot_widget(id, &source, &output, &label);
        if let Some(filmstrip_box) = self.imp().filmstrip_box.borrow().as_ref() {
            filmstrip_box.append(&widget);
        }

        self.imp().slots.borrow_mut().push(SlotData {
            id,
            source,
            output,
            label,
            container: widget,
        });

        self.select_slot(id);
    }

    fn build_slot_widget(&self, id: u64, source: &Path, output: &Path, label: &str) -> gtk4::Box {
        let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let slot_btn = gtk4::Button::new();
        slot_btn.add_css_class("flat");
        slot_btn.set_width_request(104);

        let btn_content = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        btn_content.set_margin_top(4);
        btn_content.set_margin_bottom(4);
        btn_content.set_margin_start(4);
        btn_content.set_margin_end(4);

        let thumb_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        let src_pic = gtk4::Picture::new();
        src_pic.set_size_request(50, 70);
        src_pic.set_content_fit(gtk4::ContentFit::Cover);

        let out_pic = gtk4::Picture::new();
        out_pic.set_size_request(50, 70);
        out_pic.set_content_fit(gtk4::ContentFit::Cover);

        thumb_row.append(&src_pic);
        thumb_row.append(&out_pic);

        let lbl = gtk4::Label::new(Some(label));
        lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        lbl.set_max_width_chars(12);
        lbl.add_css_class("caption");

        btn_content.append(&thumb_row);
        btn_content.append(&lbl);
        slot_btn.set_child(Some(&btn_content));

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.set_halign(gtk4::Align::Center);
        // Softer appearance as requested
        close_btn.add_css_class("dim-label");

        outer.append(&slot_btn);
        outer.append(&close_btn);

        // Load thumbnails
        let source_path = source.to_path_buf();
        let src_pic_c = src_pic.clone();
        glib::spawn_future_local(async move {
            if let Ok(tex) = load_thumbnail_for_slot(&source_path).await {
                src_pic_c.set_paintable(Some(&tex));
            }
        });

        let output_path = output.to_path_buf();
        let out_pic_c = out_pic.clone();
        glib::spawn_future_local(async move {
            if let Ok(tex) = load_thumbnail_for_slot(&output_path).await {
                out_pic_c.set_paintable(Some(&tex));
            }
        });

        let self_weak = self.downgrade();
        slot_btn.connect_clicked(move |_| {
            if let Some(s) = self_weak.upgrade() {
                s.select_slot(id);
            }
        });

        let self_weak2 = self.downgrade();
        close_btn.connect_clicked(move |_| {
            if let Some(s) = self_weak2.upgrade() {
                s.remove_slot(id);
            }
        });

        outer
    }

    fn select_slot(&self, id: u64) {
        let imp = self.imp();
        
        // Deactivate old active slot
        if let Some(old_id) = imp.active_id.get() {
            if let Some(slot) = imp.slots.borrow().iter().find(|s| s.id == old_id) {
                if let Some(btn) = slot.container.first_child().and_downcast::<gtk4::Button>() {
                    btn.remove_css_class("suggested-action");
                }
            }
        }

        imp.active_id.set(Some(id));

        // Find and activate new slot
        let slot_info = {
            let slots = imp.slots.borrow();
            slots.iter().find(|s| s.id == id).map(|s| {
                (s.source.clone(), s.output.clone(), s.container.clone())
            })
        };

        if let Some((source, output, container)) = slot_info {
            if let Some(btn) = container.first_child().and_downcast::<gtk4::Button>() {
                btn.add_css_class("suggested-action");
            }

            if let Some(stack) = imp.content_stack.borrow().as_ref() {
                stack.set_visible_child_name("viewer");
            }

            if let Some(viewer) = imp.viewer.borrow().as_ref() {
                viewer.load(source, output, || {});
            }
        }
    }

    fn remove_slot(&self, id: u64) {
        let imp = self.imp();
        let mut slots = imp.slots.borrow_mut();
        
        if let Some(idx) = slots.iter().position(|s| s.id == id) {
            let slot = slots.remove(idx);
            if let Some(filmstrip_box) = imp.filmstrip_box.borrow().as_ref() {
                filmstrip_box.remove(&slot.container);
            }

            if imp.active_id.get() == Some(id) {
                if slots.is_empty() {
                    imp.active_id.set(None);
                    if let Some(viewer) = imp.viewer.borrow().as_ref() {
                        viewer.clear();
                    }
                    if let Some(stack) = imp.content_stack.borrow().as_ref() {
                        stack.set_visible_child_name("empty");
                    }
                } else {
                    let next_id = slots[0].id;
                    drop(slots);
                    self.select_slot(next_id);
                }
            }
        }
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ComparePage {
        pub content_stack: RefCell<Option<gtk4::Stack>>,
        pub viewer: RefCell<Option<BeforeAfterViewer>>,
        pub(crate) filmstrip_box: RefCell<Option<gtk4::Box>>,
        pub(crate) slots: RefCell<Vec<SlotData>>,
        pub next_id: Cell<u64>,
        pub active_id: Cell<Option<u64>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ComparePage {
        const NAME: &'static str = "ComparePage";
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

            let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            root_box.set_parent(&*obj);

            let content_stack = gtk4::Stack::new();
            content_stack.set_vexpand(true);
            content_stack.set_hexpand(true);

            let empty_label = gtk4::Label::new(Some("Click Compare on a history entry to start"));
            empty_label.add_css_class("dim-label");
            empty_label.set_halign(gtk4::Align::Center);
            empty_label.set_valign(gtk4::Align::Center);
            content_stack.add_named(&empty_label, Some("empty"));

            let viewer = BeforeAfterViewer::new();
            content_stack.add_named(&viewer, Some("viewer"));

            root_box.append(&content_stack);

            let filmstrip_panel = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            
            let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            filmstrip_panel.append(&sep);

            let title_lbl = gtk4::Label::new(Some("Compare"));
            title_lbl.set_halign(gtk4::Align::Start);
            title_lbl.add_css_class("dim-label");
            title_lbl.add_css_class("caption");
            title_lbl.set_margin_start(8);
            filmstrip_panel.append(&title_lbl);

            let scroll = gtk4::ScrolledWindow::new();
            scroll.set_hexpand(true);
            scroll.set_height_request(110);
            scroll.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Never);

            let filmstrip_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            filmstrip_box.set_margin_top(6);
            filmstrip_box.set_margin_bottom(6);
            filmstrip_box.set_margin_start(6);
            filmstrip_box.set_margin_end(6);
            scroll.set_child(Some(&filmstrip_box));
            filmstrip_panel.append(&scroll);

            root_box.append(&filmstrip_panel);

            content_stack.set_visible_child_name("empty");

            *self.content_stack.borrow_mut() = Some(content_stack);
            *self.viewer.borrow_mut() = Some(viewer);
            *self.filmstrip_box.borrow_mut() = Some(filmstrip_box);
        }

        fn dispose(&self) {
            if let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for ComparePage {}
}

async fn load_thumbnail_for_slot(path: &Path) -> Result<gdk4::Texture, String> {
    let path = path.to_path_buf();
    let (tx, rx) = async_channel::bounded::<Result<gdk4::Texture, String>>(1);
    
    std::thread::spawn(move || {
        let result = (|| -> Result<gdk4::Texture, String> {
            let img = image::open(&path).map_err(|e| e.to_string())?;
            let thumb = img.thumbnail(100, 140);
            let rgb = thumb.to_rgba8();
            let bytes = glib::Bytes::from(rgb.as_raw());
            Ok(gdk4::MemoryTexture::new(
                rgb.width() as i32,
                rgb.height() as i32,
                gdk4::MemoryFormat::R8g8b8a8,
                &bytes,
                (rgb.width() * 4) as usize,
            ).upcast())
        })();
        let _ = tx.send_blocking(result);
    });

    rx.recv().await.unwrap_or_else(|_| Err("Thumbnail thread died".to_string()))
}

