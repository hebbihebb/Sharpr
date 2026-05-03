# GTK4 & Libadwaita in Rust: Best Practices

Specific rules for the Sharpr project architecture.

## 1. Threading & Concurrency
- **Main Thread Only:** GTK widgets and objects (`gtk::*`, `adw::*`, `gdk::*`) MUST stay on the main thread.
- **No Sync Primitives on Widgets:** Do NOT wrap GTK objects in `Arc<Mutex<T>>`.
- **Background Work:** Use `std::thread::spawn` for heavy computation (I/O, decoding, AI).
- **Communication:** Use `async_channel` to send results from workers to the main thread.
- **UI Dispatch:** Use `glib::MainContext::spawn_local` or `glib::idle_add_local_once` for UI updates from the main thread loop.

## 2. Widget Subclassing (`mod imp`)
Always follow the `gtk-rs` subclassing pattern:

```rust
mod imp {
    use super::*;
    use glib::subclass::prelude::*;

    #[derive(Default)]
    pub struct MyWidget {
        // Internal state (use RefCell/Cell for interior mutability)
        pub some_data: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MyWidget {
        const NAME: &'static str = "SharprMyWidget";
        type Type = super::MyWidget;
        type ParentType = adw::Bin; // or gtk::Widget, etc.
    }

    impl ObjectImpl for MyWidget {}
    impl WidgetImpl for MyWidget {}
    impl BinImpl for MyWidget {}
}

glib::wrapper! {
    pub struct MyWidget(ObjectSubclass<imp::MyWidget>)
        @extends gtk::Widget, adw::Bin;
}
```

## 3. Properties and Signals
- Define properties in the `imp` module using the `#[property]` macro if available or standard `GObject` property boilerplate.
- Ensure all public methods on the wrapper delegate to the `imp` struct via `self.imp()`.
