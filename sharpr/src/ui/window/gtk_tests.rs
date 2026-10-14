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

/// The widget's render node, once it has drawn. Right after mapping, or
/// while the desktop is busy, a frame can take a moment; a window the
/// compositor isn't drawing (covered, on another workspace) may give none.
fn render_node(widget: &gtk4::Widget) -> Option<gtk4::gsk::RenderNode> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let paintable = gtk4::WidgetPaintable::new(Some(widget));
        let snapshot = gtk4::Snapshot::new();
        paintable.snapshot(&snapshot, widget.width() as f64, widget.height() as f64);
        if let Some(node) = snapshot.to_node() {
            return Some(node);
        }
        if std::time::Instant::now() > deadline {
            return None;
        }
        let context = glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn screenshot(window: &SharprWindow, path: &Path) {
    screenshot_widget(window, window.upcast_ref(), path);
}

/// Renders a single widget, such as a popover, with the window's renderer.
/// Screenshots are for people to review, not assertions: when the desktop
/// gives no frame, the test notes it and carries on.
fn screenshot_widget(window: &SharprWindow, widget: &gtk4::Widget, path: &Path) {
    let Some(node) = render_node(widget) else {
        eprintln!("screenshot skipped, no frame drawn: {}", path.display());
        return;
    };
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
        crate::buckets::add_to_bucket(
            st.library_index.as_ref().unwrap(),
            bucket.id,
            &[root.join("mountain.png"), sea.clone()],
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
        crate::buckets::add_to_bucket(
            st.library_index.as_ref().unwrap(),
            second.id,
            &[root.join("mountain.png")],
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
        crate::buckets::add_to_bucket(
            st.library_index.as_ref().unwrap(),
            red.id,
            &[root.join("mountain.png")],
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
    // The right-click menu opens copies through this entry point (D21):
    // Crop Copy crops in the viewer at full size (Phase 6). Its folder is
    // set first; the default, Wallpapers, is the real Pictures folder.
    {
        let state = window.app_state();
        let mut st = state.borrow_mut();
        let mut export = st.settings.webp_export.clone();
        export.dir = Some(root.join("crops"));
        st.settings.set_webp_export(export);
    }
    window.present_copy_dialog(crate::ui::filmstrip::CopyKind::Crop, vec![sea.clone()]);
    pump_until(|| window.crop_session().is_some());
    let session = window.crop_session().unwrap();
    assert!(
        super::file_actions_blocked(window.upcast_ref()),
        "Delete, F2, Ctrl+D, and Shift+F10 are off while cropping"
    );
    assert_eq!(
        session.rect(),
        crate::export::CropRect {
            x: 0,
            y: 15,
            width: 160,
            height: 90
        },
        "starts as the largest centred 16:9"
    );
    session.set_rect(crate::export::CropRect {
        x: 16,
        y: 9,
        width: 128,
        height: 72,
    });
    let ready = std::time::Instant::now() + std::time::Duration::from_millis(500);
    pump_until(|| std::time::Instant::now() > ready);
    screenshot(&window, &root.parent().unwrap().join("crop-mode.png"));
    session.save();
    assert!(window.crop_session().is_none(), "saving leaves crop mode");
    assert!(!super::file_actions_blocked(window.upcast_ref()));
    // The finished copy, not the hidden partial file it is written as.
    let finished = || {
        std::fs::read_dir(root.join("crops"))
            .ok()
            .and_then(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .find(|path| !path.file_name().unwrap().to_string_lossy().starts_with('.'))
            })
    };
    pump_until(|| finished().is_some());
    let cropped = finished().unwrap();
    assert_eq!(image::image_dimensions(&cropped).unwrap(), (128, 72));
    let output = crate::export::crop_copy(&crate::export::CropConfig {
        source: sea.clone(),
        destination_dir: root.join("outputs"),
        rectangle: crate::export::CropRect {
            x: 0,
            y: 0,
            width: 160,
            height: 120,
        },
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
    // A bucket member moved and renamed outside Sharpr, as in Nautilus.
    let moved = root.join("sub/mountain-moved.png");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(root.join("mountain.png"), &moved).unwrap();
    // A fingerprint pass covers the library, and the index caches the result.
    window.request_fingerprints();
    pump_until(|| {
        window
            .fingerprint_progress()
            .is_some_and(|p| p.total > 0 && p.done == p.total)
    });
    let cached = window
        .imp()
        .state
        .borrow()
        .library_index
        .as_ref()
        .unwrap()
        .cached_fingerprints()
        .unwrap();
    assert_eq!(
        cached[sea.to_str().unwrap()].2,
        crate::fingerprint::hash_file(&sea).unwrap()
    );
    // After the pass, the member follows the file into its bucket.
    let index = window.imp().state.borrow().library_index.clone().unwrap();
    pump_until(|| {
        crate::buckets::view(&index, bucket, Some(&root), &[])
            .is_ok_and(|v| v.paths.contains(&moved) && v.missing == 0)
    });
    assert!(index
        .all_bucket_members()
        .unwrap()
        .iter()
        .all(|m| m.last_path != root.join("mountain.png")));
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

/// Visual search end to end with the real model: indexing in the
/// background, then words ranking images by content, file-name matches
/// first. Needs `SHARPR_TEST_MODEL_DIR` (the two GGUF files) and
/// `SHARPR_LLAMA_SERVER`; without them it only checks the name fallback.
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn visual_search_ranks_by_content() {
    let root = isolated_fixture("visual-search");
    std::fs::create_dir_all(&root).unwrap();
    // Names say nothing about content, except one misleading name.
    for (name, colour) in [
        ("one.png", [220, 30, 30]),
        ("two.png", [30, 40, 220]),
        ("three.png", [30, 190, 40]),
        ("green-notes.png", [235, 220, 40]),
    ] {
        image::RgbImage::from_pixel(256, 192, image::Rgb(colour))
            .save(root.join(name))
            .unwrap();
    }
    let model = std::env::var_os("SHARPR_TEST_MODEL_DIR").map(PathBuf::from);
    if let Some(model) = &model {
        // Installed the way a user would (D23), inside the isolated data folder.
        let dir = crate::visual_search::files::model_dir();
        std::fs::create_dir_all(dir.join("runtime")).unwrap();
        for file in [
            crate::visual_search::files::MODEL_FILE,
            crate::visual_search::files::PROJECTOR_FILE,
        ] {
            std::os::unix::fs::symlink(model.join(file), dir.join(file)).unwrap();
        }
        let server = PathBuf::from(std::env::var_os("SHARPR_LLAMA_SERVER").unwrap());
        std::fs::remove_dir(dir.join("runtime")).unwrap();
        std::os::unix::fs::symlink(server.parent().unwrap(), dir.join("runtime")).unwrap();
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
    pump_until(|| window.imp().state.borrow().library.image_count() == 4 && window.is_mapped());
    let status_text = || {
        window
            .imp()
            .search_status
            .borrow()
            .as_ref()
            .filter(|status| status.bar.is_visible())
            .map(|status| status.label.text().to_string())
    };
    let entry = window.imp().inline_search_entry.borrow().clone().unwrap();
    let paths = || {
        let st = window.imp().state.borrow();
        (0..st.library.image_count())
            .map(|i| st.library.entry_at(i).unwrap().path())
            .collect::<Vec<_>>()
    };

    if model.is_none() {
        window.show_inline_search();
        entry.set_text("green");
        pump_until(|| paths() == vec![root.join("green-notes.png")]);
        pump_until(|| status_text().is_some_and(|t| t.starts_with("Matching file names only")));
        window.close();
        return;
    }

    // Fingerprints start 10 s after launch; indexing follows them.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        use crate::visual_search::index::Status;
        match window.visual_search_status() {
            Some(Status::Done(coverage)) if coverage.indexed == 4 => break,
            Some(Status::Unavailable(message)) | Some(Status::Failed(message)) => {
                panic!("{message}")
            }
            _ => {}
        }
        assert!(std::time::Instant::now() < deadline, "indexing timed out");
        let context = glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    window.show_inline_search();
    entry.set_text("green");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while status_text().is_none_or(|t| !t.contains("by file name first")) {
        assert!(std::time::Instant::now() < deadline, "search timed out");
        let context = glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let found = paths();
    assert_eq!(found.len(), 4, "{found:?}");
    assert_eq!(found[0], root.join("green-notes.png"), "name match first");
    assert_eq!(found[1], root.join("three.png"), "then the green image");
    let status = status_text().unwrap();
    assert!(status.contains("all 4 images searched"), "{status}");
    assert!(status.contains("Closest first"), "{status}");
    assert_eq!(
        window.imp().state.borrow().library.selected_index,
        Some(0),
        "ranked results start at the top"
    );
    let status = window.imp().search_status.borrow().clone().unwrap();
    assert!(!status.more.is_visible());
    assert!(
        status.add.is_visible(),
        "ranked results can go into a bucket"
    );

    entry.set_text("red");
    pump_until(|| paths().first() == Some(&root.join("one.png")));
    let settle = std::time::Instant::now();
    pump_until(|| settle.elapsed() > std::time::Duration::from_millis(400));
    screenshot(&window, &root.parent().unwrap().join("visual-search.png"));
    eprintln!("status: {}", status_text().unwrap());
    // Shift-click from the top selects a range, and the line counts it.
    let filmstrip = window.imp().filmstrip.borrow().clone().unwrap();
    let first = paths()[0].clone();
    let third = paths()[2].clone();
    filmstrip.set_action_selection_to_path(&first);
    *filmstrip.imp().selection_anchor.borrow_mut() = Some(first);
    filmstrip.select_range_to_path(&third);
    assert_eq!(window.imp().state.borrow().selected_paths.len(), 3);
    let status = window.imp().search_status.borrow().clone().unwrap();
    assert_eq!(status.selected.text().as_str(), "3 selected");
    // Ctrl-clicking all but one away leaves a selection of one, which
    // actions honour instead of the image being viewed.
    let second = paths()[1].clone();
    filmstrip.toggle_action_selection_path(&paths()[0].clone());
    filmstrip.toggle_action_selection_path(&third);
    assert_eq!(filmstrip.selected_paths_for_action(), vec![second.clone()]);
    assert!(status.selected.is_visible());
    assert_eq!(status.selected.text().as_str(), "1 selected");
    // A plain click is navigation, not a selection of one.
    filmstrip.set_action_selection_to_path(&third);
    assert!(!status.selected.is_visible());
    // Without a selection, "Add to Bucket…" asks which results, then the
    // bucket picker opens.
    window.create_collection_with_paths("Picked".into(), Vec::new(), Vec::new());
    pump_until(|| {
        window
            .imp()
            .state
            .borrow()
            .collections
            .iter()
            .any(|c| c.name == "Picked")
    });
    filmstrip.clear_multi_selection();
    window.add_search_results_to_bucket();
    pump_until(|| window.visible_dialog().is_some());
    let choice = window
        .visible_dialog()
        .unwrap()
        .downcast::<libadwaita::AlertDialog>()
        .unwrap();
    assert_eq!(
        choice.heading().unwrap().as_str(),
        "Add Search Results to a Bucket"
    );
    choice.emit_by_name::<()>("response", &[&"all"]);
    choice.close();
    pump_until(|| {
        window.visible_dialog().is_some_and(|d| {
            d.downcast_ref::<libadwaita::AlertDialog>()
                .and_then(|d| d.heading())
                .is_some_and(|h| h.as_str() == "Add to Collection")
        })
    });
    // Clicking a bucket only ticks it; "Add" adds.
    let picker = window
        .visible_dialog()
        .unwrap()
        .downcast::<libadwaita::AlertDialog>()
        .unwrap();
    let mut widget = picker.extra_child();
    let list = loop {
        let w = widget.expect("the picker has a list");
        if let Ok(list) = w.clone().downcast::<gtk4::ListBox>() {
            break list;
        }
        widget = w.first_child();
    };
    let members = || {
        let index = window.imp().state.borrow().library_index.clone().unwrap();
        index.all_bucket_members().unwrap().len()
    };
    let before = members();
    let row = list
        .row_at_index(1)
        .expect("a bucket row after New Collection");
    list.emit_by_name::<()>("row-activated", &[&row]);
    let settle = std::time::Instant::now();
    pump_until(|| settle.elapsed() > std::time::Duration::from_millis(300));
    assert_eq!(members(), before, "a click alone adds nothing");
    picker.emit_by_name::<()>("response", &[&"add"]);
    picker.close();
    pump_until(|| members() > before);
    pump_until(|| window.visible_dialog().is_none());

    // A negative word leaves out the most alike images (here 1 of 4) and
    // file names containing it.
    window.show_inline_search();
    entry.set_text("green -red");
    pump_until(|| {
        status_text().is_some_and(|t| t.contains("1 left out"))
            && !paths().contains(&root.join("one.png"))
    });
    entry.set_text("green -notes");
    pump_until(|| {
        let shown = paths();
        !shown.is_empty() && !shown.contains(&root.join("green-notes.png"))
    });

    // Escape closes the search, and its status line with it.
    window.close_browse_search();
    pump_until(|| window.imp().state.borrow().library.image_count() == 4);
    assert!(status_text().is_none(), "the old search's status is gone");
    window.close();
}

/// Retiring tags (Phase 5): unused manual tags are offered as buckets, and a
/// chosen one becomes a bucket with the same images; tag rows stay.
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn unused_tags_are_offered_as_buckets() {
    let root = isolated_fixture("tag-offers");
    std::fs::create_dir_all(&root).unwrap();
    let mut images = Vec::new();
    for name in ["a.png", "b.png", "c.png"] {
        image::RgbImage::from_pixel(64, 48, image::Rgb([90, 120, 150]))
            .save(root.join(name))
            .unwrap();
        images.push(root.join(name));
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
    pump_until(|| window.imp().state.borrow().library.image_count() == 3 && window.is_mapped());
    let tags = window.imp().state.borrow().tags.clone().unwrap();
    for path in &images {
        tags.add_tag(path, "mountain");
    }
    tags.add_tag(&images[0], "comics");
    let rows_before = {
        let mut rows = tags.stored_rows().unwrap();
        rows.sort();
        rows
    };

    window.offer_tags_as_buckets(false);
    pump_until(|| window.visible_dialog().is_some());
    let dialog = window
        .visible_dialog()
        .unwrap()
        .downcast::<libadwaita::AlertDialog>()
        .unwrap();
    let list = dialog
        .extra_child()
        .unwrap()
        .downcast::<gtk4::ScrolledWindow>()
        .unwrap()
        .child()
        .and_then(|c| c.first_child().or(Some(c)))
        .unwrap();
    let list = if let Ok(list) = list.clone().downcast::<gtk4::ListBox>() {
        list
    } else {
        list.parent().unwrap().downcast::<gtk4::ListBox>().unwrap()
    };
    let first = list
        .row_at_index(0)
        .unwrap()
        .downcast::<libadwaita::ActionRow>()
        .unwrap();
    assert_eq!(first.title().as_str(), "mountain", "most images first");
    assert_eq!(first.subtitle().unwrap().as_str(), "3 images");
    assert!(!dialog.is_response_enabled("create"), "nothing chosen yet");
    libadwaita::prelude::ActionRowExt::activate(&first);
    pump_until(|| dialog.is_response_enabled("create"));
    let settle = std::time::Instant::now();
    pump_until(|| settle.elapsed() > std::time::Duration::from_millis(400));
    screenshot(&window, &root.parent().unwrap().join("tag-offers.png"));
    dialog.emit_by_name::<()>("response", &[&"create"]);
    dialog.close();
    let index = window.imp().state.borrow().library_index.clone().unwrap();
    pump_until(|| {
        index
            .list_collections()
            .unwrap()
            .iter()
            .any(|c| c.name == "mountain")
    });
    let bucket = index
        .list_collections()
        .unwrap()
        .into_iter()
        .find(|c| c.name == "mountain")
        .unwrap();
    assert_eq!(
        crate::buckets::view(&index, bucket.id, Some(&root), &[])
            .unwrap()
            .paths
            .len(),
        3
    );
    assert!(index
        .list_collections()
        .unwrap()
        .iter()
        .all(|c| c.name != "comics"));
    let mut rows_after = tags.stored_rows().unwrap();
    rows_after.sort();
    assert_eq!(rows_after, rows_before, "tag rows are untouched");
    window.close();
}

/// The subtitle of the first `AdwActionRow` titled `title` under `widget`.
fn row_subtitle(widget: &gtk4::Widget, title: &str) -> Option<String> {
    if let Some(row) = widget.downcast_ref::<libadwaita::ActionRow>() {
        if row.title().as_str() == title {
            return row.subtitle().map(|s| s.to_string());
        }
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = row_subtitle(&current, title) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

/// The Library Overview shows the library's size, formats, copies, and
/// buckets (Phase 5).
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn library_overview_counts_the_library() {
    let root = isolated_fixture("overview");
    std::fs::create_dir_all(&root).unwrap();
    for (name, colour) in [("one.png", [200, 40, 40]), ("two.png", [40, 200, 40])] {
        image::RgbImage::from_pixel(64, 48, image::Rgb(colour))
            .save(root.join(name))
            .unwrap();
    }
    std::fs::copy(root.join("one.png"), root.join("one-copy.png")).unwrap();
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
    window.set_default_size(1024, 700);
    window.present();
    pump_until(|| window.imp().state.borrow().library.image_count() == 3 && window.is_mapped());
    let index = window.imp().state.borrow().library_index.clone().unwrap();
    crate::fingerprint::run_pass(&index, &root, &[], &[], &|| false, &|_| {}).unwrap();

    crate::ui::library_overview::present(&window);
    pump_until(|| window.visible_dialog().is_some());
    let dialog: gtk4::Widget = window.visible_dialog().unwrap().upcast();
    pump_until(|| row_subtitle(&dialog, "Images").is_some_and(|s| s != "…"));
    assert!(row_subtitle(&dialog, "Images")
        .unwrap()
        .starts_with("3 images"));
    assert_eq!(row_subtitle(&dialog, "Formats").unwrap(), "PNG 3");
    assert_eq!(
        row_subtitle(&dialog, "Fingerprinted").unwrap(),
        "3 of 3 images"
    );
    assert_eq!(
        row_subtitle(&dialog, "Exact Copies").unwrap(),
        "1 group of identical files (1 extra)"
    );
    let settle = std::time::Instant::now();
    pump_until(|| settle.elapsed() > std::time::Duration::from_millis(500));
    screenshot(
        &window,
        &root.parent().unwrap().join("library-overview.png"),
    );
    window.close();
}

/// Phase 6: the export dialog opens for the selection, and the background
/// job writes lossless WebP copies without touching the originals.
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn webp_export_runs_in_the_background() {
    let root = isolated_fixture("export");
    std::fs::create_dir_all(&root).unwrap();
    let mut sources = Vec::new();
    for (name, colour) in [("a.png", [200, 40, 40]), ("b.png", [40, 40, 200])] {
        image::RgbImage::from_pixel(96, 54, image::Rgb(colour))
            .save(root.join(name))
            .unwrap();
        sources.push(root.join(name));
    }
    let originals: Vec<Vec<u8>> = sources.iter().map(|p| std::fs::read(p).unwrap()).collect();
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

    crate::ui::export_dialog::present(&window, sources.clone());
    pump_until(|| window.visible_dialog().is_some());
    let dialog = window
        .visible_dialog()
        .unwrap()
        .downcast::<libadwaita::AlertDialog>()
        .unwrap();
    assert_eq!(
        dialog.heading().unwrap().as_str(),
        "Export 2 Images as WebP"
    );
    let settle = std::time::Instant::now();
    pump_until(|| settle.elapsed() > std::time::Duration::from_millis(400));
    screenshot(&window, &root.parent().unwrap().join("export-dialog.png"));
    // Cancel: an export from the dialog would go to the real Pictures folder,
    // which an isolated test must never touch.
    dialog.close();
    pump_until(|| window.visible_dialog().is_none());

    let out = root.parent().unwrap().join("export-out");
    window.start_webp_export(
        sources.clone(),
        crate::export::batch::WebpOptions {
            destination: out.clone(),
            lossless: true,
            quality: 90,
            fit: Some((48, 48)),
        },
    );
    pump_until(|| out.join("a.webp").exists() && out.join("b.webp").exists());
    let exported = image::open(out.join("a.webp")).unwrap();
    assert_eq!((exported.width(), exported.height()), (48, 27), "fitted");
    for (source, original) in sources.iter().zip(&originals) {
        assert_eq!(&std::fs::read(source).unwrap(), original);
    }
    window.close();
}

/// Phase 7: "Find Copies…" groups identical files, keeps the first, and
/// trashes only what the user confirms. (The isolated `XDG_DATA_HOME` holds
/// the test's trash.)
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn copies_are_found_and_trashed_only_on_request() {
    let root = isolated_fixture("copies");
    std::fs::create_dir_all(&root).unwrap();
    image::RgbImage::from_pixel(96, 54, image::Rgb([30, 140, 90]))
        .save(root.join("keep.png"))
        .unwrap();
    std::fs::copy(root.join("keep.png"), root.join("keep copy.png")).unwrap();
    image::RgbImage::from_pixel(96, 54, image::Rgb([200, 30, 30]))
        .save(root.join("other.png"))
        .unwrap();
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
    window.set_default_size(1024, 700);
    window.present();
    pump_until(|| window.imp().state.borrow().library.image_count() == 3 && window.is_mapped());
    let index = window.imp().state.borrow().library_index.clone().unwrap();
    crate::fingerprint::run_pass(&index, &root, &[], &[], &|| false, &|_| {}).unwrap();

    crate::ui::copies_dialog::present(&window);
    pump_until(|| window.visible_dialog().is_some());
    let dialog: gtk4::Widget = window.visible_dialog().unwrap().upcast();
    pump_until(|| row_subtitle(&dialog, "keep copy.png").is_some());
    assert!(
        row_subtitle(&dialog, "other.png").is_none(),
        "a different picture"
    );
    // Nothing is ticked at first; the buttons tick the extras or nothing.
    fn descendants(widget: &gtk4::Widget, out: &mut Vec<gtk4::Widget>) {
        let mut child = widget.first_child();
        while let Some(c) = child {
            out.push(c.clone());
            descendants(&c, out);
            child = c.next_sibling();
        }
    }
    let mut all = Vec::new();
    descendants(&dialog, &mut all);
    let checks: Vec<gtk4::CheckButton> = all
        .iter()
        .filter_map(|w| w.clone().downcast::<gtk4::CheckButton>().ok())
        .collect();
    let ticked = || checks.iter().filter(|c| c.is_active()).count();
    assert_eq!(checks.len(), 2, "one group of two");
    assert_eq!(ticked(), 0, "nothing is ticked for deletion");
    let button = |label: &str| {
        all.iter()
            .filter_map(|w| w.clone().downcast::<gtk4::Button>().ok())
            .find(|b| b.label().as_deref() == Some(label))
            .unwrap()
    };
    button("Select All Extras").emit_clicked();
    assert_eq!(ticked(), 1, "every file but the one to keep");
    assert!(!checks[0].is_active(), "the keeper stays unticked");
    let settle = std::time::Instant::now();
    pump_until(|| settle.elapsed() > std::time::Duration::from_millis(500));
    screenshot(&window, &root.parent().unwrap().join("copies.png"));
    button("Select None").emit_clicked();
    assert_eq!(ticked(), 0);
    // "Compare" opens the group there; dismissing the copy and pressing
    // Done comes back with it ticked and the keeper not.
    button("Compare").emit_clicked();
    pump_until(|| window.imp().state.borrow().scope == ViewScope::Compare);
    let page = window.imp().compare_page.borrow().clone().unwrap();
    pump_until(|| page.survivors().len() == 2);
    page.handle_key(gdk4::Key::bracketright, gdk4::ModifierType::empty());
    page.close();
    pump_until(|| window.visible_dialog().is_some());
    assert_eq!(ticked(), 1);
    assert!(!checks[0].is_active(), "the survivor is not ticked");
    assert!(
        window.imp().state.borrow().selected_paths.is_empty(),
        "browsing selection untouched"
    );
    button("Select None").emit_clicked();
    assert!(
        root.join("keep copy.png").exists(),
        "nothing moves before confirming"
    );
    window.visible_dialog().unwrap().close();

    let stamp = |name: &str| {
        let path = root.join(name);
        let stamp = crate::fingerprint::file_stamp(&std::fs::metadata(&path).unwrap());
        (path, stamp)
    };
    window.trash_copies(vec![crate::copies::TrashGroup {
        extras: vec![stamp("keep copy.png")],
        kept: vec![stamp("keep.png")],
    }]);
    pump_until(|| !root.join("keep copy.png").exists());
    assert!(root.join("keep.png").exists());
    assert!(root.join("other.png").exists());
    pump_until(|| window.imp().state.borrow().library.image_count() == 2);
    window.close();
}

/// Phase 8 acceptance: five images narrow down to one; zoom lock keeps the
/// same view when flipping; the split works while zoomed; leaving Compare
/// selects the survivors.
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn compare_narrows_five_images_to_one() {
    let root = isolated_fixture("compare");
    std::fs::create_dir_all(&root).unwrap();
    let colours = [
        [200, 40, 40],
        [40, 200, 40],
        [40, 40, 200],
        [200, 200, 40],
        [40, 200, 200],
    ];
    let mut paths = Vec::new();
    for (i, colour) in colours.iter().enumerate() {
        let path = root.join(format!("{i}.png"));
        image::RgbImage::from_fn(320, 180, |x, _| {
            image::Rgb(if x < 160 {
                *colour
            } else {
                [colour[2], colour[0], colour[1]]
            })
        })
        .save(&path)
        .unwrap();
        paths.push(path);
    }
    let originals: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
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
    pump_until(|| window.imp().state.borrow().library.image_count() == 5 && window.is_mapped());

    let filmstrip = window.imp().filmstrip.borrow().clone().unwrap();
    filmstrip.select_all();
    gio::prelude::ActionGroupExt::activate_action(&window, "compare", None);
    let page = window.imp().compare_page.borrow().clone().unwrap();
    pump_until(|| page.survivors().len() == 5);
    let view = page.view();
    pump_until(|| page.imp().shown.borrow().iter().all(Option::is_some));

    // Zoom lock: at 100%, flipping shows the same region.
    view.set_mode(crate::ui::compare_view::Mode::Flip);
    view.set_zoom(Some(1.0));
    let before = view.views();
    view.flip();
    assert_eq!(view.views(), before);
    assert_eq!(before[0], before[1], "one shared view while locked");
    // The split moves at any zoom.
    view.set_mode(crate::ui::compare_view::Mode::Split);
    view.set_split(0.3);
    assert!((view.split() - 0.3).abs() < 1e-9);
    view.set_zoom(None);
    let stack = window.imp().content_stack.borrow().clone().unwrap();
    let settle = std::time::Instant::now();
    pump_until(|| {
        !stack.is_transition_running() && settle.elapsed() > std::time::Duration::from_millis(500)
    });
    screenshot(&window, &root.parent().unwrap().join("compare-split.png"));

    assert!(super::file_actions_blocked(window.upcast_ref()));
    let inner = window
        .imp()
        .presentation_inner_split
        .borrow()
        .clone()
        .unwrap();
    assert!(!inner.shows_sidebar(), "the filmstrip makes way");
    // The focus moved into Compare, off the filmstrip.
    let focus = gtk4::prelude::GtkWindowExt::focus(&window).expect("something has focus");
    assert!(
        focus.is_ancestor(&page) || focus == *page.upcast_ref::<gtk4::Widget>(),
        "focus is on {focus:?}"
    );
    // Each side's picker shows any image not dismissed; picking the other
    // side's image swaps them.
    let pickers = page.imp().pickers.clone();
    assert_eq!(pickers[1].model().unwrap().n_items(), 5);
    pickers[1].set_selected(3);
    pump_until(|| page.imp().shown.borrow()[1].as_ref() == Some(&paths[3]));
    pickers[0].set_selected(3);
    pump_until(|| page.imp().shown.borrow()[0].as_ref() == Some(&paths[3]));
    assert_eq!(page.imp().shown.borrow()[1].as_ref(), Some(&paths[0]));
    let settle = std::time::Instant::now();
    pump_until(|| settle.elapsed() > std::time::Duration::from_millis(400));
    screenshot(&window, &root.parent().unwrap().join("compare-pickers.png"));
    let none = gdk4::ModifierType::empty();
    for key in [
        gdk4::Key::bracketleft,
        gdk4::Key::bracketright,
        gdk4::Key::bracketleft,
        gdk4::Key::bracketright,
    ] {
        assert_eq!(page.handle_key(key, none), glib::Propagation::Stop);
    }
    assert_eq!(page.survivors().len(), 1, "five narrow down to one");
    assert_eq!(
        view.mode(),
        crate::ui::compare_view::Mode::Flip,
        "the last image fills the canvas"
    );
    assert!(!view.flipped());
    let winner = page.survivors()[0].clone();
    page.close();
    pump_until(|| window.imp().state.borrow().scope != ViewScope::Compare);
    let st = window.imp().state.borrow();
    assert_eq!(
        st.selected_paths.iter().cloned().collect::<Vec<_>>(),
        vec![winner.clone()],
        "the survivor stays selected"
    );
    drop(st);

    // Switching the content away (as searching or opening a folder does)
    // closes Compare properly, keeping who was left.
    filmstrip.select_all();
    gio::prelude::ActionGroupExt::activate_action(&window, "compare", None);
    pump_until(|| page.survivors().len() == 5);
    page.handle_key(gdk4::Key::bracketright, none);
    stack.set_visible_child_name("viewer");
    pump_until(|| window.imp().state.borrow().scope != ViewScope::Compare);
    assert_eq!(window.imp().state.borrow().selected_paths.len(), 4);
    assert!(page.survivors().is_empty(), "Compare was closed");
    assert!(!super::file_actions_blocked(window.upcast_ref()));
    assert!(inner.shows_sidebar(), "the filmstrip is back");

    for (path, original) in paths.iter().zip(&originals) {
        assert_eq!(
            &std::fs::read(path).unwrap(),
            original,
            "Compare changes no file"
        );
    }
    window.close();
}

/// A minus word typed last becomes a chip when Enter is pressed, as it
/// does after a space.
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn enter_turns_a_final_minus_word_into_a_chip() {
    let root = isolated_fixture("minus-chip");
    std::fs::create_dir_all(&root).unwrap();
    image::RgbImage::from_pixel(64, 36, image::Rgb([90, 90, 90]))
        .save(root.join("tower.png"))
        .unwrap();
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
    window.present();
    pump_until(|| window.imp().state.borrow().library.image_count() == 1 && window.is_mapped());

    let entry = window.imp().inline_search_entry.borrow().clone().unwrap();
    entry.set_text("tower -water");
    entry.emit_activate();
    let chips = window.imp().browse_options.borrow().chips.chips.clone();
    assert_eq!(
        chips,
        vec![crate::model::search_query::Chip::Without("water".into())]
    );
    assert_eq!(entry.text(), "tower");
    // Plain words stay in the box.
    entry.emit_activate();
    assert_eq!(entry.text(), "tower");
    window.close();
}
