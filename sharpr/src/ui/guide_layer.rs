//! The viewer's 16:9 guide layer (D36, D38). It draws with GTK snapshot
//! nodes, which the GPU composites, so moving or resizing the guide redraws a
//! few rectangles instead of repainting a viewer-sized Cairo surface on the
//! processor for every pointer movement.

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::RefCell;

type Draw = Box<dyn Fn(&gtk4::Snapshot)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct GuideLayer {
        pub draw: RefCell<Option<Draw>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GuideLayer {
        const NAME: &'static str = "SharprGuideLayer";
        type Type = super::GuideLayer;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for GuideLayer {}

    impl WidgetImpl for GuideLayer {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            if let Some(draw) = self.draw.borrow().as_ref() {
                draw(snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct GuideLayer(ObjectSubclass<imp::GuideLayer>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl Default for GuideLayer {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl GuideLayer {
    pub fn set_draw(&self, draw: impl Fn(&gtk4::Snapshot) + 'static) {
        *self.imp().draw.borrow_mut() = Some(Box::new(draw));
        self.queue_draw();
    }
}
