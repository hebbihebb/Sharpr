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

#[track_caller]
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
/// Delivers Ctrl+A as GTK's bubble phase does: from `focus` up to the window,
/// the first shortcut that handles it wins. Returns the widget that did.
fn bubble_ctrl_a(focus: &gtk4::Widget) -> Option<gtk4::Widget> {
    let mut widget = Some(focus.clone());
    while let Some(current) = widget {
        let controllers = current.observe_controllers();
        for i in 0..controllers.n_items() {
            let Some(controller) = controllers
                .item(i)
                .and_downcast::<gtk4::ShortcutController>()
            else {
                continue;
            };
            for j in 0..controller.n_items() {
                let shortcut = controller.item(j).and_downcast::<gtk4::Shortcut>().unwrap();
                let trigger = shortcut.trigger().map(|t| t.to_str());
                let Some(action) = shortcut.action() else {
                    continue;
                };
                if trigger.as_deref() == Some("<Control>a")
                    && action.activate(
                        gtk4::ShortcutActionFlags::empty(),
                        &current,
                        shortcut.arguments().as_ref(),
                    )
                {
                    return Some(current);
                }
            }
        }
        widget = current.parent();
    }
    None
}

fn window_shortcut(window: &SharprWindow, trigger: &str) -> bool {
    let expected = gtk4::ShortcutTrigger::parse_string(trigger)
        .unwrap()
        .to_str();
    let controllers = window.observe_controllers();
    for i in 0..controllers.n_items() {
        let Some(controller) = controllers
            .item(i)
            .and_downcast::<gtk4::ShortcutController>()
        else {
            continue;
        };
        for j in 0..controller.n_items() {
            let shortcut = controller.item(j).and_downcast::<gtk4::Shortcut>().unwrap();
            if shortcut.trigger().is_some_and(|t| t.to_str() == expected) {
                return shortcut.action().unwrap().activate(
                    gtk4::ShortcutActionFlags::empty(),
                    window,
                    shortcut.arguments().as_ref(),
                );
            }
        }
    }
    panic!("Compare shortcut is installed");
}

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
fn screenshot_widget(window: &impl IsA<gtk4::Window>, widget: &gtk4::Widget, path: &Path) {
    let Some(node) = render_node(widget) else {
        eprintln!("screenshot skipped, no frame drawn: {}", path.display());
        return;
    };
    let renderer = window
        .as_ref()
        .renderer()
        .expect("realized window has a renderer");
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
    let settled_at = std::time::Instant::now();
    pump_until(|| settled_at.elapsed() > std::time::Duration::from_millis(400));
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
    let settled_at = std::time::Instant::now();
    pump_until(|| settled_at.elapsed() > std::time::Duration::from_millis(400));
    screenshot(&window, &root.parent().unwrap().join("search-chips.png"));
    // Ctrl+I's Info panel opens once the image's details are loaded (D18).
    let viewer = window.imp().viewer.borrow().clone().unwrap();
    viewer.show_info();
    let info = viewer.imp().metadata_chip.imp().details.clone();
    pump_until(|| info.is_visible());
    let settled_at = std::time::Instant::now();
    pump_until(|| settled_at.elapsed() > std::time::Duration::from_millis(300));
    screenshot_widget(
        &window,
        info.upcast_ref(),
        &root.parent().unwrap().join("info-panel.png"),
    );
    viewer.imp().metadata_chip.hide_info();
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
    // Exercise the visible menu and chips, not the retired hidden controls.
    let open = window.imp().open_folder_cb.borrow().clone().unwrap();
    open(root.clone());
    pump_until(|| {
        let st = window.app_state();
        let st = st.borrow();
        st.scope == ViewScope::Folder(root.clone()) && st.library.image_count() == 2
    });
    let filmstrip = window.imp().filmstrip.borrow().clone().unwrap();
    let sea_index = window
        .app_state()
        .borrow()
        .library
        .index_of_path(&sea)
        .unwrap();
    filmstrip.navigate_to(sea_index);
    let controls: Vec<_> = find_widgets::<gtk4::MenuButton>(window.upcast_ref())
        .into_iter()
        .filter(|b| b.tooltip_text().as_deref() == Some("Sort and filter"))
        .collect();
    assert_eq!(
        controls.len(),
        1,
        "one combined control survives header handoff"
    );
    let activate_sort_row = |text: &str| {
        controls[0].popup();
        let menu = controls[0].popover().unwrap();
        pump_until(|| menu.is_mapped());
        let label = find_widgets::<gtk4::Label>(menu.upcast_ref())
            .into_iter()
            .find(|label| label.label() == text)
            .unwrap();
        let mut row = label.parent().unwrap();
        while row.type_().name() != "GtkModelButton" {
            row = row.parent().expect("sort belongs to a native menu row");
        }
        assert!(row.is_sensitive(), "{text} must be enabled");
        assert!(row.activate(), "activate {text}");
        controls[0].popdown();
    };
    activate_sort_row("Name");
    for (label, order, first) in [
        (
            "Ascending",
            crate::model::library::SortOrder::NameAsc,
            root.join("mountain.png"),
        ),
        (
            "Descending",
            crate::model::library::SortOrder::NameDesc,
            sea.clone(),
        ),
    ] {
        activate_sort_row(label);
        pump_until(|| {
            let st = window.app_state();
            let st = st.borrow();
            st.sort_order == order && st.library.entry_at(0).is_some_and(|e| e.path() == first)
        });
        assert_eq!(
            window
                .app_state()
                .borrow()
                .library
                .selected_index
                .and_then(|i| window.app_state().borrow().library.entry_at(i))
                .unwrap()
                .path(),
            sea,
            "changing direction preserves the selected image"
        );
    }
    for (theme, scheme) in [
        ("light", libadwaita::ColorScheme::ForceLight),
        ("dark", libadwaita::ColorScheme::ForceDark),
    ] {
        libadwaita::StyleManager::default().set_color_scheme(scheme);
        for (width, height) in [(1600, 900), (1024, 600)] {
            window.set_default_size(width, height);
            pump_until(|| window.width() == width && window.height() == height);
            controls[0].popup();
            pump_until(|| controls[0].popover().is_some_and(|p| p.is_mapped()));
            settle(200);
            screenshot(
                &window,
                &root
                    .parent()
                    .unwrap()
                    .join(format!("sort-filter-{theme}-{width}.png")),
            );
            screenshot_widget(
                &window,
                controls[0].popover().unwrap().upcast_ref(),
                &root
                    .parent()
                    .unwrap()
                    .join(format!("sort-filter-menu-{theme}-{width}.png")),
            );
            controls[0].popdown();
            settle(50);
        }
    }
    controls[0].popup();
    let menu = controls[0].popover().unwrap();
    pump_until(|| menu.is_mapped());
    let ratio = find_widgets::<gtk4::Label>(menu.upcast_ref())
        .into_iter()
        .find(|l| l.label() == "16:9")
        .unwrap();
    let mut row = ratio.parent().unwrap();
    while row.type_().name() != "GtkModelButton" {
        row = row.parent().expect("filter belongs to a native menu row");
    }
    assert!(row.activate(), "activate the actual menu item");
    controls[0].popdown();
    pump_until(|| window.app_state().borrow().library.image_count() == 1);
    let menu_result = window
        .app_state()
        .borrow()
        .library
        .entry_at(0)
        .unwrap()
        .path();
    assert_eq!(menu_result, root.join("mountain.png"));
    assert_eq!(
        window.imp().browse_options.borrow().chips.chips[0].label(),
        "16:9"
    );
    find_widgets::<gtk4::Button>(window.upcast_ref())
        .into_iter()
        .find(|b| b.tooltip_text().as_deref() == Some("Remove filter"))
        .unwrap()
        .emit_clicked();
    pump_until(|| window.app_state().borrow().library.image_count() == 2);
    window.close_browse_search();
    window.show_inline_search();
    entry.set_text("16:9 ");
    pump_until(|| window.app_state().borrow().library.image_count() == 1);
    assert_eq!(
        window
            .app_state()
            .borrow()
            .library
            .entry_at(0)
            .unwrap()
            .path(),
        menu_result
    );
    window.close_browse_search();
    pump_until(|| {
        let st = window.app_state();
        let st = st.borrow();
        st.library.image_count() == 2
            && st
                .library
                .selected_index
                .and_then(|i| st.library.entry_at(i))
                .is_some_and(|e| e.path() == sea)
    });
    assert_eq!(
        window.app_state().borrow().scope,
        ViewScope::Folder(root.clone())
    );
    assert!(window.imp().browse_options.borrow().chips.is_empty());
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
    for (folder, count) in [(&first, 5000), (&second, 40)] {
        std::fs::create_dir_all(folder).unwrap();
        for i in 0..count {
            image::RgbImage::from_pixel(32, 18, image::Rgb([i as u8, 100, 150]))
                .save(folder.join(format!("image-{i:04}.bmp")))
                .unwrap();
        }
    }
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::bench::init();
    crate::bench::start_main_loop_probe();
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
    pump_until(|| window.imp().state.borrow().library.image_count() == 5000 && window.is_mapped());
    let filmstrip = window.imp().filmstrip.borrow().clone().unwrap();
    let scroll = filmstrip.imp().scroll.vadjustment();
    pump_until(|| scroll.page_size() > 0.0 && scroll.upper() > scroll.page_size());
    for fraction in [0.0, 0.8, 0.2, 1.0, 0.4, 0.0] {
        scroll.set_value((scroll.upper() - scroll.page_size()) * fraction);
        settle(50);
    }
    for i in (0..5000).step_by(97).rev() {
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
    // The handle stays wide to grab, without padding its column (user, October 15).
    let fast_scroll = filmstrip.imp().scroll.vscrollbar();
    assert!(
        fast_scroll.is_mapped() && fast_scroll.width() <= 16,
        "{}",
        fast_scroll.width()
    );
    screenshot_widget(
        &window,
        filmstrip.upcast_ref(),
        &fixtures.join("filmstrip-scroll.png"),
    );
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
                .is_some_and(|h| h.as_str() == "Add to Bucket")
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
    let row = list.row_at_index(1).expect("a bucket row after New Bucket");
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
#[allow(deprecated)] // Inspect GTK's native-chooser fallback in this process only.
fn webp_export_runs_in_the_background() {
    let root = isolated_fixture("export");
    // Exercise the real FileDialog callbacks without driving a host portal.
    std::env::set_var("GDK_DEBUG", "no-portals");
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
    let mut export_settings = settings.webp_export.clone();
    export_settings.dir = Some(root.clone());
    settings.set_webp_export(export_settings);
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
    let dialog = window.visible_dialog().unwrap();
    assert!(dialog.has_css_class("sharpr-sheet"));
    assert_eq!(dialog.title(), "Export 2 Images as WebP");
    pump_until(|| dialog.is_mapped());
    let main_surface = window.surface().unwrap();
    assert_eq!(
        dialog.native().unwrap().surface().as_ref(),
        Some(&main_surface)
    );
    assert!(!find_widgets::<gtk4::Popover>(window.upcast_ref())
        .iter()
        .any(|p| p.is_visible() && p.has_css_class("sharpr-picker")));
    for (theme, scheme) in [
        ("light", libadwaita::ColorScheme::ForceLight),
        ("dark", libadwaita::ColorScheme::ForceDark),
    ] {
        libadwaita::StyleManager::default().set_color_scheme(scheme);
        for (width, height) in [(1600, 900), (1024, 600)] {
            window.set_default_size(width, height);
            pump_until(|| window.width() == width && window.height() == height);
            settle(250);
            let bounds = dialog.child().unwrap().compute_bounds(&window).unwrap();
            assert!(bounds.width() <= width as f32 && bounds.height() <= height as f32);
            screenshot(
                &window,
                &root
                    .parent()
                    .unwrap()
                    .join(format!("export-{theme}-{width}.png")),
            );
        }
    }
    let button = |sheet: &libadwaita::Dialog, label: &str| {
        find_widgets::<gtk4::Button>(sheet.upcast_ref())
            .into_iter()
            .find(|b| b.label().as_deref() == Some(label))
            .unwrap()
    };
    button(&dialog, "Cancel").emit_clicked();
    pump_until(|| window.visible_dialog().is_none());
    crate::ui::export_dialog::present(&window, sources.clone());
    pump_until(|| window.visible_dialog().is_some());
    let dialog = window.visible_dialog().unwrap();
    settle(400);
    let controllers = window.observe_controllers();
    let outside = (0..controllers.n_items())
        .filter_map(|i| controllers.item(i).and_downcast::<gtk4::GestureClick>())
        .find(|c| c.name().as_deref() == Some("export-sheet-outside"))
        .unwrap();
    let center = dialog
        .child()
        .unwrap()
        .compute_bounds(&window)
        .unwrap()
        .center();
    outside.emit_by_name::<()>(
        "pressed",
        &[&1i32, &f64::from(center.x()), &f64::from(center.y())],
    );
    assert_eq!(window.visible_dialog().as_ref(), Some(&dialog));
    outside.emit_by_name::<()>("pressed", &[&1i32, &4.0f64, &4.0f64]);
    pump_until(|| window.visible_dialog().is_none());
    let controllers = window.observe_controllers();
    assert!(!(0..controllers.n_items())
        .filter_map(|i| controllers.item(i).and_downcast::<gtk4::GestureClick>())
        .any(|c| c.name().as_deref() == Some("export-sheet-outside")));
    crate::ui::export_dialog::present(&window, sources.clone());
    pump_until(|| window.visible_dialog().is_some());
    let dialog = window.visible_dialog().unwrap();
    let lossless = find_widgets::<libadwaita::SwitchRow>(dialog.upcast_ref())
        .into_iter()
        .find(|row| row.title() == "Lossless")
        .unwrap();
    lossless.set_active(false);
    let quality = find_widgets::<libadwaita::SpinRow>(dialog.upcast_ref())
        .into_iter()
        .find(|row| row.title() == "Quality")
        .unwrap();
    quality.set_value(83.0);
    let native_chooser = || {
        gtk4::Window::toplevels()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|w| w.downcast::<gtk4::FileChooserDialog>().ok())
            .find(|w| w.is_visible())
    };
    let out = root.join("export-out");
    std::fs::create_dir_all(&out).unwrap();
    for response in [gtk4::ResponseType::Cancel, gtk4::ResponseType::Accept] {
        button(&dialog, "Choose…").emit_clicked();
        pump_until(|| native_chooser().is_some());
        let chooser = native_chooser().unwrap();
        if response == gtk4::ResponseType::Accept {
            // FileDialog applies its initial folder asynchronously. Wait for
            // that before navigating, or it can replace our test selection.
            pump_until(|| chooser.current_folder().and_then(|f| f.path()).as_ref() == Some(&root));
            chooser
                .set_current_folder(Some(&gio::File::for_path(&out)))
                .unwrap();
            pump_until(|| chooser.file().and_then(|f| f.path()).as_ref() == Some(&out));
        }
        chooser.response(response);
        pump_until(|| !chooser.is_visible());
        settle(200);
        assert!(
            window.visible_dialog().as_ref() == Some(&dialog) && dialog.is_mapped(),
            "the same export sheet must stay mapped after the chooser: {response:?}"
        );
        assert!(!lossless.is_active(), "folder choice keeps edited options");
        assert_eq!(quality.value(), 83.0);
    }
    assert!(find_widgets::<libadwaita::ActionRow>(dialog.upcast_ref())
        .iter()
        .any(|r| r.subtitle().is_some_and(|s| s.ends_with("/export-out"))));
    // Use lossless export below for an exact decoded-pixel comparison.
    lossless.set_active(true);
    find_widgets::<libadwaita::SwitchRow>(dialog.upcast_ref())
        .into_iter()
        .find(|row| row.title() == "Fit Within")
        .unwrap()
        .set_active(true);
    for row in find_widgets::<libadwaita::SpinRow>(dialog.upcast_ref()) {
        if row.title() == "Width" || row.title() == "Height" {
            row.set_value(48.0);
        }
    }
    button(&dialog, "Export").emit_clicked();
    pump_until(|| window.visible_dialog().is_none());
    assert_eq!(
        window
            .app_state()
            .borrow()
            .settings
            .webp_export
            .dir
            .as_ref(),
        Some(&out)
    );
    pump_until(|| out.join("a.webp").exists() && out.join("b.webp").exists());
    let exported = image::open(out.join("a.webp")).unwrap();
    assert_eq!((exported.width(), exported.height()), (48, 27), "fitted");
    assert!(exported.to_rgb8().pixels().all(|p| p.0 == [200, 40, 40]));
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
    // Ctrl+A from the focused filmstrip list reaches the window's shortcut,
    // which selects every image (user test of `ef91d32`: the list's own
    // Ctrl+A swallowed it).
    filmstrip.imp().list_view.grab_focus();
    assert!(
        !window_shortcut(&window, "<Control><Shift>c"),
        "one image cannot open Compare"
    );
    let handled_by = bubble_ctrl_a(filmstrip.imp().list_view.upcast_ref());
    assert_eq!(
        handled_by.as_ref(),
        Some(window.upcast_ref::<gtk4::Widget>()),
        "Ctrl+A is handled by the window, not the list"
    );
    assert_eq!(window.imp().state.borrow().selected_paths.len(), 5);
    window.show_inline_search();
    window
        .imp()
        .inline_search_entry
        .borrow()
        .as_ref()
        .unwrap()
        .grab_focus();
    super::gtk_tests::settle(150);
    assert!(
        !window_shortcut(&window, "<Control><Shift>c"),
        "typing keeps its shortcut keys"
    );
    filmstrip.imp().list_view.grab_focus();
    super::gtk_tests::settle(150);
    let dialog = libadwaita::Dialog::new();
    dialog.present(Some(&window));
    pump_until(|| window.visible_dialog().is_some());
    assert!(
        !window_shortcut(&window, "<Control><Shift>c"),
        "a modal sheet blocks Compare"
    );
    dialog.close();
    pump_until(|| window.visible_dialog().is_none());
    filmstrip.imp().list_view.grab_focus();
    super::gtk_tests::settle(250);
    window.imp().crop_preparing.set(true);
    assert!(
        !window_shortcut(&window, "<Control><Shift>c"),
        "crop preparation blocks Compare"
    );
    window.imp().crop_preparing.set(false);
    assert!(
        window_shortcut(&window, "<Control><Shift>c"),
        "two or more selected images open Compare"
    );
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
    // Visible candidate activation changes only the explicit target side.
    let candidates = &page.imp().candidates;
    assert_eq!(
        candidates.target.get(),
        crate::ui::compare_item::Side::Right
    );
    assert_eq!(candidates.store.n_items(), 5);
    candidates.list.emit_by_name::<()>("activate", &[&3u32]);
    pump_until(|| page.imp().shown.borrow()[1].as_ref() == Some(&paths[3]));
    assert_eq!(page.survivors(), paths, "choosing dismisses nobody");
    candidates.replace[0].set_active(true);
    candidates.list.emit_by_name::<()>("activate", &[&3u32]);
    pump_until(|| page.imp().shown.borrow()[0].as_ref() == Some(&paths[3]));
    assert_eq!(page.imp().shown.borrow()[1].as_ref(), Some(&paths[0]));
    // Cold-cache rows acquire actual thumbnails off-thread.
    pump_until(|| {
        find_widgets::<gtk4::Picture>(&candidates.root.clone().upcast())
            .iter()
            .filter(|p| p.paintable().is_some())
            .count()
            == 5
    });
    for (theme, scheme) in [
        ("light", libadwaita::ColorScheme::ForceLight),
        ("dark", libadwaita::ColorScheme::ForceDark),
    ] {
        libadwaita::StyleManager::default().set_color_scheme(scheme);
        for (width, height) in [(1024, 600), (1600, 900)] {
            window.set_default_size(width, height);
            pump_until(|| window.width() == width && window.height() == height);
            super::gtk_tests::settle(200);
            screenshot(
                &window,
                &root
                    .parent()
                    .unwrap()
                    .join(format!("compare-candidates-{theme}-{width}.png")),
            );
        }
    }
    let all = page.survivors();
    page.keep(crate::ui::compare_item::Side::Left);
    assert_eq!(candidates.store.n_items(), 4);
    page.undo();
    assert_eq!(candidates.store.n_items(), 5);
    assert_eq!(page.survivors(), all, "Undo restores original order");
    candidates.list.grab_focus();
    assert_eq!(
        window.handle_mode_key(gdk4::Key::Right, gdk4::ModifierType::empty()),
        glib::Propagation::Proceed
    );
    assert_eq!(
        page.survivors(),
        all,
        "candidate navigation cannot pick a winner"
    );
    page.view().grab_focus();
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

    // A large candidate set creates only visible row widgets. Scrolling and
    // starting another session must not apply an earlier session's thumbnails.
    let many: Vec<_> = (0..500)
        .map(|i| {
            let path = root.join(format!("candidate-{i:04}.png"));
            std::fs::hard_link(&paths[i % paths.len()], &path).unwrap();
            path
        })
        .collect();
    window.enter_compare_mode(many);
    pump_until(|| page.imp().candidates.store.n_items() == 500);
    super::gtk_tests::settle(200);
    let widgets = find_widgets::<gtk4::Picture>(&page.imp().candidates.root.clone().upcast());
    assert!(
        widgets.len() < 500,
        "candidate rows are virtualized, got {}",
        widgets.len()
    );
    assert!(
        widgets.iter().filter(|p| p.paintable().is_some()).count() < 50,
        "only visible candidates request thumbnail decodes"
    );
    page.imp()
        .candidates
        .list
        .scroll_to(499, gtk4::ListScrollFlags::FOCUS, None);
    super::gtk_tests::settle(200);
    page.imp()
        .candidates
        .list
        .emit_by_name::<()>("activate", &[&499u32]);
    assert!(page.imp().shown.borrow()[1]
        .as_ref()
        .unwrap()
        .ends_with("candidate-0499.png"));
    page.close();
    window.enter_compare_mode(paths.clone());
    assert_eq!(
        page.imp().candidates.target.get(),
        crate::ui::compare_item::Side::Right
    );
    assert_eq!(page.imp().candidates.store.n_items(), 5);
    super::gtk_tests::settle(200);
    page.close();

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

/// D25: Delete trashes at once, and Undo puts the image back where it was,
/// never over a file that has since taken its name. (The isolated
/// `XDG_DATA_HOME` holds the test's trash.)
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn filmstrip_trash_can_be_undone() {
    let root = isolated_fixture("trash-undo");
    std::fs::create_dir_all(&root).unwrap();
    for (name, colour) in [
        ("a.png", [200, 40, 40]),
        ("b.png", [40, 200, 40]),
        ("c.png", [40, 40, 200]),
    ] {
        image::RgbImage::from_pixel(96, 54, image::Rgb(colour))
            .save(root.join(name))
            .unwrap();
    }
    let (a, b) = (root.join("a.png"), root.join("b.png"));
    let (a_bytes, b_bytes) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
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
    let count = || window.imp().state.borrow().library.image_count();
    pump_until(|| count() == 3 && window.is_mapped());
    let state = window.app_state();
    let filmstrip = window.imp().filmstrip.borrow().clone().unwrap();
    let viewer = window.imp().viewer.borrow().clone().unwrap();

    super::trash_paths(
        Some(&window),
        &state,
        &filmstrip,
        &viewer,
        std::slice::from_ref(&a),
    );
    assert!(!a.exists());
    assert_eq!(count(), 2);
    window.restore_trashed(vec![a.clone()]);
    pump_until(|| {
        a.exists()
            && count() == 3
            && state.borrow().library.selected_entry().map(|e| e.path()) == Some(a.clone())
    });
    assert_eq!(std::fs::read(&a).unwrap(), a_bytes, "back unchanged");

    // A new file at the name stays; the trashed one waits in the trash.
    super::trash_paths(
        Some(&window),
        &state,
        &filmstrip,
        &viewer,
        std::slice::from_ref(&b),
    );
    assert!(!b.exists());
    std::fs::write(&b, b"a new file").unwrap();
    assert_eq!(
        crate::file_ops::restore_from_trash(&b).unwrap_err(),
        "another file now has its name"
    );
    assert_eq!(std::fs::read(&b).unwrap(), b"a new file");
    std::fs::remove_file(&b).unwrap();
    crate::file_ops::restore_from_trash(&b).unwrap();
    assert_eq!(std::fs::read(&b).unwrap(), b_bytes, "still restorable");
    window.close();
}

/// Sky top, sky horizon, sun, near ridge.
type Palette = [[u8; 3]; 4];

/// A generated landscape for screenshots: a sky gradient, a sun, and two
/// ridges of hills.
fn scene(width: u32, height: u32, palette: Palette, phase: f32) -> image::RgbImage {
    let mix = |a: [u8; 3], b: [u8; 3], t: f32| {
        image::Rgb([0, 1, 2].map(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t) as u8))
    };
    let (w, h) = (width as f32, height as f32);
    let (sun_x, sun_y, sun_r) = (w * (0.3 + 0.4 * phase), h * 0.42, h.min(w) * 0.09);
    image::RgbImage::from_fn(width, height, |x, y| {
        let (fx, fy) = (x as f32, y as f32);
        let ridge = |base: f32, amp: f32, freq: f32| {
            h * base
                + amp
                    * h
                    * ((fx / w * freq + phase * 6.0).sin() * 0.6
                        + (fx / w * freq * 2.3).cos() * 0.4)
        };
        if fy > ridge(0.78, 0.05, 9.0) {
            return mix(palette[3], [8, 8, 12], (fy / h - 0.75) * 2.0);
        }
        if fy > ridge(0.62, 0.07, 5.0) {
            return mix(palette[3], palette[1], 0.45);
        }
        let d = ((fx - sun_x).powi(2) + (fy - sun_y).powi(2)).sqrt();
        if d < sun_r {
            return image::Rgb(palette[2]);
        }
        let sky = mix(palette[0], palette[1], (fy / (h * 0.7)).min(1.0));
        let glow = (1.0 - (d / (sun_r * 5.0))).max(0.0) * 0.5;
        mix([sky.0[0], sky.0[1], sky.0[2]], palette[2], glow)
    })
}

/// Every widget of type `T` under `root`, popovers included.
fn find_widgets<T: IsA<gtk4::Widget>>(root: &gtk4::Widget) -> Vec<T> {
    let mut found = Vec::new();
    let mut child = root.first_child();
    while let Some(c) = child {
        if let Ok(t) = c.clone().downcast::<T>() {
            found.push(t);
        }
        found.extend(find_widgets::<T>(&c));
        child = c.next_sibling();
    }
    found
}

fn settle(ms: u64) {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    pump_until(|| std::time::Instant::now() > until);
}

/// Contextual controls preserve browsing state and reach the same actions
/// from the keyboard, including when a header button has focus.
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn mode_headers_keep_choices_and_restore_search() {
    let root = isolated_fixture("mode-headers");
    let output = root.join("a deliberately long crop destination folder name");
    std::fs::create_dir_all(&output).unwrap();
    let paths: Vec<_> = (0..5)
        .map(|i| root.join(format!("image-{i}-a-long-name-for-the-image-picker.png")))
        .collect();
    for (i, path) in paths.iter().enumerate() {
        image::RgbImage::from_pixel(160, 90, image::Rgb([40 + i as u8 * 30, 90, 130]))
            .save(path)
            .unwrap();
    }
    let originals: Vec<_> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let mut settings = AppSettings::load();
    settings.libraries.clear();
    settings
        .add_library("Test", root.clone(), FolderMode::TopLevel)
        .unwrap();
    settings.set_active_library_last_folder(Some(root.clone()));
    let mut export = settings.webp_export.clone();
    export.dir = Some(output.clone());
    settings.set_webp_export(export);
    let app = libadwaita::Application::builder()
        .application_id("io.github.hebbihebb.Sharpr.GtkTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let window = SharprWindow::new(&app);
    window.set_default_size(1024, 600);
    window.present();
    pump_until(|| window.is_mapped() && window.app_state().borrow().library.image_count() == 5);
    window.show_inline_search();
    let entry = window.imp().inline_search_entry.borrow().clone().unwrap();
    entry.set_text("image png ");
    pump_until(|| {
        !window.imp().browse_options.borrow().chips.is_empty()
            && window.app_state().borrow().library.image_count() == 5
    });
    let query = entry.text();
    let chips = window.imp().browse_options.borrow().chips.clone();
    let header = window.imp().presentation_header.borrow().clone().unwrap();
    let page = window.imp().compare_page.borrow().clone().unwrap();
    let none = gdk4::ModifierType::empty();
    for (scheme, theme) in [
        (libadwaita::ColorScheme::ForceLight, "light"),
        (libadwaita::ColorScheme::ForceDark, "dark"),
    ] {
        libadwaita::StyleManager::default().set_color_scheme(scheme);
        for (width, height) in [(1024, 600), (1600, 900)] {
            window.set_default_size(width, height);
            window.enter_compare_mode(paths.clone());
            settle(500);
            assert_eq!(
                window.width(),
                width,
                "controls fit without enlarging the window"
            );
            assert!(window
                .imp()
                .presentation_outer_split
                .borrow()
                .as_ref()
                .unwrap()
                .is_collapsed());
            assert!(!window
                .imp()
                .presentation_outer_split
                .borrow()
                .as_ref()
                .unwrap()
                .shows_sidebar());
            assert!(window
                .imp()
                .presentation_inner_split
                .borrow()
                .as_ref()
                .unwrap()
                .is_collapsed());
            assert!(!window
                .imp()
                .presentation_inner_split
                .borrow()
                .as_ref()
                .unwrap()
                .shows_sidebar());
            assert!(!entry.is_mapped(), "search makes way at {theme} {width}: scope={:?}, page={:?}, header active={}, survivors={}", window.app_state().borrow().scope, window.imp().content_stack.borrow().as_ref().unwrap().visible_child_name(), window.imp().mode_header.borrow().as_ref().unwrap().active(), page.survivors().len());
            assert!(!window
                .imp()
                .search_chip_bar
                .borrow()
                .as_ref()
                .unwrap()
                .is_mapped());
            assert!(
                page.header().start.is_ancestor(&header),
                "controls belong to the top bar"
            );
            let controls = page.header().start.compute_bounds(&window).unwrap();
            assert!(
                controls.x() >= 0.0 && controls.x() + controls.width() <= window.width() as f32
            );
            assert_eq!(
                window.handle_mode_key(gdk4::Key::Left, none),
                glib::Propagation::Stop
            );
            assert_eq!(page.imp().shown.borrow()[0].as_ref(), Some(&paths[0]));
            assert!(!page.survivors().contains(&paths[1]), "left keeps left");
            window.handle_mode_key(gdk4::Key::z, gdk4::ModifierType::CONTROL_MASK);
            assert_eq!(page.survivors(), paths);
            window.handle_mode_key(gdk4::Key::Right, none);
            assert_eq!(page.imp().shown.borrow()[1].as_ref(), Some(&paths[1]));
            assert!(!page.survivors().contains(&paths[0]), "right keeps right");
            page.undo();
            page.imp().keep[0].emit_clicked();
            assert!(
                !page.survivors().contains(&paths[1]),
                "the matching button does the same"
            );
            page.undo();
            assert_eq!(
                window.handle_mode_key(gdk4::Key::Left, gdk4::ModifierType::ALT_MASK),
                glib::Propagation::Proceed
            );
            assert_eq!(page.survivors(), paths);
            page.imp().candidates.list.grab_focus();
            assert_eq!(
                window.handle_mode_key(gdk4::Key::Right, none),
                glib::Propagation::Proceed,
                "candidate arrows never choose a winner"
            );
            settle(200);
            page.view().grab_focus();
            screenshot(&window, &root.join(format!("{theme}-{width}-compare.png")));
            page.close();
            pump_until(|| entry.is_mapped());
            assert_eq!(entry.text(), query);
            assert_eq!(window.imp().browse_options.borrow().chips, chips);
            crate::ui::crop_mode::start(&window, paths[0].clone());
            pump_until(|| window.crop_session().is_some());
            settle(500);
            assert!(!entry.is_mapped());
            let save = find_widgets::<gtk4::Button>(header.upcast_ref())
                .into_iter()
                .find(|b| b.label().as_deref() == Some("Save Copy"))
                .unwrap();
            let bounds = save.compute_bounds(&window).unwrap();
            assert!(bounds.y() < 42.0 && bounds.x() + bounds.width() <= window.width() as f32);
            screenshot(&window, &root.join(format!("{theme}-{width}-crop.png")));
            save.grab_focus();
            assert_eq!(
                window.handle_mode_key(gdk4::Key::Escape, none),
                glib::Propagation::Stop
            );
            pump_until(|| entry.is_mapped());
            assert_eq!(entry.text(), query);
            assert_eq!(window.imp().browse_options.borrow().chips, chips);
        }
    }
    crate::ui::crop_mode::start(&window, paths[0].clone());
    pump_until(|| window.crop_session().is_some());
    let save = find_widgets::<gtk4::Button>(header.upcast_ref())
        .into_iter()
        .find(|b| b.label().as_deref() == Some("Save Copy"))
        .unwrap();
    save.grab_focus();
    assert_eq!(
        window.handle_mode_key(gdk4::Key::Return, none),
        glib::Propagation::Stop
    );
    pump_until(|| {
        std::fs::read_dir(&output)
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.path().extension().is_some_and(|e| e == "webp"))
    });
    let copy = std::fs::read_dir(&output)
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.path().extension().is_some_and(|e| e == "webp"))
        .unwrap()
        .path();
    assert_eq!(
        image::open(&copy).unwrap().to_rgb8(),
        image::open(&paths[0]).unwrap().to_rgb8()
    );
    assert!(window.crop_session().is_none());
    for (path, bytes) in paths.iter().zip(originals) {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    window.close();
}

/// A real Fontconfig configuration with no font directories proves that
/// Pango loads the embedded files, rather than a matching desktop font.
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn bundled_fonts_work_without_system_fonts() {
    let root = isolated_fixture("bundled-fonts");
    std::fs::create_dir_all(&root).unwrap();
    let config = root.join("fonts.conf");
    std::fs::write(
        &config,
        format!(
            "<?xml version=\"1.0\"?><!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n\
             <fontconfig><reset-dirs/><cachedir>{}</cachedir></fontconfig>",
            root.join("font-cache").display()
        ),
    )
    .unwrap();
    // Nextest starts this test in its own process, before GTK/Pango starts.
    std::env::set_var("FONTCONFIG_FILE", config);
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let window = gtk4::Window::new();
    let context = window.pango_context();
    let font_map = context.font_map().unwrap();
    // Pango still lists generic families such as "Sans" in an empty map.
    assert!(
        !font_map
            .list_families()
            .iter()
            .any(|f| f.name().contains("JetBrains")),
        "the UI font is unavailable before loading the bundled files"
    );
    crate::ui::look::install(&window);
    // Loading again must leave the original anonymous files alive.
    crate::ui::look::install(&window);
    for weight in [
        pango::Weight::Normal,
        pango::Weight::Medium,
        pango::Weight::Bold,
    ] {
        for style in [pango::Style::Normal, pango::Style::Italic] {
            let mut description = pango::FontDescription::from_string("JetBrains Mono 13");
            description.set_weight(weight);
            description.set_style(style);
            let font = context
                .load_font(&description)
                .expect("embedded font is usable");
            assert_eq!(font.describe().family().as_deref(), Some("JetBrains Mono"));
            assert_eq!(font.describe().weight(), weight);
            assert_eq!(font.describe().style(), style);
            for character in "Sharpr ÆÞÐ 3840×2160 →".chars() {
                assert!(font.has_char(character), "missing {character}");
            }
        }
    }
    window.add_css_class("sharpr");
    window.add_css_class("sharpr-mono-all");
    let label = gtk4::Label::new(Some("Sharpr  ·  3840×2160  ·  ÆÞÐ  ·  Ctrl+?"));
    window.set_child(Some(&label));
    window.set_default_size(800, 160);
    window.present();
    pump_until(|| window.is_mapped());
    settle(500);
    screenshot_widget(&window, label.upcast_ref(), &root.join("bundled-font.png"));
    window.close();
}

/// The sheets must belong to the styled window, preserve settings wiring,
/// and expose the single library-root action.
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn preferences_and_shortcuts_share_the_main_window() {
    let root = isolated_fixture("styled-sheets");
    std::fs::create_dir_all(&root).unwrap();
    image::RgbImage::from_pixel(96, 54, image::Rgb([40, 90, 130]))
        .save(root.join("sample.png"))
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
    window.set_default_size(1024, 600);
    window.present();
    pump_until(|| window.is_mapped() && window.app_state().borrow().library.image_count() == 1);
    assert_eq!(
        app.accels_for_action("win.show-help-overlay"),
        ["<Control>question"]
    );
    let out = root.join("screenshots");
    std::fs::create_dir_all(&out).unwrap();
    for (scheme, theme) in [
        (libadwaita::ColorScheme::ForceLight, "light"),
        (libadwaita::ColorScheme::ForceDark, "dark"),
    ] {
        libadwaita::StyleManager::default().set_color_scheme(scheme);
        for (width, height) in [(1024, 600), (1600, 900)] {
            window.set_default_size(width, height);
            settle(300);
            gtk4::prelude::WidgetExt::activate_action(&window, "win.show-preferences", None)
                .unwrap();
            pump_until(|| window.visible_dialog().is_some());
            let prefs = window
                .visible_dialog()
                .unwrap()
                .downcast::<libadwaita::PreferencesDialog>()
                .unwrap();
            pump_until(|| prefs.is_mapped());
            assert_eq!(prefs.root(), Some(window.clone().upcast()));
            assert_eq!(app.windows().len(), 1, "no separate Preferences window");
            let root_button = find_widgets::<gtk4::Button>(prefs.upcast_ref())
                .into_iter()
                .find(|button| button.action_name().as_deref() == Some("win.choose-library-root"))
                .expect("one root chooser shares the window action");
            assert!(root_button.is_sensitive());
            settle(250);
            screenshot(
                &window,
                &out.join(format!("{theme}-{width}-preferences-library.png")),
            );
            let gpu = find_widgets::<libadwaita::SwitchRow>(prefs.upcast_ref())
                .into_iter()
                .find(|row| row.title() == "Use Graphics Card")
                .unwrap();
            let original = gpu.is_active();
            gpu.set_active(!original);
            assert_eq!(
                window.app_state().borrow().settings.visual_search_gpu,
                !original
            );
            gpu.set_active(original);
            // Read the effective font from a rendered label, rather than just
            // checking a class that might not reach the dialog's contents.
            let label = find_widgets::<gtk4::Label>(prefs.upcast_ref())
                .into_iter()
                .find(|l| l.text() == "Library Root")
                .unwrap();
            assert!(label
                .pango_context()
                .font_description()
                .unwrap()
                .family()
                .unwrap()
                .contains("JetBrains"));
            screenshot(
                &window,
                &out.join(format!("{theme}-{width}-preferences-settings.png")),
            );
            prefs.close();
            pump_until(|| window.visible_dialog().is_none());
            gtk4::prelude::WidgetExt::activate_action(&window, "win.show-help-overlay", None)
                .unwrap();
            pump_until(|| window.visible_dialog().is_some());
            let shortcuts = window.visible_dialog().unwrap();
            assert!(shortcuts.is::<libadwaita::ShortcutsDialog>());
            assert_eq!(shortcuts.root(), Some(window.clone().upcast()));
            assert_eq!(app.windows().len(), 1, "no separate Shortcuts window");
            pump_until(|| shortcuts.is_mapped());
            settle(250);
            screenshot(&window, &out.join(format!("{theme}-{width}-shortcuts.png")));
            shortcuts.close();
            pump_until(|| window.visible_dialog().is_none());
        }
    }
    window.close();
}

/// Phase 9 inventory: screenshots of every surface and state, at 1024×600
/// and 1600×900, light and dark, into `tour/` beside the fixtures. Generated
/// pictures only; nothing is asserted beyond reaching each state.
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn interface_tour_screenshots() {
    let root = isolated_fixture("Wallpapers");
    let phone = root.join("Phone");
    std::fs::create_dir_all(&phone).unwrap();
    let out = root.parent().unwrap().join("tour");
    std::fs::create_dir_all(&out).unwrap();
    let dawn = [
        [40, 60, 120],
        [240, 150, 110],
        [255, 230, 170],
        [50, 45, 70],
    ];
    let night = [[8, 12, 35], [40, 50, 110], [235, 235, 210], [15, 20, 40]];
    let desert = [
        [70, 140, 210],
        [230, 200, 150],
        [255, 245, 215],
        [170, 100, 60],
    ];
    let forest = [
        [150, 175, 170],
        [205, 215, 200],
        [245, 245, 230],
        [40, 75, 55],
    ];
    let aurora = [[10, 30, 40], [40, 160, 130], [190, 255, 220], [20, 30, 45]];
    let coast = [[90, 60, 120], [250, 120, 80], [255, 210, 120], [35, 40, 75]];
    let files: [(&Path, &str, u32, u32, Palette, f32); 10] = [
        (&root, "alpine-dawn.png", 1920, 1080, dawn, 0.2),
        (&root, "harbour-night.jpg", 1920, 1080, night, 0.7),
        (&root, "desert-noon.jpg", 3840, 2160, desert, 0.5),
        (&root, "forest-mist.png", 1080, 1350, forest, 0.4),
        (&root, "aurora-ultrawide.png", 2560, 1080, aurora, 0.9),
        (&root, "coast-sunset.png", 1920, 1080, coast, 0.1),
        (&root, "city-dusk.jpg", 1600, 900, night, 0.3),
        (&root, "square-tile.png", 1200, 1200, desert, 0.8),
        (&phone, "dawn-portrait.png", 1080, 1920, dawn, 0.6),
        (&phone, "aurora-portrait.png", 1080, 1920, aurora, 0.2),
    ];
    for (dir, name, w, h, palette, phase) in files {
        scene(w, h, palette, phase).save(dir.join(name)).unwrap();
    }
    // An exact copy for Find Copies.
    std::fs::copy(
        root.join("alpine-dawn.png"),
        root.join("alpine-dawn (1).png"),
    )
    .unwrap();

    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let style = libadwaita::StyleManager::default();
    style.set_color_scheme(libadwaita::ColorScheme::ForceLight);
    let mut settings = AppSettings::load();
    settings.libraries.clear();
    settings
        .add_library("Pictures", root.clone(), FolderMode::DrillDown)
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
    let count = || window.imp().state.borrow().library.image_count();
    pump_until(|| count() >= 1 && window.is_mapped());
    // Full screen, so the sidebar shows (user, October 15).
    window.maximize();
    pump_until(|| window.is_maximized());
    let state = window.app_state();
    let filmstrip = window.imp().filmstrip.borrow().clone().unwrap();
    let viewer = window.imp().viewer.borrow().clone().unwrap();
    let sidebar = window.imp().sidebar.borrow().clone().unwrap();
    let path_at = |name: &str| root.join(name);
    let go_to = |name: &str| {
        let index = state
            .borrow()
            .library
            .index_of_path(&path_at(name))
            .unwrap();
        filmstrip.navigate_to(index);
    };
    let shot = |name: &str| {
        settle(500);
        screenshot(&window, &out.join(format!("{name}.png")));
    };
    let shot_widget = |widget: &gtk4::Widget, name: &str| {
        settle(400);
        screenshot_widget(&window, widget, &out.join(format!("{name}.png")));
    };
    let close_dialog = || {
        if let Some(dialog) = window.visible_dialog() {
            dialog.force_close();
        }
        pump_until(|| window.visible_dialog().is_none());
    };
    // Look prototype: quick pickers are popovers.
    let picker = || {
        find_widgets::<gtk4::Popover>(window.upcast_ref())
            .into_iter()
            .find(|p| p.is_visible() && p.has_css_class("sharpr-picker"))
    };
    let menu_button = |tooltip: &str| {
        find_widgets::<gtk4::MenuButton>(window.upcast_ref())
            .into_iter()
            .find(|b| b.tooltip_text().as_deref() == Some(tooltip))
    };

    // Buckets, so the sidebar and chips show them.
    for (name, members) in [
        (
            "Favourites",
            vec![
                "alpine-dawn.png",
                "aurora-ultrawide.png",
                "coast-sunset.png",
            ],
        ),
        (
            "Desktop 16:9",
            vec![
                "alpine-dawn.png",
                "harbour-night.jpg",
                "desert-noon.jpg",
                "coast-sunset.png",
            ],
        ),
    ] {
        let paths = members.iter().map(|m| path_at(m)).collect();
        window.create_collection_with_paths(name.into(), Vec::new(), paths);
        pump_until(|| state.borrow().collections.iter().any(|c| c.name == name));
    }
    let open_root = || {
        window.imp().state.borrow_mut().library.current_folder = None;
        if let Some(open) = window.imp().open_folder_cb.borrow().as_ref() {
            open(root.clone());
        }
        pump_until(|| count() == 9);
    };
    open_root();
    // Buckets and Find Copies work from fingerprints.
    let index = state.borrow().library_index.clone().unwrap();
    crate::fingerprint::run_pass(&index, &root, &[], &[], &|| false, &|_| {}).unwrap();
    go_to("alpine-dawn.png");
    pump_until(|| viewer.imp().metadata_chip.is_visible());
    // Let the "Added to bucket" toasts pass, and every thumbnail load.
    settle(11_000);

    // 01: the main window, full screen and small, both themes.
    viewer.reveal_chip();
    shot("01-main-full-light");
    style.set_color_scheme(libadwaita::ColorScheme::ForceDark);
    shot("01-main-full-dark");
    window.unmaximize();
    window.set_default_size(1024, 600);
    let small = std::time::Instant::now() + std::time::Duration::from_secs(3);
    pump_until(|| window.width() == 1024 || std::time::Instant::now() > small);
    shot("01-main-1024-dark");
    style.set_color_scheme(libadwaita::ColorScheme::ForceLight);
    shot("01-main-1024-light");
    window.maximize();
    pump_until(|| window.is_maximized());

    // 02: menus and popovers.
    for (tooltip, name) in [
        ("Main Menu", "02-main-menu"),
        ("Sort and filter", "02-sort-and-filter-menu"),
        ("Switch Library", "02-library-switcher"),
    ] {
        let Some(button) = menu_button(tooltip) else {
            // The look prototype merges Filter into Sort.
            continue;
        };
        let mapped = std::time::Instant::now() + std::time::Duration::from_secs(2);
        pump_until(|| button.is_mapped() || std::time::Instant::now() > mapped);
        if !button.is_mapped() {
            // The look prototype hides the library switcher.
            continue;
        }
        button.popup();
        let popover = button.popover().unwrap();
        pump_until(|| popover.is_visible());
        shot_widget(popover.upcast_ref(), name);
        button.popdown();
    }
    assert!(filmstrip.open_context_menu_for_current());
    let context = || {
        find_widgets::<gtk4::PopoverMenu>(filmstrip.upcast_ref())
            .into_iter()
            .find(|p| p.is_visible())
    };
    pump_until(|| context().is_some());
    shot_widget(context().unwrap().upcast_ref(), "02-right-click-one");
    context().unwrap().popdown();
    viewer.show_info();
    let info = viewer.imp().metadata_chip.imp().details.clone();
    pump_until(|| info.is_visible());
    shot_widget(info.upcast_ref(), "02-info-panel");
    viewer.imp().metadata_chip.hide_info();

    // 03: a selection of four, its right-click menu.
    for name in [
        "alpine-dawn.png",
        "harbour-night.jpg",
        "desert-noon.jpg",
        "coast-sunset.png",
    ] {
        filmstrip.toggle_action_selection_path(&path_at(name));
    }
    shot("03-selection");
    assert!(filmstrip.open_context_menu_for_current());
    pump_until(|| context().is_some());
    shot_widget(context().unwrap().upcast_ref(), "03-right-click-several");
    context().unwrap().popdown();

    // 04: Compare in its three modes.
    let selected: Vec<PathBuf> = filmstrip.selected_paths_for_action();
    gio::prelude::ActionGroupExt::activate_action(&window, "compare", None);
    let page = window.imp().compare_page.borrow().clone().unwrap();
    pump_until(|| page.survivors().len() == selected.len());
    // The 4K JPEG decodes slowly in a debug build.
    settle(3000);
    shot("04-compare-side-by-side");
    page.imp().mode_buttons.borrow()[1].set_active(true);
    shot("04-compare-split");
    page.imp().mode_buttons.borrow()[2].set_active(true);
    shot("04-compare-flip");
    style.set_color_scheme(libadwaita::ColorScheme::ForceDark);
    page.imp().mode_buttons.borrow()[0].set_active(true);
    shot("04-compare-side-by-side-dark");
    style.set_color_scheme(libadwaita::ColorScheme::ForceLight);
    page.close();
    pump_until(|| {
        window
            .imp()
            .compare_page
            .borrow()
            .as_ref()
            .is_none_or(|p| !p.is_mapped())
    });
    filmstrip.clear_multi_selection();

    // 05: crop at full size.
    go_to("coast-sunset.png");
    window.present_copy_dialog(
        crate::ui::filmstrip::CopyKind::Crop,
        vec![path_at("coast-sunset.png")],
    );
    pump_until(|| window.crop_session().is_some());
    shot("05-crop");
    window.crop_session().unwrap().close();
    pump_until(|| window.crop_session().is_none());

    // 06: search, with chips and the results line.
    let entry = window.imp().inline_search_entry.borrow().clone().unwrap();
    window.show_inline_search();
    entry.set_text("dawn 16:9 -water ");
    settle(800);
    shot("06-search-chips");
    window.add_search_results_to_bucket();
    pump_until(|| picker().is_some());
    shot_widget(picker().unwrap().upcast_ref(), "06-add-results-choice");
    picker().unwrap().popdown();
    window.close_browse_search();
    settle(400);

    // 07: a bucket, and the dialogs that act on images.
    let favourites = state
        .borrow()
        .collections
        .iter()
        .find(|c| c.name == "Favourites")
        .map(|c| c.id)
        .unwrap();
    sidebar.emit_collection_selected(favourites);
    // Four: the exact copy shares its fingerprint, so it belongs too.
    pump_until(|| window.imp().bucket_open_pending.get().is_none() && count() == 4);
    shot("07-bucket-view");
    filmstrip.request_add_paths_to_bucket(vec![path_at("city-dusk.jpg")]);
    pump_until(|| picker().is_some());
    shot_widget(picker().unwrap().upcast_ref(), "07-bucket-picker");
    picker().unwrap().popdown();
    crate::ui::export_dialog::present(
        &window,
        vec![path_at("alpine-dawn.png"), path_at("city-dusk.jpg")],
    );
    pump_until(|| window.visible_dialog().is_some());
    shot("07-export");
    close_dialog();
    let new_bucket = find_widgets::<gtk4::Button>(sidebar.upcast_ref())
        .into_iter()
        .find(|b| b.tooltip_text().as_deref() == Some("New Bucket"))
        .unwrap();
    new_bucket.emit_clicked();
    pump_until(|| window.visible_dialog().is_some());
    shot("07-new-bucket");
    close_dialog();
    sidebar.select_folder(&root);
    open_root();

    // 08: library tools.
    crate::ui::copies_dialog::present(&window);
    pump_until(|| window.visible_dialog().is_some());
    let copies: gtk4::Widget = window.visible_dialog().unwrap().upcast();
    pump_until(|| row_subtitle(&copies, "alpine-dawn (1).png").is_some());
    shot("08-find-copies");
    close_dialog();
    crate::ui::library_overview::present(&window);
    pump_until(|| window.visible_dialog().is_some());
    settle(800);
    shot("08-library-overview");
    close_dialog();
    crate::ui::help_window::show_help_window(&window);
    pump_until(|| window.visible_dialog().is_some());
    shot("08-manual");
    close_dialog();
    gtk4::prelude::WidgetExt::activate_action(&window, "win.show-help-overlay", None).unwrap();
    pump_until(|| window.visible_dialog().is_some());
    shot("08-shortcuts");
    close_dialog();
    gtk4::prelude::WidgetExt::activate_action(&window, "win.show-preferences", None).unwrap();
    pump_until(|| window.visible_dialog().is_some());
    let prefs = window
        .visible_dialog()
        .unwrap()
        .downcast::<libadwaita::PreferencesDialog>()
        .unwrap();
    shot("08-preferences-library");
    if let Some(page) = find_widgets::<libadwaita::PreferencesPage>(prefs.upcast_ref())
        .into_iter()
        .find(|p| p.title() == "Appearance")
    {
        prefs.set_visible_page(&page);
        shot("08-preferences-appearance");
    }
    close_dialog();

    // 10: the readout face everywhere (the look prototype's variant).
    go_to("alpine-dawn.png");
    window.set_mono_everywhere(true);
    viewer.reveal_chip();
    shot("10-mono-all-main");
    if let Some(button) = menu_button("Sort and filter") {
        button.popup();
        let popover = button.popover().unwrap();
        pump_until(|| popover.is_visible());
        shot_widget(popover.upcast_ref(), "10-mono-all-sort-menu");
        button.popdown();
    }
    assert!(filmstrip.open_context_menu_for_current());
    pump_until(|| context().is_some());
    shot_widget(context().unwrap().upcast_ref(), "10-mono-all-right-click");
    context().unwrap().popdown();
    window.set_mono_everywhere(false);

    // 09: presentation (fullscreen).
    go_to("aurora-ultrawide.png");
    window.enter_presentation_mode();
    settle(1000);
    shot("09-presentation");
    window.exit_presentation_mode();
    settle(600);
    window.close();
}

#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn repeat_bucket_and_expanding_info_preserve_originals() {
    let root = isolated_fixture("repeat-info");
    std::fs::create_dir_all(&root).unwrap();
    let paths: Vec<_> = (0..2)
        .map(|i| {
            let path = root.join(format!("image-{i}.png"));
            image::RgbImage::from_pixel(160, 90, image::Rgb([70, 100, 150]))
                .save(&path)
                .unwrap();
            path
        })
        .collect();
    let originals: Vec<_> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let mut settings = AppSettings::load();
    settings.libraries.clear();
    let library = settings
        .add_library("Test", root.clone(), FolderMode::DrillDown)
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
    pump_until(|| window.app_state().borrow().library.image_count() == 2 && window.is_mapped());
    let index = window.app_state().borrow().library_index.clone().unwrap();
    let bucket = index
        .create_collection(&library, None, "Wallpapers", &[], None, None)
        .unwrap();
    window
        .app_state()
        .borrow_mut()
        .collections
        .push(bucket.clone());
    window.change_collection_tags(bucket.id, vec![paths[0].clone()], false);
    pump_until(|| window.app_state().borrow().settings.last_bucket() == Some(bucket.id));
    let strip = window.imp().filmstrip.borrow().clone().unwrap();
    strip.set_action_selection_to_path(&paths[1]);
    let entry = window.imp().inline_search_entry.borrow().clone().unwrap();
    window.show_inline_search();
    entry.grab_focus();
    settle(150);
    assert!(
        !window_shortcut(&window, "<Control><Shift>b"),
        "typing blocks repeat-add"
    );
    strip.imp().list_view.grab_focus();
    settle(150);
    assert!(window_shortcut(&window, "<Control><Shift>b"));
    pump_until(|| {
        crate::buckets::view(&index, bucket.id, Some(&root), &[])
            .unwrap()
            .paths
            .len()
            == 2
    });
    window.app_state().borrow_mut().disabled_folders = vec![root.clone()];
    window.add_to_last_bucket(vec![paths[0].clone()]);
    settle(150);
    assert_eq!(
        crate::buckets::view(&index, bucket.id, Some(&root), &[])
            .unwrap()
            .paths
            .len(),
        2
    );
    window.app_state().borrow_mut().disabled_folders.clear();
    let viewer = window.imp().viewer.borrow().clone().unwrap();
    let chip = &viewer.imp().metadata_chip;
    pump_until(|| chip.imp().has_metadata.get());
    pump_until(|| chip.imp().buckets_label.text().contains("Wallpapers"));
    chip.set_hovered(true);
    pump_until(|| chip.is_info_open());
    assert!(
        find_widgets::<gtk4::Popover>(chip.upcast_ref()).is_empty(),
        "details stay inside the chip"
    );
    chip.set_hovered(false);
    pump_until(|| !chip.is_info_open());
    assert!(chip.show_info());
    chip.set_hovered(false);
    settle(250);
    assert!(chip.is_info_open(), "keyboard/click info stays pinned");
    for (theme, scheme) in [
        ("light", libadwaita::ColorScheme::ForceLight),
        ("dark", libadwaita::ColorScheme::ForceDark),
    ] {
        libadwaita::StyleManager::default().set_color_scheme(scheme);
        for (width, height) in [(1024, 600), (1600, 900)] {
            window.set_default_size(width, height);
            pump_until(|| window.width() == width && window.height() == height);
            settle(100);
            let bounds = chip.compute_bounds(&window).unwrap();
            assert!(bounds.x() >= 0.0 && bounds.x() + bounds.width() <= width as f32);
            screenshot(
                &window,
                &root
                    .parent()
                    .unwrap()
                    .join(format!("expanded-info-{theme}-{width}.png")),
            );
        }
    }
    assert!(viewer.hide_info());
    chip.set_hovered(true);
    chip.clear();
    settle(250);
    assert!(
        !chip.is_info_open(),
        "a delayed hover cannot reopen a cleared image"
    );
    for (path, bytes) in paths.iter().zip(originals) {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    window.close();
}

#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn survey_narrows_selection_and_restores_browsing_without_file_changes() {
    let root = isolated_fixture("survey");
    std::fs::create_dir_all(&root).unwrap();
    let paths: Vec<_> = (0..8)
        .map(|i| {
            let path = root.join(format!("scene-{i}.png"));
            let (width, height) = if i % 3 == 0 { (180, 280) } else { (320, 180) };
            image::RgbImage::from_fn(width, height, |x, y| {
                image::Rgb([x as u8, (i * 25) as u8, y as u8])
            })
            .save(&path)
            .unwrap();
            path
        })
        .collect();
    let originals: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let mut settings = AppSettings::load();
    settings.libraries.clear();
    settings
        .add_library("Survey", root.clone(), FolderMode::DrillDown)
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
    pump_until(|| window.app_state().borrow().library.image_count() == 8 && window.is_mapped());
    let strip = window.imp().filmstrip.borrow().clone().unwrap();
    strip.imp().list_view.grab_focus();
    assert!(!window_shortcut(&window, "<Control><Shift>s"));
    window.show_inline_search();
    let entry = window.imp().inline_search_entry.borrow().clone().unwrap();
    entry.set_text("png ");
    pump_until(|| {
        !window.imp().browse_options.borrow().chips.is_empty()
            && window.app_state().borrow().library.image_count() == 8
            && window.app_state().borrow().library.current_folder.is_none()
    });
    let chips = window.imp().browse_options.borrow().chips.clone();
    strip.select_all();
    strip.imp().list_view.grab_focus();
    settle(150);
    let ordered: Vec<_> = (0..8)
        .map(|i| {
            window
                .app_state()
                .borrow()
                .library
                .entry_at(i)
                .unwrap()
                .path()
        })
        .collect();
    assert_eq!(window.app_state().borrow().selected_paths.len(), 8);
    assert!(window_shortcut(&window, "<Control><Shift>s"));
    assert_eq!(window.app_state().borrow().scope, ViewScope::Survey);
    let page = window.imp().survey_page.borrow().clone().unwrap();
    assert_eq!(page.survivors(), ordered);
    assert!(super::file_actions_blocked(window.upcast_ref()));
    assert!(!entry.is_mapped());
    pump_until(|| {
        find_widgets::<gtk4::Picture>(page.root.upcast_ref())
            .iter()
            .filter(|p| p.paintable().is_some())
            .count()
            == 8
    });
    page.dismiss(3);
    assert_eq!(page.store.n_items(), 7);
    assert!(!page.survivors().contains(&ordered[3]));
    page.undo();
    assert_eq!(page.survivors(), ordered);
    page.grid
        .model()
        .and_downcast::<gtk4::SingleSelection>()
        .unwrap()
        .set_selected(2);
    assert_eq!(
        window.handle_mode_key(gdk4::Key::Delete, gdk4::ModifierType::empty()),
        glib::Propagation::Stop
    );
    assert!(!page.survivors().contains(&ordered[2]));
    window.handle_mode_key(gdk4::Key::z, gdk4::ModifierType::CONTROL_MASK);
    assert_eq!(page.survivors(), ordered);
    for (theme, scheme) in [
        ("light", libadwaita::ColorScheme::ForceLight),
        ("dark", libadwaita::ColorScheme::ForceDark),
    ] {
        libadwaita::StyleManager::default().set_color_scheme(scheme);
        for (width, height) in [(1024, 600), (1600, 900)] {
            window.set_default_size(width, height);
            pump_until(|| window.width() == width && window.height() == height);
            settle(200);
            screenshot(
                &window,
                &root
                    .parent()
                    .unwrap()
                    .join(format!("survey-{theme}-{width}.png")),
            );
            let bounds = page.header.end.compute_bounds(&window).unwrap();
            assert!(bounds.x() >= 0.0 && bounds.x() + bounds.width() <= width as f32);
        }
    }
    for number in 0..8 {
        page.dismiss(number);
    }
    assert!(page.survivors().is_empty());
    page.undo();
    page.undo();
    assert_eq!(page.survivors(), ordered[6..8]);
    window.handle_mode_key(gdk4::Key::Escape, gdk4::ModifierType::empty());
    pump_until(|| window.app_state().borrow().scope == ViewScope::Folder(root.clone()));
    assert_eq!(
        window.app_state().borrow().selected_paths,
        ordered[6..8].iter().cloned().collect()
    );
    assert_eq!(window.imp().browse_options.borrow().chips, chips);
    assert!(entry.is_mapped());
    for (path, bytes) in paths.iter().zip(originals) {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    window.close();
}

#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn one_root_preserves_library_identity_members_fingerprints_and_vectors() {
    let root = isolated_fixture("one-root");
    let narrow = root.join("keep");
    let deep = narrow.join("nested");
    let disabled = root.join("disabled");
    let blocked = narrow.join("blocked");
    for path in [&root, &narrow, &deep, &disabled, &blocked] {
        std::fs::create_dir_all(path).unwrap();
    }
    let paths = [
        root.join("outside.png"),
        narrow.join("inside.png"),
        deep.join("deep.png"),
        disabled.join("secret.png"),
        blocked.join("blocked.png"),
    ];
    for (i, path) in paths.iter().enumerate() {
        image::RgbImage::from_pixel(64, 48, image::Rgb([i as u8 * 40, 90, 150]))
            .save(path)
            .unwrap();
    }
    let originals: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let mut settings = AppSettings::load();
    settings.libraries.clear();
    let library = settings
        .add_library("Original identity", root.clone(), FolderMode::TopLevel)
        .unwrap();
    settings.set_active_library_last_folder(Some(root.clone()));
    settings.set_active_library_ignored_folders(vec![disabled.clone(), blocked.clone()]);
    let mut inactive = settings.active_library().unwrap().clone();
    inactive.id = "inactive-preserved".into();
    inactive.name = "Retired configuration".into();
    inactive.root = deep.clone();
    settings.libraries.push(inactive.clone());
    settings.save();
    let app = libadwaita::Application::builder()
        .application_id("io.github.hebbihebb.Sharpr.GtkTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let window = SharprWindow::new(&app);
    window.set_default_size(1024, 600);
    window.present();
    pump_until(|| window.app_state().borrow().library.image_count() == 1 && window.is_mapped());
    let index = window.app_state().borrow().library_index.clone().unwrap();
    let tags = window.app_state().borrow().tags.clone().unwrap();
    tags.add_tag(&paths[0], "legacy-kept");
    let tag_rows = tags.stored_rows().unwrap();
    let bucket = index
        .create_collection(&library, None, "Kept", &[], None, None)
        .unwrap();
    crate::buckets::add_to_bucket(&index, bucket.id, &paths[..3]).unwrap();
    let members = index.all_bucket_members().unwrap();
    let fingerprints = index.cached_fingerprints().unwrap();
    let vectors =
        crate::visual_search::store::Store::open(&crate::visual_search::store::default_path())
            .unwrap();
    for row in &members {
        vectors
            .put_vector(
                "root-test",
                row.fingerprint.as_ref().unwrap(),
                &vec![0.25; crate::visual_search::DIMENSIONS],
            )
            .unwrap();
    }
    let before_vectors = vectors.vectors("root-test").unwrap();
    let sidebar = window.imp().sidebar.borrow().clone().unwrap();
    // Even a legacy top-level config now exposes deep folders. Expanding
    // doesn't decode files or enter disabled children.
    sidebar.imp().collapsed_folder_paths.borrow_mut().clear();
    sidebar.refresh_active_library(window.app_state());
    settle(150);
    let visible_folder_paths = || {
        find_widgets::<crate::ui::sidebar::FolderRow>(sidebar.upcast_ref())
            .iter()
            .map(|row| row.path())
            .collect::<Vec<_>>()
    };
    // Root is initially collapsed by the default tree. Open it and keep.
    let expand = |path: &Path| {
        let row = find_widgets::<crate::ui::sidebar::FolderRow>(sidebar.upcast_ref())
            .into_iter()
            .find(|row| row.path() == path)
            .unwrap();
        row.disclosure_button().unwrap().emit_clicked();
    };
    expand(&narrow);
    pump_until(|| visible_folder_paths().contains(&deep));
    assert!(window.lookup_action("choose-library-root").is_some());
    assert!(window
        .change_library_root(PathBuf::from("relative"))
        .is_err());
    window.change_library_root(narrow.clone()).unwrap();
    pump_until(|| {
        window
            .app_state()
            .borrow()
            .settings
            .active_library()
            .unwrap()
            .root
            == narrow
            && window.app_state().borrow().library.image_count() == 1
    });
    let view = crate::buckets::view(
        &index,
        bucket.id,
        Some(&narrow),
        &[disabled.clone(), blocked.clone()],
    )
    .unwrap();
    assert_eq!(view.paths.len(), 2);
    assert!(!view.paths.contains(&paths[0]));
    assert_eq!(view.missing, 0);
    let reloaded = AppSettings::load();
    assert_eq!(
        reloaded.active_library_id.as_deref(),
        Some(library.as_str())
    );
    assert_eq!(
        reloaded.active_library().unwrap().folder_mode,
        FolderMode::TopLevel
    );
    assert_eq!(
        reloaded.active_library().unwrap().ignored_folders,
        vec![disabled.clone(), blocked.clone()]
    );
    assert_eq!(reloaded.libraries[1], inactive);
    // Narrow again then widen immediately: the old sidebar worker must not
    // restore its tree or open the stale root after the latest choice.
    window.change_library_root(deep.clone()).unwrap();
    window.change_library_root(root.clone()).unwrap();
    pump_until(|| {
        window.app_state().borrow().library.current_folder.as_ref() == Some(&deep)
            && visible_folder_paths().contains(&root)
    });
    settle(200);
    assert_eq!(
        window
            .app_state()
            .borrow()
            .settings
            .active_library()
            .unwrap()
            .root,
        root
    );
    let view = crate::buckets::view(
        &index,
        bucket.id,
        Some(&root),
        &[disabled.clone(), blocked.clone()],
    )
    .unwrap();
    assert_eq!((view.paths.len(), view.missing), (3, 0));
    assert_eq!(index.all_bucket_members().unwrap().len(), members.len());
    for (path, value) in fingerprints {
        assert_eq!(
            index.cached_fingerprints().unwrap().get(&path),
            Some(&value)
        );
    }
    assert_eq!(vectors.vectors("root-test").unwrap(), before_vectors);
    assert_eq!(tags.stored_rows().unwrap(), tag_rows);
    for (path, bytes) in paths.iter().zip(originals) {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    gio::prelude::ActionGroupExt::activate_action(&window, "show-preferences", None);
    pump_until(|| window.visible_dialog().is_some());
    let rows = find_widgets::<libadwaita::ActionRow>(window.visible_dialog().unwrap().upcast_ref());
    assert!(rows.iter().any(|row| row.title() == "Library Root"));
    assert!(!rows.iter().any(|row| matches!(
        row.title().as_str(),
        "Add Library" | "Export Buckets" | "Import Buckets"
    )));
    pump_until(|| {
        window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    settle(200);
    screenshot(
        &window,
        &root.parent().unwrap().join("one-root-preferences.png"),
    );
    window.visible_dialog().unwrap().close();
    window.close();
}

fn menu_rows(model: &gio::MenuModel) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for i in 0..model.n_items() {
        let label = model
            .item_attribute_value(i, "label", None)
            .and_then(|value| value.get::<String>());
        let action = model
            .item_attribute_value(i, "action", None)
            .and_then(|value| value.get::<String>());
        if let Some(label) = label {
            rows.push((label, action.unwrap_or_default()));
        }
        for link in ["section", "submenu"] {
            if let Some(child) = model.item_link(i, link) {
                rows.extend(menu_rows(&child));
            }
        }
    }
    rows
}

#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn phase_nine_help_viewer_menu_and_theme_layouts() {
    let root = isolated_fixture("phase-nine-polish");
    std::fs::create_dir_all(&root).unwrap();
    for i in 0..3 {
        image::RgbImage::from_fn(640, 360, |x, y| {
            image::Rgb([(x / 3) as u8, (y / 2) as u8, 70 + i * 50])
        })
        .save(root.join(format!("image-{i}.png")))
        .unwrap();
    }
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let mut settings = AppSettings::load();
    settings.libraries.clear();
    let library = settings
        .add_library("Polish", root.clone(), FolderMode::TopLevel)
        .unwrap();
    settings.set_active_library_last_folder(Some(root.clone()));
    settings.set_metadata_visible(false); // Retired switch must not hide the contextual readout.
    let app = libadwaita::Application::builder()
        .application_id("io.github.hebbihebb.Sharpr.GtkTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let window = SharprWindow::new(&app);
    window.set_default_size(1024, 600);
    window.present();
    pump_until(|| window.app_state().borrow().library.image_count() == 3 && window.is_mapped());
    let strip = window.imp().filmstrip.borrow().clone().unwrap();
    let viewer = window.imp().viewer.borrow().clone().unwrap();
    pump_until(|| {
        viewer.current_path().is_some()
            && viewer.imp().picture.paintable().is_some()
            && viewer.imp().metadata_chip.imp().has_metadata.get()
    });
    assert!(viewer.imp().metadata_chip.imp().enabled.get());
    assert!(
        !AppSettings::load().metadata_visible,
        "legacy key stays stored"
    );
    let icons = gtk4::IconTheme::for_display(&gdk4::Display::default().unwrap());
    for name in [
        "bucket",
        "add-bucket",
        "compare",
        "crop",
        "webp",
        "below-4k",
        "copies",
    ] {
        assert!(
            icons.has_icon(&format!("sharpr-{name}-symbolic")),
            "bundled {name} icon"
        );
    }
    let primary = menu_rows(SharprWindow::build_primary_menu().upcast_ref());
    assert!(primary.iter().any(|(label, _)| label == "Help"));
    assert!(!primary.iter().any(|(label, _)| matches!(
        label.as_str(),
        "Show Overlay" | "Fit to Window" | "Manual" | "Keyboard Shortcuts" | "About Sharpr"
    )));
    let path = viewer.current_path().unwrap();
    strip.set_action_selection_to_path(&path);
    assert!(strip.open_context_menu_for_current());
    let row_menu = find_widgets::<gtk4::PopoverMenu>(strip.upcast_ref())
        .into_iter()
        .find(|menu| menu.is_visible())
        .unwrap();
    let expected = menu_rows(&row_menu.menu_model().unwrap());
    row_menu.popdown();
    assert!(strip.open_viewer_menu(&viewer, 100.0, 100.0));
    let image_menu = &viewer.imp().image_menu;
    pump_until(|| image_menu.is_mapped());
    assert_eq!(menu_rows(&image_menu.menu_model().unwrap()), expected);
    settle(250);
    screenshot(
        &window,
        &root.parent().unwrap().join("viewer-image-menu.png"),
    );
    image_menu.popdown();
    // Invoke the shared bucket writer from the large viewer's action group.
    let index = window.app_state().borrow().library_index.clone().unwrap();
    let bucket = index
        .create_collection(&library, None, "Viewer choice", &[], None, None)
        .unwrap();
    window
        .app_state()
        .borrow_mut()
        .collections
        .push(bucket.clone());
    window
        .app_state()
        .borrow()
        .settings
        .remember_bucket(bucket.id);
    gtk4::prelude::WidgetExt::activate_action(&viewer, "filmstrip.add-last-bucket", None).unwrap();
    pump_until(|| index.all_bucket_members().unwrap().len() == 1);
    assert_eq!(index.all_bucket_members().unwrap()[0].last_path, path);
    window.enter_presentation_mode();
    pump_until(|| window.is_fullscreen());
    settle(350);
    viewer.grab_focus();
    if !strip.is_mapped() {
        assert!(!strip.open_context_menu_for_current());
    }
    assert!(window_shortcut(&window, "<Shift>F10|Menu"));
    pump_until(|| {
        find_widgets::<gtk4::PopoverMenu>(window.upcast_ref())
            .iter()
            .any(|popover| popover.is_mapped())
    });
    for popover in find_widgets::<gtk4::PopoverMenu>(window.upcast_ref()) {
        popover.popdown();
    }
    window.exit_presentation_mode();
    pump_until(|| !window.is_fullscreen());
    settle(350);
    let style = libadwaita::StyleManager::default();
    if std::env::var("ADW_DEBUG_HIGH_CONTRAST").as_deref() == Ok("1") {
        assert!(style.is_high_contrast());
        assert!(window.has_css_class("sharpr-high-contrast"));
    }
    for (theme, scheme) in [
        ("light", libadwaita::ColorScheme::ForceLight),
        ("dark", libadwaita::ColorScheme::ForceDark),
    ] {
        style.set_color_scheme(scheme);
        for (width, height) in [(1024, 600), (1600, 900)] {
            window.set_default_size(width, height);
            pump_until(|| window.width() == width && window.height() == height);
            settle(150);
            let menu = find_widgets::<gtk4::MenuButton>(window.upcast_ref())
                .into_iter()
                .find(|button| button.tooltip_text().as_deref() == Some("Main Menu"))
                .unwrap();
            let before = menu.compute_bounds(&window).unwrap();
            window.show_inline_search();
            let entry = window.imp().inline_search_entry.borrow().clone().unwrap();
            entry.set_text("png ");
            pump_until(|| {
                window.app_state().borrow().library.current_folder.is_none()
                    && window.app_state().borrow().library.image_count() == 3
            });
            settle(150);
            let after = menu.compute_bounds(&window).unwrap();
            assert_eq!((before.x(), before.y()), (after.x(), after.y()));
            assert!(after.x() + after.width() <= width as f32);
            screenshot(
                &window,
                &root
                    .parent()
                    .unwrap()
                    .join(format!("polish-{theme}-{width}.png")),
            );
            window.close_browse_search();
            settle(350);
            gtk4::prelude::WidgetExt::activate_action(&window, "win.show-help", None).unwrap();
            pump_until(|| {
                window
                    .visible_dialog()
                    .is_some_and(|dialog| dialog.is_mapped())
            });
            settle(150);
            let help = window.visible_dialog().unwrap();
            let rows = find_widgets::<libadwaita::ActionRow>(help.upcast_ref());
            assert_eq!(rows.len(), 5);
            screenshot(
                &window,
                &root
                    .parent()
                    .unwrap()
                    .join(format!("help-{theme}-{width}.png")),
            );
            rows.into_iter()
                .find(|row| row.title() == "Keyboard Shortcuts")
                .unwrap()
                .emit_by_name::<()>("activated", &[]);
            pump_until(|| {
                window
                    .visible_dialog()
                    .is_some_and(|dialog| dialog.is::<libadwaita::ShortcutsDialog>())
            });
            assert_eq!(app.windows().len(), 1);
            window.visible_dialog().unwrap().close();
            pump_until(|| window.visible_dialog().is_none());
        }
    }
    window.close();
}

/// An image smaller than the screen keeps filling the viewer when the view
/// grows (fullscreen, sidebar, resize), not only after a zoom (user, October
/// 15: a 2,048 px crop copy stayed small in fullscreen).
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn small_image_keeps_fitting_when_the_viewer_grows() {
    let root = isolated_fixture("fit-on-resize");
    std::fs::create_dir_all(&root).unwrap();
    image::RgbImage::from_pixel(320, 180, image::Rgb([40, 120, 200]))
        .save(root.join("small.png"))
        .unwrap();
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let mut settings = AppSettings::load();
    settings.libraries.clear();
    settings
        .add_library("Fit", root.clone(), FolderMode::TopLevel)
        .unwrap();
    settings.set_active_library_last_folder(Some(root.clone()));
    let app = libadwaita::Application::builder()
        .application_id("io.github.hebbihebb.Sharpr.GtkTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let window = SharprWindow::new(&app);
    window.set_default_size(1600, 900);
    window.present();
    let viewer = window.imp().viewer.borrow().clone().unwrap();
    pump_until(|| window.is_mapped() && viewer.imp().picture.paintable().is_some());
    let fitted = |viewer: &crate::ui::viewer::ViewerPane| {
        let imp = viewer.imp();
        let (view_w, view_h) = (
            imp.scrolled_window.width() as f64,
            imp.scrolled_window.height() as f64,
        );
        let expected = (view_w / 320.0).min(view_h / 180.0) * 320.0;
        (imp.picture.width() as f64 - expected).abs() <= 2.0
    };
    pump_until(|| fitted(&viewer));
    // Presentation hides both side panels, so the view grows without the
    // viewer being remapped.
    let before = viewer.imp().scrolled_window.width();
    window.enter_presentation_mode();
    pump_until(|| viewer.imp().scrolled_window.width() > before);
    // The side panels animate away; the picture must end up filling the view.
    pump_until(|| fitted(&viewer));
    window.close();
}

/// D35: Crop Copy, WebP export and Duplicate put the original and the copy
/// in Edits › Cropped / Exported / Duplicated, once each; the switch stops
/// it; originals keep their bytes.
#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn edits_collect_originals_and_copies_in_buckets() {
    let root = isolated_fixture("edit-buckets");
    std::fs::create_dir_all(&root).unwrap();
    let (a, b) = (root.join("a.png"), root.join("b.png"));
    for (path, colour) in [(&a, [200, 40, 40]), (&b, [40, 40, 200])] {
        image::RgbImage::from_pixel(160, 90, image::Rgb(colour))
            .save(path)
            .unwrap();
    }
    let originals = [std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()];
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let mut settings = AppSettings::load();
    let mut export = settings.webp_export.clone();
    export.dir = Some(root.join("crops"));
    settings.set_webp_export(export);
    settings.libraries.clear();
    settings
        .add_library("Edits", root.clone(), FolderMode::TopLevel)
        .unwrap();
    settings.set_active_library_last_folder(Some(root.clone()));
    assert!(settings.edit_buckets, "on by default");
    let app = libadwaita::Application::builder()
        .application_id("io.github.hebbihebb.Sharpr.GtkTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let window = SharprWindow::new(&app);
    window.set_default_size(1024, 600);
    window.present();
    pump_until(|| window.app_state().borrow().library.image_count() == 2 && window.is_mapped());
    let index = window.app_state().borrow().library_index.clone().unwrap();
    let members = |name: &str| -> Option<Vec<PathBuf>> {
        let bucket = index
            .list_collections()
            .unwrap()
            .into_iter()
            .find(|bucket| bucket.name == name)?;
        let mut paths = crate::buckets::view(&index, bucket.id, Some(&root), &[])
            .unwrap()
            .paths;
        paths.sort();
        Some(paths)
    };
    let in_folder = |folder: &str| -> Vec<PathBuf> {
        let mut paths: Vec<_> = std::fs::read_dir(root.join(folder))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| !path.file_name().unwrap().to_string_lossy().starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        paths.sort();
        paths
    };

    // Crop Copy: the original and its crop, saved in another folder.
    window.present_copy_dialog(crate::ui::filmstrip::CopyKind::Crop, vec![a.clone()]);
    pump_until(|| window.crop_session().is_some());
    // A real crop: a whole-image crop is byte-identical to the export below,
    // and buckets show identical copies together.
    let session = window.crop_session().unwrap();
    session.set_rect(crate::export::CropRect {
        x: 16,
        y: 9,
        width: 128,
        height: 72,
    });
    session.save();
    pump_until(|| members("Cropped").is_some_and(|m| m.len() == 2));
    let crop = in_folder("crops");
    assert_eq!(
        members("Cropped").unwrap(),
        vec![a.clone(), crop[0].clone()]
    );

    // A batch export: every original and its WebP.
    window.start_webp_export(
        vec![a.clone(), b.clone()],
        crate::export::batch::WebpOptions {
            destination: root.join("export"),
            lossless: true,
            quality: 90,
            fit: None,
        },
    );
    pump_until(|| members("Exported").is_some_and(|m| m.len() == 4));
    let mut expected = vec![a.clone(), b.clone()];
    expected.extend(in_folder("export"));
    expected.sort();
    assert_eq!(members("Exported").unwrap(), expected);

    // Duplicate, beside the original.
    let strip = window.imp().filmstrip.borrow().clone().unwrap();
    strip.emit_duplicate_requested(b.clone());
    pump_until(|| members("Duplicated").is_some_and(|m| m.len() == 2));

    let buckets = index.list_collections().unwrap();
    let edits = buckets
        .iter()
        .find(|bucket| bucket.name == crate::buckets::EDITS_BUCKET)
        .unwrap();
    for name in ["Cropped", "Exported", "Duplicated"] {
        let bucket = buckets.iter().find(|bucket| bucket.name == name).unwrap();
        assert_eq!(bucket.parent_id, Some(edits.id), "{name} sits under Edits");
    }
    assert_eq!(
        buckets
            .iter()
            .filter(|bucket| bucket.name == crate::buckets::EDITS_BUCKET)
            .count(),
        1
    );

    // Turned off, a further duplicate joins nothing.
    window
        .app_state()
        .borrow_mut()
        .settings
        .set_edit_buckets(false);
    let before = members("Duplicated").unwrap();
    strip.emit_duplicate_requested(a.clone());
    let copies = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .count();
    pump_until(|| {
        std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .count()
            >= copies
    });
    settle(500);
    assert_eq!(members("Duplicated").unwrap(), before);
    assert_eq!(std::fs::read(&a).unwrap(), originals[0]);
    assert_eq!(std::fs::read(&b).unwrap(), originals[1]);
    window.close();
}

#[test]
#[ignore = "requires isolated GSettings/data directories and a graphical session"]
fn internal_rename_keeps_buckets_when_the_old_filename_is_reused() {
    let root = isolated_fixture("audit-rename");
    std::fs::create_dir_all(&root).unwrap();
    let original = root.join("a.png");
    let renamed = root.join("b.png");
    image::RgbImage::from_pixel(160, 90, image::Rgb([200, 40, 40]))
        .save(&original)
        .unwrap();
    let bytes = std::fs::read(&original).unwrap();
    gtk4::init().unwrap();
    libadwaita::init().unwrap();
    crate::register_resources();
    let mut settings = AppSettings::load();
    settings.libraries.clear();
    settings
        .add_library("Audit rename", root.clone(), FolderMode::DrillDown)
        .unwrap();
    settings.set_active_library_last_folder(Some(root.clone()));
    let app = libadwaita::Application::builder()
        .application_id("io.github.hebbihebb.Sharpr.GtkTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let window = SharprWindow::new(&app);
    window.present();
    pump_until(|| window.is_mapped() && window.app_state().borrow().library.image_count() == 1);
    let state = window.app_state();
    let index = state.borrow().library_index.clone().unwrap();
    let tags = state.borrow().tags.clone().unwrap();
    let library = state.borrow().settings.active_library_id.clone().unwrap();
    let bucket = index
        .create_collection(&library, None, "Keep", &[], None, None)
        .unwrap();
    crate::buckets::add_to_bucket(&index, bucket.id, std::slice::from_ref(&original)).unwrap();
    tags.add_tag(&original, "legacy");
    window
        .imp()
        .filmstrip
        .borrow()
        .as_ref()
        .unwrap()
        .emit_rename_requested(original.clone());
    let dialog = window
        .visible_dialog()
        .unwrap()
        .downcast::<libadwaita::AlertDialog>()
        .unwrap();
    let entries = find_widgets::<gtk4::Entry>(dialog.upcast_ref());
    assert_eq!(entries.len(), 1);
    entries[0].set_text("b.png");
    find_widgets::<gtk4::Button>(dialog.upcast_ref())
        .into_iter()
        .find(|button| button.label().as_deref() == Some("Rename"))
        .unwrap()
        .emit_clicked();
    pump_until(|| renamed.exists());
    image::RgbImage::from_pixel(160, 90, image::Rgb([40, 40, 200]))
        .save(&original)
        .unwrap();
    crate::fingerprint::run_pass(&index, &root, &[], &[], &|| false, &|_| {}).unwrap();
    crate::buckets::relink(&index).unwrap();
    assert_eq!(
        crate::buckets::view(&index, bucket.id, Some(&root), &[])
            .unwrap()
            .paths,
        vec![renamed.clone()]
    );
    assert_eq!(std::fs::read(&renamed).unwrap(), bytes);
    assert_eq!(tags.tags_for_path(&renamed), vec!["legacy"]);
    window.close();
}
