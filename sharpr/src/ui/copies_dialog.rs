//! "Find Copies…" (`ROADMAP.md` Phase 7, decision D14): groups of exact,
//! resized, and converted copies, each with the highest resolution first
//! and the others ticked for the trash. Nothing moves until the user
//! confirms, and failures are reported.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::copies::{CopyGroup, CopyReport};
use crate::ui::library_overview::count;
use crate::ui::window::SharprWindow;

/// One reviewed file and its "move to the trash" check.
struct Choice {
    path: PathBuf,
    stamp: (u64, i64),
    group: usize,
    check: gtk4::CheckButton,
}

type Checks = Rc<RefCell<Vec<Choice>>>;

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

/// What is in view, for the window's wording: "in “Wallpapers”".
fn scope_label(state: &crate::ui::state::AppState) -> String {
    use crate::ui::state::ViewScope;
    match &state.scope {
        ViewScope::Folder(folder) => format!(
            "in \u{201c}{}\u{201d}",
            folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| folder.display().to_string())
        ),
        ViewScope::Collection(id) => state
            .collections
            .iter()
            .find(|c| c.id == *id)
            .map(|c| format!("in the bucket \u{201c}{}\u{201d}", c.name))
            .unwrap_or_else(|| "in this bucket".into()),
        ViewScope::Search => "among the search results".into(),
        _ => "among the images shown".into(),
    }
}

/// Opens the window and looks for copies among the images in view (the
/// folder, bucket, or search results shown) on a worker.
pub fn present(window: &SharprWindow) {
    let (paths, scope, index) = {
        let state = window.app_state();
        let st = state.borrow();
        let paths: Vec<PathBuf> = (0..st.library.image_count())
            .filter_map(|i| st.library.entry_at(i).map(|e| e.path()))
            .collect();
        (paths, scope_label(&st), st.library_index.clone())
    };
    let dialog = libadwaita::Dialog::new();
    dialog.set_title("Copies");
    dialog.set_content_width(760);
    dialog.set_content_height(680);
    let toolbar = libadwaita::ToolbarView::new();
    toolbar.add_top_bar(&libadwaita::HeaderBar::new());
    let stack = gtk4::Stack::new();
    let searching = libadwaita::StatusPage::new();
    searching.set_title(&format!("Looking for Copies {scope}…"));
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

    let Some(index) = index else {
        searching.set_title("No Library");
        spinner.stop();
        return;
    };
    if paths.len() < 2 {
        searching.set_title("Nothing to Compare");
        searching.set_description(Some("Open a folder with at least two images first."));
        spinner.stop();
        return;
    }
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
            &paths,
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
        let result = rx
            .recv()
            .await
            .unwrap_or_else(|_| Err("The search stopped unexpectedly.".into()));
        let (Some(window), Some(dialog)) = (window.upgrade(), dialog_weak.upgrade()) else {
            return;
        };
        spinner.stop();
        match result {
            Ok(report) if report.groups.is_empty() => {
                let empty = libadwaita::StatusPage::new();
                empty.set_icon_name(Some("emblem-ok-symbolic"));
                empty.set_title(&format!("No Copies Found {scope}"));
                empty.set_description(Some(&checked_note(&report)));
                stack.add_named(&empty, Some("empty"));
                stack.set_visible_child_name("empty");
            }
            Ok(report) => {
                let results = results_page(&window, &dialog, &report, &scope);
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
    scope: &str,
) -> gtk4::Widget {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let scroller = gtk4::ScrolledWindow::new();
    scroller.set_vexpand(true);
    scroller.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    let list = libadwaita::PreferencesPage::new();
    let extras: usize = report.groups.iter().map(|g| g.files.len() - 1).sum();
    let summary = libadwaita::PreferencesGroup::new();
    summary.set_title(&format!(
        "{} {} of copies {scope}",
        count(report.groups.len()),
        if report.groups.len() == 1 {
            "group"
        } else {
            "groups"
        }
    ));
    summary.set_description(Some(&format!(
        "{} beyond the first of each group. The highest resolution comes first, marked Keep. \
         Nothing is ticked: tick what should go, or use Select All Extras. {}",
        images_text(extras),
        checked_note(report)
    )));
    list.add(&summary);

    let checks: Checks = Rc::default();
    let mut thumbnails = Vec::new();
    for (group_index, group) in report.groups.iter().enumerate() {
        let rows = group_rows(group, group_index, &checks, &mut thumbnails);
        rows.set_header_suffix(Some(&compare_button(
            window,
            dialog,
            group,
            group_index,
            &checks,
        )));
        list.add(&rows);
    }
    load_thumbnails(thumbnails);
    scroller.set_child(Some(&list));
    page.append(&scroller);

    let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    bar.set_margin_top(8);
    bar.set_margin_bottom(12);
    bar.set_margin_start(12);
    bar.set_margin_end(12);
    let select_extras = gtk4::Button::with_label("Select All Extras");
    select_extras.set_tooltip_text(Some("Tick every file except the one marked Keep"));
    let select_none = gtk4::Button::with_label("Select None");
    {
        let all = checks.clone();
        select_extras.connect_clicked(move |_| {
            let checks = all.borrow();
            for (i, choice) in checks.iter().enumerate() {
                // The first of each group is the one to keep.
                let first = i == 0 || checks[i - 1].group != choice.group;
                choice.check.set_active(!first);
            }
        });
        let checks = checks.clone();
        select_none.connect_clicked(move |_| {
            for choice in checks.borrow().iter() {
                choice.check.set_active(false);
            }
        });
    }
    bar.append(&select_extras);
    bar.append(&select_none);
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
            let chosen: Vec<_> = checks.iter().filter(|c| c.check.is_active()).collect();
            let size: u64 = chosen.iter().map(|c| c.stamp.0).sum();
            selected.set_text(&format!(
                "{} ticked · {}",
                images_text(chosen.len()),
                bytes(size)
            ));
            trash.set_sensitive(!chosen.is_empty());
        }
    };
    refresh();
    for choice in checks.borrow().iter() {
        let refresh = refresh.clone();
        choice.check.connect_toggled(move |_| refresh());
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
            let mut plan = vec![crate::copies::TrashGroup::default(); groups];
            for choice in checks.iter() {
                let entry = (choice.path.clone(), choice.stamp);
                if choice.check.is_active() {
                    plan[choice.group].extras.push(entry);
                } else {
                    plan[choice.group].kept.push(entry);
                }
            }
            plan.retain(|group| !group.extras.is_empty());
            confirm_trash(&window, &dialog, plan);
        });
    }
    page.upcast()
}

/// "Compare": opens the group in Compare, the copy to keep on the left;
/// back here, the images dismissed there are ticked and the rest are not.
/// Compare itself never changes files.
fn compare_button(
    window: &SharprWindow,
    dialog: &libadwaita::Dialog,
    group: &CopyGroup,
    group_index: usize,
    checks: &Checks,
) -> gtk4::Button {
    let button = gtk4::Button::with_label("Compare");
    button.add_css_class("flat");
    button.set_valign(gtk4::Align::Center);
    button.set_tooltip_text(Some(
        "Look at these side by side; the ones you dismiss come back ticked",
    ));
    let paths: Vec<PathBuf> = group.files.iter().map(|f| f.path.clone()).collect();
    let window = window.downgrade();
    let dialog = dialog.clone();
    let checks = checks.clone();
    button.connect_clicked(move |_| {
        let Some(window) = window.upgrade() else {
            return;
        };
        // The window waits, closed but kept, for Compare to finish.
        dialog.close();
        let dialog = dialog.clone();
        let checks = checks.clone();
        let parent = window.downgrade();
        window.compare_then(paths.clone(), move |survivors| {
            for choice in checks.borrow().iter() {
                if choice.group == group_index {
                    choice.check.set_active(!survivors.contains(&choice.path));
                }
            }
            if let Some(parent) = parent.upgrade() {
                dialog.present(Some(&parent));
            }
        });
    });
    button
}

fn group_rows(
    group: &CopyGroup,
    group_index: usize,
    checks: &Checks,
    thumbnails: &mut Vec<(PathBuf, gtk4::Picture)>,
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
        thumbnails.push((file.path.clone(), picture));
        if i == 0 {
            let keep = gtk4::Label::new(Some("Keep"));
            keep.add_css_class("success");
            keep.add_css_class("caption-heading");
            row.add_suffix(&keep);
        }
        let check = gtk4::CheckButton::new();
        check.set_valign(gtk4::Align::Center);
        check.set_tooltip_text(Some("Move to the trash"));
        // Nothing is ticked until the user chooses.
        check.set_active(false);
        row.add_suffix(&check);
        row.set_activatable_widget(Some(&check));
        rows.add(&row);
        checks.borrow_mut().push(Choice {
            path: file.path.clone(),
            stamp: file.stamp,
            group: group_index,
            check,
        });
    }
    rows
}

/// Each row's thumbnail, from Sharpr's or the system's cache, or else a
/// small preview decoded here. One worker takes them in list order, so a
/// long list never starts thousands of decodes at once; it stops when the
/// window closes.
fn load_thumbnails(rows: Vec<(PathBuf, gtk4::Picture)>) {
    let paths: Vec<PathBuf> = rows.iter().map(|(path, _)| path.clone()).collect();
    let pictures: Vec<glib::WeakRef<gtk4::Picture>> = rows
        .iter()
        .map(|(_, picture)| picture.downgrade())
        .collect();
    let (tx, rx) = async_channel::bounded(4);
    std::thread::spawn(move || {
        for (i, path) in paths.into_iter().enumerate() {
            // Small, so every row's picture is the same modest size.
            let image = crate::thumbnails::worker::load_cached_rgba(&path)
                .and_then(|(rgba, w, h)| {
                    image::RgbaImage::from_raw(w, h, rgba).map(image::DynamicImage::ImageRgba8)
                })
                .or_else(|| {
                    let png = crate::visual_search::prepare::prepare_png(&path).ok()?;
                    image::load_from_memory(&png).ok()
                });
            let thumbnail = image.map(|image| {
                let small = image.thumbnail(96, 64).to_rgba8();
                let (w, h) = small.dimensions();
                (small.into_raw(), w, h)
            });
            if tx.send_blocking((i, thumbnail)).is_err() {
                return;
            }
        }
    });
    glib::MainContext::default().spawn_local(async move {
        while let Ok((i, thumbnail)) = rx.recv().await {
            // Closed: dropping the receiver stops the worker.
            let Some(picture) = pictures[i].upgrade() else {
                return;
            };
            let Some((rgba, width, height)) = thumbnail else {
                continue;
            };
            let texture = gdk4::MemoryTexture::new(
                width as i32,
                height as i32,
                gdk4::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_owned(rgba),
                width as usize * 4,
            );
            picture.set_paintable(Some(&texture));
        }
    });
}

fn confirm_trash(
    window: &SharprWindow,
    dialog: &libadwaita::Dialog,
    plan: Vec<crate::copies::TrashGroup>,
) {
    let files: Vec<_> = plan.iter().flat_map(|g| &g.extras).collect();
    let size: u64 = files.iter().map(|(_, stamp)| stamp.0).sum();
    let emptied_groups = plan.iter().filter(|g| g.kept.is_empty()).count();
    let mut body = format!(
        "{} ({}) will move to the trash, where you can restore them. Any file that changes before then stays.",
        images_text(files.len()),
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
        window.trash_copies(plan.clone());
    });
    alert.present(Some(dialog));
}
