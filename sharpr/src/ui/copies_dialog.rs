//! "Find Copies…" (`ROADMAP.md` Phase 7, decision D14): groups of exact,
//! resized, and converted copies, each with the highest resolution first
//! and the others ticked for the trash. Nothing moves until the user
//! confirms, and failures are reported.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::copies::{CopyGroup, CopyReport};
use crate::ui::library_overview::count;
use crate::ui::window::SharprWindow;

/// Each file's path, size in bytes, group, and "move to the trash" check.
type Checks = Rc<RefCell<Vec<(PathBuf, u64, usize, gtk4::CheckButton)>>>;

fn bytes(n: u64) -> String {
    let n = n as f64;
    if n >= 1e9 {
        format!("{:.1} GB", n / 1e9)
    } else if n >= 1e6 {
        format!("{:.1} MB", n / 1e6)
    } else {
        format!("{:.0} KB", n / 1e3)
    }
}

fn format_of(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "JPEG".into(),
        Some("jxl") => "JPEG XL".into(),
        Some(other) => other.to_ascii_uppercase(),
        None => "Image".into(),
    }
}

fn folder_of(path: &std::path::Path) -> String {
    let parent = path.parent().unwrap_or(path);
    match dirs::home_dir()
        .as_deref()
        .and_then(|home| parent.strip_prefix(home).ok())
    {
        Some(rest) => format!("~/{}", rest.display()),
        None => parent.display().to_string(),
    }
}

/// Opens the window and looks for copies on a worker.
pub fn present(window: &SharprWindow) {
    let dialog = libadwaita::Dialog::new();
    dialog.set_title("Copies");
    dialog.set_content_width(760);
    dialog.set_content_height(680);
    let toolbar = libadwaita::ToolbarView::new();
    toolbar.add_top_bar(&libadwaita::HeaderBar::new());
    let stack = gtk4::Stack::new();
    let searching = libadwaita::StatusPage::new();
    searching.set_title("Looking for Copies…");
    searching.set_description(Some(
        "Exact copies, and resized or converted copies of the same picture",
    ));
    let spinner = gtk4::Spinner::new();
    spinner.set_size_request(32, 32);
    spinner.start();
    searching.set_child(Some(&spinner));
    stack.add_named(&searching, Some("searching"));
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(window));

    let (root, disabled, index) = {
        let state = window.app_state();
        let st = state.borrow();
        (
            st.settings.active_library().map(|l| l.root.clone()),
            st.disabled_folders.clone(),
            st.library_index.clone(),
        )
    };
    let (Some(root), Some(index)) = (root, index) else {
        searching.set_title("No Library");
        spinner.stop();
        return;
    };
    // Closing the window stops the search; the count shows it's moving.
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    let canceled = Arc::new(AtomicBool::new(false));
    {
        let canceled = canceled.clone();
        dialog.connect_closed(move |_| canceled.store(true, Ordering::Relaxed));
    }
    let progress = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
    {
        let progress = progress.clone();
        let searching = searching.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            let Some(searching) = searching.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let (done, total) = (
                progress.0.load(Ordering::Relaxed),
                progress.1.load(Ordering::Relaxed),
            );
            if total > 0 {
                searching.set_description(Some(&format!(
                    "Comparing similar pictures: {done} of {total}"
                )));
            }
            glib::ControlFlow::Continue
        });
    }
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        // Visual-search vectors find resized and converted copies; without
        // them, only exact copies are found.
        let vectors = crate::visual_search::files::locate()
            .ok()
            .and_then(|files| {
                let store = crate::visual_search::store::Store::open(
                    &crate::visual_search::store::default_path(),
                )
                .ok()?;
                let key = store.model_key(&files).ok()?;
                store.vectors(&key).ok()
            })
            .unwrap_or_default();
        let result = crate::copies::find(
            &index,
            &root,
            &disabled,
            &vectors,
            &|| canceled.load(Ordering::Relaxed),
            &|done, total| {
                progress.0.store(done, Ordering::Relaxed);
                progress.1.store(total, Ordering::Relaxed);
            },
        );
        let _ = tx.send_blocking(result);
    });
    let window = window.downgrade();
    let dialog_weak = dialog.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let Ok(result) = rx.recv().await else { return };
        let (Some(window), Some(dialog)) = (window.upgrade(), dialog_weak.upgrade()) else {
            return;
        };
        spinner.stop();
        match result {
            Ok(report) if report.groups.is_empty() => {
                let empty = libadwaita::StatusPage::new();
                empty.set_icon_name(Some("emblem-ok-symbolic"));
                empty.set_title("No Copies Found");
                empty.set_description(Some(&checked_note(&report)));
                stack.add_named(&empty, Some("empty"));
                stack.set_visible_child_name("empty");
            }
            Ok(report) => {
                let results = results_page(&window, &dialog, &report);
                stack.add_named(&results, Some("results"));
                stack.set_visible_child_name("results");
            }
            Err(error) => {
                searching.set_title("Couldn't Look for Copies");
                searching.set_description(Some(&error));
            }
        }
    });
}

fn checked_note(report: &CopyReport) -> String {
    let mut note = format!("{} checked.", images_text(report.images));
    if report.not_indexed > 0 {
        note.push_str(&format!(
            " {} aren't indexed for visual search yet, so only their exact copies could be found.",
            count(report.not_indexed)
        ));
    }
    note
}

fn images_text(n: usize) -> String {
    format!("{} {}", count(n), if n == 1 { "image" } else { "images" })
}

/// The groups, each file with a "move to the trash" check; the extras are
/// ticked. The bottom button asks before anything moves.
fn results_page(
    window: &SharprWindow,
    dialog: &libadwaita::Dialog,
    report: &CopyReport,
) -> gtk4::Widget {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let scroller = gtk4::ScrolledWindow::new();
    scroller.set_vexpand(true);
    scroller.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    let list = libadwaita::PreferencesPage::new();
    let extras: usize = report.groups.iter().map(|g| g.files.len() - 1).sum();
    let summary = libadwaita::PreferencesGroup::new();
    summary.set_title(&format!(
        "{} {} of copies",
        count(report.groups.len()),
        if report.groups.len() == 1 {
            "group"
        } else {
            "groups"
        }
    ));
    summary.set_description(Some(&format!(
        "{} beyond the first of each group; the highest resolution comes first and is kept. {}",
        images_text(extras),
        checked_note(report)
    )));
    list.add(&summary);

    let checks: Checks = Rc::default();
    let thumbnails: Rc<RefCell<HashMap<PathBuf, gtk4::Picture>>> = Rc::default();
    for (group_index, group) in report.groups.iter().enumerate() {
        list.add(&group_rows(group, group_index, &checks, &thumbnails));
    }
    scroller.set_child(Some(&list));
    page.append(&scroller);

    let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    bar.set_margin_top(8);
    bar.set_margin_bottom(12);
    bar.set_margin_start(12);
    bar.set_margin_end(12);
    let selected = gtk4::Label::new(None);
    selected.add_css_class("dim-label");
    selected.add_css_class("numeric");
    selected.set_hexpand(true);
    selected.set_xalign(0.0);
    let trash = gtk4::Button::with_label("Move to Trash…");
    trash.add_css_class("destructive-action");
    bar.append(&selected);
    bar.append(&trash);
    page.append(&bar);

    let refresh = {
        let checks = checks.clone();
        let selected = selected.clone();
        let trash = trash.clone();
        move || {
            let checks = checks.borrow();
            let chosen: Vec<_> = checks.iter().filter(|(.., c)| c.is_active()).collect();
            let size: u64 = chosen.iter().map(|(_, b, ..)| *b).sum();
            selected.set_text(&format!(
                "{} ticked · {}",
                images_text(chosen.len()),
                bytes(size)
            ));
            trash.set_sensitive(!chosen.is_empty());
        }
    };
    refresh();
    for (.., check) in checks.borrow().iter() {
        let refresh = refresh.clone();
        check.connect_toggled(move |_| refresh());
    }

    {
        let window = window.downgrade();
        let dialog = dialog.downgrade();
        let checks = checks.clone();
        let groups = report.groups.len();
        trash.connect_clicked(move |_| {
            let (Some(window), Some(dialog)) = (window.upgrade(), dialog.upgrade()) else {
                return;
            };
            let checks = checks.borrow();
            let chosen: Vec<(PathBuf, u64, usize)> = checks
                .iter()
                .filter(|(.., c)| c.is_active())
                .map(|(p, b, g, _)| (p.clone(), *b, *g))
                .collect();
            let emptied = (0..groups)
                .filter(|g| {
                    checks
                        .iter()
                        .filter(|(_, _, cg, _)| cg == g)
                        .all(|(.., c)| c.is_active())
                })
                .count();
            confirm_trash(&window, &dialog, chosen, emptied);
        });
    }
    page.upcast()
}

fn group_rows(
    group: &CopyGroup,
    group_index: usize,
    checks: &Checks,
    thumbnails: &Rc<RefCell<HashMap<PathBuf, gtk4::Picture>>>,
) -> libadwaita::PreferencesGroup {
    let rows = libadwaita::PreferencesGroup::new();
    rows.set_title(if group.identical {
        "Identical Files"
    } else {
        "Same Picture"
    });
    rows.set_description(Some(&format!(
        "{} · the extras take {}",
        images_text(group.files.len()),
        bytes(group.extra_bytes())
    )));
    for (i, file) in group.files.iter().enumerate() {
        let row = libadwaita::ActionRow::new();
        row.set_title(
            &file
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        row.set_title_lines(1);
        let size = file
            .size
            .map(|(w, h)| format!("{w} × {h}"))
            .unwrap_or_else(|| "size unknown".into());
        row.set_subtitle(&format!(
            "{size} · {} · {} · {}",
            format_of(&file.path),
            bytes(file.bytes),
            folder_of(&file.path)
        ));
        row.set_subtitle_lines(2);
        let picture = gtk4::Picture::new();
        picture.set_size_request(64, 48);
        picture.set_content_fit(gtk4::ContentFit::Cover);
        picture.set_can_shrink(true);
        picture.add_css_class("card");
        row.add_prefix(&picture);
        thumbnails
            .borrow_mut()
            .insert(file.path.clone(), picture.clone());
        load_thumbnail(file.path.clone(), picture);
        if i == 0 {
            let keep = gtk4::Label::new(Some("Keep"));
            keep.add_css_class("success");
            keep.add_css_class("caption-heading");
            row.add_suffix(&keep);
        }
        let check = gtk4::CheckButton::new();
        check.set_valign(gtk4::Align::Center);
        check.set_tooltip_text(Some("Move to the trash"));
        check.set_active(i > 0);
        row.add_suffix(&check);
        row.set_activatable_widget(Some(&check));
        rows.add(&row);
        checks
            .borrow_mut()
            .push((file.path.clone(), file.bytes, group_index, check));
    }
    rows
}

/// A thumbnail from Sharpr's or the system's cache, or else a small preview
/// decoded here; all on a worker.
fn load_thumbnail(path: PathBuf, picture: gtk4::Picture) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let thumbnail = crate::thumbnails::worker::load_cached_rgba(&path).or_else(|| {
            let png = crate::visual_search::prepare::prepare_png(&path).ok()?;
            let image = image::load_from_memory(&png)
                .ok()?
                .thumbnail(160, 160)
                .to_rgba8();
            Some((image.width(), image.height(), image.into_raw())).map(|(w, h, rgba)| (rgba, w, h))
        });
        let _ = tx.send_blocking(thumbnail);
    });
    glib::MainContext::default().spawn_local(async move {
        let Ok(Some((rgba, width, height))) = rx.recv().await else {
            return;
        };
        let texture = gdk4::MemoryTexture::new(
            width as i32,
            height as i32,
            gdk4::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(rgba),
            width as usize * 4,
        );
        picture.set_paintable(Some(&texture));
    });
}

fn confirm_trash(
    window: &SharprWindow,
    dialog: &libadwaita::Dialog,
    chosen: Vec<(PathBuf, u64, usize)>,
    emptied_groups: usize,
) {
    let size: u64 = chosen.iter().map(|(_, b, _)| *b).sum();
    let mut body = format!(
        "{} ({}) will move to the trash, where you can restore them.",
        images_text(chosen.len()),
        bytes(size)
    );
    if emptied_groups > 0 {
        body.push_str(&format!(
            " Every file of {} {} is ticked: no copy of {} will be left.",
            count(emptied_groups),
            if emptied_groups == 1 {
                "group"
            } else {
                "groups"
            },
            if emptied_groups == 1 {
                "that picture"
            } else {
                "those pictures"
            }
        ));
    }
    let alert = libadwaita::AlertDialog::new(Some("Move Copies to the Trash?"), Some(&body));
    alert.add_response("cancel", "Cancel");
    alert.add_response("trash", "Move to Trash");
    alert.set_response_appearance("trash", libadwaita::ResponseAppearance::Destructive);
    alert.set_default_response(Some("cancel"));
    alert.set_close_response("cancel");
    let window = window.downgrade();
    let dialog_weak = dialog.downgrade();
    alert.connect_response(None, move |_, response| {
        if response != "trash" {
            return;
        }
        let (Some(window), Some(dialog)) = (window.upgrade(), dialog_weak.upgrade()) else {
            return;
        };
        dialog.close();
        let paths: Vec<PathBuf> = chosen.iter().map(|(p, ..)| p.clone()).collect();
        window.trash_copies(paths);
    });
    alert.present(Some(dialog));
}
