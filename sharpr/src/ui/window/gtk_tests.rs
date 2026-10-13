//! Run explicitly with isolated data/settings and a graphical session.
use super::*;

fn isolated_fixture(case: &str) -> PathBuf {
    assert_eq!(std::env::var("GSETTINGS_BACKEND").unwrap(), "memory");
    let fixtures = PathBuf::from(std::env::var("SHARPR_GTK_FIXTURE").unwrap());
    let parent = fixtures.parent().expect("isolated fixture parent");
    assert!(parent.is_absolute());
    assert!(
        parent.starts_with(std::env::temp_dir())
            || parent.components().any(|c| c.as_os_str() == ".tmp")
    );
    let unique = format!("{case}-{}", std::process::id());
    for variable in ["XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"] {
        let base = PathBuf::from(std::env::var(variable).expect("explicit isolated XDG directory"));
        assert!(
            base.starts_with(parent),
            "XDG directories must be beside the fixtures"
        );
        let directory = base.join(&unique);
        std::fs::create_dir_all(&directory).unwrap();
        // Nextest runs each GTK test in its own process, before GTK/GIO starts.
        std::env::set_var(variable, directory);
    }
    fixtures.join(unique)
}

fn pump_until(mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "GTK operation did not complete"
        );
        let context = glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Whether the bound row for `path` shows its index pill with `class`.
fn pill_has_class(widget: &gtk4::Widget, path: &Path, class: &str) -> bool {
    fn has_pill(widget: &gtk4::Widget, class: &str) -> bool {
        if widget.has_css_class("filmstrip-index-label") {
            return widget.has_css_class(class);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if has_pill(&current, class) {
                return true;
            }
            child = current.next_sibling();
        }
        false
    }
    let row_path = unsafe {
        widget
            .data::<PathBuf>("fs-path")
            .map(|p| p.as_ref().clone())
    };
    if row_path.as_deref() == Some(path) {
        return has_pill(widget, class);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if pill_has_class(&current, path, class) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

fn screenshot(window: &SharprWindow, path: &Path) {
    let paintable = gtk4::WidgetPaintable::new(Some(window));
    let snapshot = gtk4::Snapshot::new();
    paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
    let node = snapshot
        .to_node()
        .expect("realized window has a render node");
    let renderer = window.renderer().expect("realized window has a renderer");
    renderer
        .render_texture(&node, None)
        .save_to_png(path)
        .unwrap();
}

/// Renders a single widget, such as a popover, with the window's renderer.
fn screenshot_widget(window: &SharprWindow, widget: &gtk4::Widget, path: &Path) {
    let paintable = gtk4::WidgetPaintable::new(Some(widget));
    let snapshot = gtk4::Snapshot::new();
    paintable.snapshot(&snapshot, widget.width() as f64, widget.height() as f64);
    let node = snapshot
        .to_node()
        .expect("visible widget has a render node");
    let renderer = window.renderer().expect("realized window has a renderer");
    renderer
        .render_texture(&node, None)
        .save_to_png(path)
        .unwrap();
}

#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn scoped_search_restores_folder_and_bucket_and_crop_dialog_renders() {
    let root = isolated_fixture("search-copy");
    std::fs::create_dir_all(&root).unwrap();
    for (name, (w, h)) in [("mountain.png", (160, 90)), ("sea.png", (160, 120))] {
        let mut image = image::RgbImage::new(w, h);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgb([x as u8, y as u8, 150]);
        }
        image.save(root.join(name)).unwrap();
    }
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let mut settings = AppSettings::load();
    settings.libraries.clear();
    settings
        .add_library("Test", root.clone(), FolderMode::TopLevel)
        .unwrap();
    settings.set_active_library_last_folder(Some(root.clone()));
    let app = libadwaita::Application::builder()
        .application_id("io.github.hebbihebb.Sharpr.GtkTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let window = SharprWindow::new(&app);
    window.set_default_size(1024, 600);
    window.present();
    pump_until(|| window.imp().state.borrow().library.image_count() == 2 && window.is_mapped());
    let sea = root.join("sea.png");
    let index = window
        .imp()
        .state
        .borrow()
        .library
        .index_of_path(&sea)
        .unwrap();
    window
        .imp()
        .filmstrip
        .borrow()
        .as_ref()
        .unwrap()
        .navigate_to(index);
    let entry = window.imp().inline_search_entry.borrow().clone().unwrap();
    window.show_inline_search();
    entry.set_text("mountain");
    pump_until(|| {
        window.imp().state.borrow().library.image_count() == 1
            && window
                .imp()
                .state
                .borrow()
                .library
                .entry_at(0)
                .unwrap()
                .path()
                .ends_with("mountain.png")
    });
    entry.set_text("sea");
    pump_until(|| {
        window.imp().state.borrow().library.image_count() == 1
            && window
                .imp()
                .state
                .borrow()
                .library
                .entry_at(0)
                .unwrap()
                .path()
                == sea
    });
    entry.set_text("");
    pump_until(|| window.imp().state.borrow().library.image_count() == 2);
    assert_eq!(
        window.imp().state.borrow().scope,
        ViewScope::Folder(root.clone())
    );
    assert_eq!(
        window.imp().state.borrow().library.current_folder,
        Some(root.clone())
    );
    assert_eq!(
        window
            .imp()
            .state
            .borrow()
            .library
            .entry_at(window.imp().state.borrow().library.selected_index.unwrap())
            .unwrap()
            .path(),
        sea
    );
    entry.set_text("sea");
    pump_until(|| window.imp().state.borrow().library.image_count() == 1);
    window.imp().state.borrow_mut().sort_order = crate::model::library::SortOrder::NameDesc;
    window.run_browse_query("sea".into(), true);
    entry.set_text("");
    pump_until(|| {
        let st = window.imp().state.borrow();
        st.library.image_count() == 2 && st.library.entry_at(0).is_some_and(|e| e.path() == sea)
    });
    // Exercise the exact Escape operation order, not only clearing text.
    entry.set_text("sea");
    pump_until(|| window.imp().state.borrow().library.image_count() == 1);
    window.imp().state.borrow_mut().sort_order = crate::model::library::SortOrder::NameAsc;
    window.run_browse_query("sea".into(), true);
    window.close_browse_search();
    pump_until(|| {
        let st = window.imp().state.borrow();
        st.library.image_count() == 2
            && st
                .library
                .entry_at(0)
                .is_some_and(|e| e.path().ends_with("mountain.png"))
    });
    // Typing part of a filter word offers completions (decision D16).
    window.show_inline_search();
    entry.set_text("4");
    pump_until(|| {
        window
            .imp()
            .search_suggestions
            .borrow()
            .as_ref()
            .is_some_and(|s| s.is_visible() && s.first_child().is_some())
    });
    let settle = std::time::Instant::now();
    pump_until(|| settle.elapsed() > std::time::Duration::from_millis(400));
    screenshot(
        &window,
        &root.parent().unwrap().join("search-suggestions.png"),
    );
    entry.set_text("");
    pump_until(|| window.imp().state.borrow().library.image_count() == 2);
    // Finished filter words become chips and narrow the view (decision D16).
    window.show_inline_search();
    entry.set_text("png 16:9 ");
    pump_until(|| {
        let st = window.imp().state.borrow();
        st.library.image_count() == 1
            && st
                .library
                .entry_at(0)
                .is_some_and(|e| e.path().ends_with("mountain.png"))
    });
    assert_eq!(entry.text().as_str(), "");
    assert_eq!(window.imp().browse_options.borrow().chips.chips.len(), 2);
    let settle = std::time::Instant::now();
    pump_until(|| settle.elapsed() > std::time::Duration::from_millis(400));
    screenshot(&window, &root.parent().unwrap().join("search-chips.png"));
    // Ctrl+I's Info panel opens once the image's details are loaded (D18).
    let viewer = window.imp().viewer.borrow().clone().unwrap();
    viewer.show_info();
    let info = viewer.imp().metadata_chip.imp().popover.clone();
    pump_until(|| info.is_visible());
    let settle = std::time::Instant::now();
    pump_until(|| settle.elapsed() > std::time::Duration::from_millis(300));
    screenshot_widget(
        &window,
        info.upcast_ref(),
        &root.parent().unwrap().join("info-panel.png"),
    );
    info.popdown();
    window
        .imp()
        .browse_options
        .borrow_mut()
        .chips
        .chips
        .retain(|chip| chip.label() != "16:9");
    window.rebuild_search_chips();
    window.run_scoped_search(String::new());
    pump_until(|| window.imp().state.borrow().library.image_count() == 2);
    window.close_browse_search();
    pump_until(|| window.imp().state.borrow().library.image_count() == 2);
    assert!(window.imp().browse_options.borrow().chips.is_empty());
    let bucket = {
        let st = window.imp().state.borrow();
        let bucket = st
            .library_index
            .as_ref()
            .unwrap()
            .create_collection(
                st.settings.active_library_id.as_deref().unwrap(),
                None,
                "Test bucket",
                &[],
                None,
                None,
            )
            .unwrap();
        st.tags
            .as_ref()
            .unwrap()
            .change_tags_checked(
                &[root.join("mountain.png"), sea.clone()],
                std::slice::from_ref(&bucket.primary_tag),
                false,
            )
            .unwrap();
        bucket.id
    };
    // Open the bucket the way a sidebar click does: membership loads on a
    // worker. Narrow the folder view first so the bucket's two images prove it.
    window.show_inline_search();
    entry.set_text("sea");
    pump_until(|| window.imp().state.borrow().library.image_count() == 1);
    window
        .imp()
        .sidebar
        .borrow()
        .as_ref()
        .unwrap()
        .emit_collection_selected(bucket);
    pump_until(|| {
        let st = window.imp().state.borrow();
        st.scope == ViewScope::Collection(bucket) && st.library.image_count() == 2
    });
    window.show_inline_search();
    window.imp().state.borrow_mut().scope = ViewScope::Collection(bucket);
    entry.set_text("mountain");
    pump_until(|| window.imp().state.borrow().library.image_count() == 1);
    entry.set_text("");
    pump_until(|| window.imp().state.borrow().library.image_count() == 2);
    assert_eq!(
        window.imp().state.borrow().scope,
        ViewScope::Collection(bucket)
    );
    // Searching before a bucket's members arrive must search that bucket, not
    // the view still on screen, and clearing must show the bucket.
    let second = {
        let st = window.imp().state.borrow();
        let second = st
            .library_index
            .as_ref()
            .unwrap()
            .create_collection(
                st.settings.active_library_id.as_deref().unwrap(),
                None,
                "Second bucket",
                &[],
                None,
                None,
            )
            .unwrap();
        st.tags
            .as_ref()
            .unwrap()
            .change_tags_checked(
                &[root.join("mountain.png")],
                std::slice::from_ref(&second.primary_tag),
                false,
            )
            .unwrap();
        second.id
    };
    let sidebar = window.imp().sidebar.borrow().clone().unwrap();
    sidebar.emit_collection_selected(second);
    window.show_inline_search();
    entry.set_text("sea");
    window.run_scoped_search("sea".into());
    pump_until(|| {
        let st = window.imp().state.borrow();
        window.imp().bucket_open_pending.get().is_none()
            && window.imp().browse_snapshot.borrow().is_some()
            && st.library.image_count() == 0
    });
    let ready = std::time::Instant::now() + std::time::Duration::from_millis(300);
    pump_until(|| std::time::Instant::now() > ready);
    assert_eq!(window.imp().state.borrow().library.image_count(), 0);
    entry.set_text("");
    pump_until(|| window.imp().state.borrow().library.image_count() == 1);
    {
        let st = window.imp().state.borrow();
        assert_eq!(st.scope, ViewScope::Collection(second));
        assert!(st
            .library
            .entry_at(0)
            .unwrap()
            .path()
            .ends_with("mountain.png"));
    }
    sidebar.emit_collection_selected(bucket);
    pump_until(|| {
        let st = window.imp().state.borrow();
        st.scope == ViewScope::Collection(bucket) && st.library.image_count() == 2
    });
    // A visible row takes a new bucket colour without being scrolled away.
    {
        let st = window.imp().state.borrow();
        let index = st.library_index.as_ref().unwrap();
        let red = index
            .create_collection(
                st.settings.active_library_id.as_deref().unwrap(),
                None,
                "Red bucket",
                &[],
                Some("#e01b24"),
                None,
            )
            .unwrap();
        st.tags
            .as_ref()
            .unwrap()
            .change_tags_checked(
                &[root.join("mountain.png")],
                std::slice::from_ref(&red.primary_tag),
                false,
            )
            .unwrap();
        let filmstrip = window.imp().filmstrip.borrow().clone().unwrap();
        filmstrip.refresh_collection_colors(&index.list_collections().unwrap());
    }
    let list_view = window
        .imp()
        .filmstrip
        .borrow()
        .as_ref()
        .unwrap()
        .imp()
        .list_view
        .clone();
    pump_until(|| {
        pill_has_class(
            list_view.upcast_ref(),
            &root.join("mountain.png"),
            "filmstrip-pill-color-e01b24",
        )
    });
    window.imp().state.borrow_mut().scope = ViewScope::Folder(root.clone());
    window.imp().browse_options.borrow_mut().dimensions = crate::model::browse::DimensionFilters {
        aspect: crate::model::browse::AspectFilter::WideScreen,
        min_width: 160,
        min_height: 90,
    };
    window.run_scoped_search(String::new());
    pump_until(|| window.imp().state.borrow().library.image_count() == 1);
    assert!(window
        .imp()
        .state
        .borrow()
        .library
        .entry_at(0)
        .unwrap()
        .path()
        .ends_with("mountain.png"));
    window.imp().browse_options.borrow_mut().dimensions = Default::default();
    window.run_scoped_search(String::new());
    assert_eq!(window.imp().state.borrow().library.image_count(), 2);
    window.clear_inline_search(true);
    let index = window
        .imp()
        .state
        .borrow()
        .library
        .index_of_path(&sea)
        .unwrap();
    window
        .imp()
        .filmstrip
        .borrow()
        .as_ref()
        .unwrap()
        .navigate_to(index);
    // Shift+F10 opens the current image's menu from the keyboard (D21).
    let filmstrip = window.imp().filmstrip.borrow().clone().unwrap();
    pump_until(|| filmstrip.open_context_menu_for_current());
    assert_eq!(
        filmstrip
            .imp()
            .menu_ctx
            .borrow()
            .as_ref()
            .map(|c| c.clicked_path.clone()),
        Some(sea.clone())
    );
    // The right-click menu opens copies through this entry point (D21).
    window.present_copy_dialog(crate::ui::filmstrip::CopyKind::Crop, sea.clone());
    pump_until(|| window.visible_dialog().is_some());
    let ready = std::time::Instant::now() + std::time::Duration::from_secs(2);
    pump_until(|| std::time::Instant::now() > ready);
    screenshot(&window, &root.parent().unwrap().join("crop-dialog.png"));
    window.visible_dialog().unwrap().close();
    let output = crate::export::resize_copy(&crate::export::ResizeConfig {
        source: sea.clone(),
        destination_dir: root.join("outputs"),
        target_width: 160,
        target_height: 120,
        format: crate::export::ResizeFormat::Webp,
        quality: 90,
    })
    .unwrap()
    .output;
    window.show_created_copy(output.clone());
    pump_until(|| {
        let st = window.imp().state.borrow();
        st.library
            .selected_index
            .and_then(|i| st.library.entry_at(i))
            .is_some_and(|e| e.path() == output)
    });
    // A very large image is shown as a screen-sized copy at its original size;
    // zooming to 1:1 loads full resolution.
    let large = root.parent().unwrap().join("large-6000x4000.bmp");
    image::RgbImage::from_pixel(6000, 4000, image::Rgb([40, 90, 160]))
        .save(&large)
        .unwrap();
    let viewer = window.imp().viewer.borrow().clone().unwrap();
    viewer.load_image(large.clone());
    let shown_width = || {
        viewer
            .imp()
            .picture
            .paintable()
            .map(|p| p.intrinsic_width())
            .unwrap_or(0)
    };
    pump_until(|| shown_width() > 0);
    assert!(shown_width() < 6000, "copy expected, got {}", shown_width());
    assert_eq!(viewer.imp().full_size.get(), Some((6000, 4000)));
    viewer.set_zoom_mode(crate::ui::viewer::ZoomMode::OneToOne);
    pump_until(|| shown_width() == 6000);
    assert_eq!(viewer.imp().full_size.get(), None);
    window.close();
}

#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn rapid_navigation_and_folder_switch_preserve_latest_view() {
    let fixtures = isolated_fixture("navigation");
    let root = fixtures.join("navigation");
    let first = root.join("first");
    let second = root.join("second");
    for (folder, count) in [(&first, 400), (&second, 40)] {
        std::fs::create_dir_all(folder).unwrap();
        for i in 0..count {
            image::RgbImage::from_pixel(32, 18, image::Rgb([i as u8, 100, 150]))
                .save(folder.join(format!("image-{i:04}.bmp")))
                .unwrap();
        }
    }
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let mut settings = AppSettings::load();
    settings.libraries.clear();
    settings
        .add_library("Navigation test", root, FolderMode::TopLevel)
        .unwrap();
    settings.set_active_library_last_folder(Some(first.clone()));
    let app = libadwaita::Application::builder()
        .application_id("io.github.hebbihebb.Sharpr.NavigationTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let window = SharprWindow::new(&app);
    window.present();
    pump_until(|| window.imp().state.borrow().library.image_count() == 400 && window.is_mapped());
    let filmstrip = window.imp().filmstrip.borrow().clone().unwrap();
    for i in (0..400).step_by(17).rev() {
        filmstrip.navigate_to(i);
    }
    let open = window.imp().open_folder_cb.borrow().clone().unwrap();
    open(second.clone());
    open(first.clone());
    open(second.clone());
    pump_until(|| {
        let st = window.imp().state.borrow();
        st.library.current_folder == Some(second.clone()) && st.library.image_count() == 40
    });
    for i in (0..40).rev() {
        filmstrip.navigate_to(i);
    }
    let settled = std::time::Instant::now() + std::time::Duration::from_millis(400);
    pump_until(|| std::time::Instant::now() > settled);
    let st = window.imp().state.borrow();
    assert_eq!(st.scope, ViewScope::Folder(second.clone()));
    assert_eq!(st.library.image_count(), 40);
    for i in 0..40 {
        assert!(st.library.entry_at(i).unwrap().path().starts_with(&second));
    }
    assert!(st.library.selected_index.is_some());
    drop(st);
    window.close();
}
