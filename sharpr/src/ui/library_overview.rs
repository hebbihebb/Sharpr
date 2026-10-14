//! The Library Overview: how big the library is and what its indexes hold
//! (`ROADMAP.md` Phase 5). Statistics are gathered on a worker when the
//! window opens; the visual-search figures update every second while it is
//! open.

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::library_overview::LibraryStats;
use crate::ui::window::SharprWindow;
use crate::visual_search::index::Status;

/// `12345` → `12,345`.
pub fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

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

/// `754 s` → `about 13 minutes`.
pub fn duration(d: std::time::Duration) -> String {
    let minutes = d.as_secs().div_ceil(60);
    if minutes < 2 {
        "about a minute".into()
    } else if minutes < 90 {
        format!("about {minutes} minutes")
    } else {
        format!("about {:.1} hours", minutes as f64 / 60.0)
    }
}

fn images(n: usize) -> String {
    format!("{} {}", count(n), if n == 1 { "image" } else { "images" })
}

fn row(title: &str) -> libadwaita::ActionRow {
    let row = libadwaita::ActionRow::new();
    row.set_title(title);
    row.set_subtitle("…");
    row.set_subtitle_selectable(true);
    row.add_css_class("property");
    row
}

pub fn present(window: &SharprWindow) {
    let dialog = libadwaita::PreferencesDialog::new();
    dialog.set_title("Library Overview");
    dialog.set_search_enabled(false);
    let page = libadwaita::PreferencesPage::new();
    dialog.add(&page);

    let (root, disabled, library_id) = {
        let state = window.app_state();
        let st = state.borrow();
        (
            st.settings.active_library().map(|l| l.root.clone()),
            st.disabled_folders.clone(),
            st.settings.active_library_id.clone(),
        )
    };

    let library = libadwaita::PreferencesGroup::new();
    library.set_title("Library");
    if let Some(root) = &root {
        library.set_description(Some(&root.display().to_string()));
    }
    let size_row = row("Images");
    let formats_row = row("Formats");
    let below_row = row("Below 4K");
    let disabled_row = row("Disabled Folders");
    for r in [&size_row, &formats_row, &below_row, &disabled_row] {
        library.add(r);
    }
    page.add(&library);

    let identity = libadwaita::PreferencesGroup::new();
    identity.set_title("File Identity");
    identity.set_description(Some(
        "Fingerprints let buckets follow images that are moved or renamed outside Sharpr.",
    ));
    let fingerprint_row = row("Fingerprinted");
    let copies_row = row("Exact Copies");
    identity.add(&fingerprint_row);
    identity.add(&copies_row);
    page.add(&identity);

    let visual = libadwaita::PreferencesGroup::new();
    visual.set_title("Visual Search");
    let coverage_row = row("Indexed");
    let activity_row = row("Indexing");
    let model_row = row("Model");
    model_row.set_subtitle(&format!(
        "EmbeddingGemma 2 · {}",
        crate::visual_search::files::model_dir().display()
    ));
    let open_button = gtk4::Button::with_label("Open Folder");
    open_button.set_valign(gtk4::Align::Center);
    open_button.connect_clicked(|_| {
        let dir = crate::visual_search::files::model_dir();
        // An empty folder is harmless and gives the user a place to put files.
        let _ = std::fs::create_dir_all(&dir);
        let uri = gio::File::for_path(&dir).uri();
        let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
    });
    model_row.add_suffix(&open_button);
    let file_row = row("Index File");
    let rebuild_button = gtk4::Button::with_label("Rebuild…");
    rebuild_button.set_valign(gtk4::Align::Center);
    rebuild_button.add_css_class("destructive-action");
    {
        let window = window.downgrade();
        let dialog = dialog.downgrade();
        rebuild_button.connect_clicked(move |_| {
            if let (Some(window), Some(dialog)) = (window.upgrade(), dialog.upgrade()) {
                confirm_rebuild(&window, &dialog);
            }
        });
    }
    file_row.add_suffix(&rebuild_button);
    for r in [&coverage_row, &activity_row, &model_row, &file_row] {
        visual.add(r);
    }
    page.add(&visual);

    let buckets = libadwaita::PreferencesGroup::new();
    buckets.set_title("Buckets");
    let buckets_row = row("Buckets");
    let missing_row = row("Can't Be Found");
    buckets.add(&buckets_row);
    buckets.add(&missing_row);
    page.add(&buckets);

    let fill_stats = move |stats: &LibraryStats| {
        size_row.set_subtitle(&format!(
            "{} · {}",
            images(stats.images),
            bytes(stats.bytes)
        ));
        let formats: Vec<String> = stats
            .formats
            .iter()
            .map(|(format, n)| format!("{format} {}", count(*n)))
            .collect();
        formats_row.set_subtitle(&if formats.is_empty() {
            "None".into()
        } else {
            formats.join(" · ")
        });
        let mut below = images(stats.below_4k);
        if stats.size_unknown > 0 {
            below.push_str(&format!(
                " · sizes of {} not read yet",
                count(stats.size_unknown)
            ));
        }
        below_row.set_subtitle(&below);
        disabled_row.set_subtitle(&if stats.disabled.is_empty() {
            "None".into()
        } else {
            stats
                .disabled
                .iter()
                .map(|(folder, n)| {
                    let name = folder
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| folder.display().to_string());
                    format!("{name} ({})", images(*n))
                })
                .collect::<Vec<_>>()
                .join(" · ")
        });
        fingerprint_row.set_subtitle(&format!(
            "{} of {}",
            count(stats.fingerprinted),
            images(stats.images)
        ));
        copies_row.set_subtitle(&if stats.copy_groups == 0 {
            "None".into()
        } else {
            format!(
                "{} {} of identical files ({} extra)",
                count(stats.copy_groups),
                if stats.copy_groups == 1 {
                    "group"
                } else {
                    "groups"
                },
                count(stats.extra_copies)
            )
        });
        file_row.set_subtitle(&match stats.vector_file_bytes {
            Some(n) => format!("{} · safe to delete; it rebuilds", bytes(n)),
            None => "Not created yet".into(),
        });
        buckets_row.set_subtitle(&format!(
            "{} {} · {} in them",
            count(stats.buckets),
            if stats.buckets == 1 {
                "bucket"
            } else {
                "buckets"
            },
            images(stats.bucket_images)
        ));
        missing_row.set_subtitle(&if stats.bucket_missing == 0 {
            "None".into()
        } else {
            format!(
                "{} · kept as members until they turn up",
                images(stats.bucket_missing)
            )
        });
    };

    if let (Some(root), Some(index)) = (root, window.app_state().borrow().library_index.clone()) {
        let (tx, rx) = async_channel::bounded(1);
        let vector_file = crate::visual_search::store::default_path();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(crate::library_overview::gather(
                &index,
                &root,
                &disabled,
                library_id.as_deref(),
                &vector_file,
            ));
        });
        let dialog_weak = dialog.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            if dialog_weak.upgrade().is_none() {
                return;
            }
            match result {
                Ok(stats) => fill_stats(&stats),
                Err(error) => {
                    crate::bench_warn!("library_overview.fail", serde_json::json!({"error": error}))
                }
            }
        });
    }

    let refresh_visual = {
        let window = window.downgrade();
        move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            let failed = |n: usize| {
                if n == 0 {
                    String::new()
                } else {
                    format!(" · {} couldn't be read", count(n))
                }
            };
            let (coverage, activity) = match window.visual_search_status() {
                None | Some(Status::Waiting) => (
                    "Waiting for fingerprints".to_string(),
                    "Not started".to_string(),
                ),
                Some(Status::Unavailable(message)) => ("Not available".to_string(), message),
                Some(Status::Failed(error)) => (
                    "Stopped".to_string(),
                    format!("{error}. It tries again after the next fingerprint pass"),
                ),
                Some(Status::Indexing(c)) => {
                    let pace = window.visual_search_pace();
                    let mut activity = format!(
                        "Running on {}",
                        window
                            .visual_search_device()
                            .unwrap_or_else(|| "the processor".into())
                    );
                    if let (Some(rate), Some(left)) = (pace.per_second, pace.remaining) {
                        activity
                            .push_str(&format!(" · {rate:.1} images/s · {} left", duration(left)));
                    }
                    (
                        format!(
                            "{} of {}{}",
                            count(c.indexed),
                            images(c.images),
                            failed(c.failed)
                        ),
                        activity,
                    )
                }
                Some(Status::Done(c)) => {
                    let pace = window.visual_search_pace();
                    let finished = pace
                        .last_finished
                        .map(|t| {
                            let ago = t.elapsed().unwrap_or_default();
                            if ago.as_secs() < 60 {
                                " · last pass finished just now".to_string()
                            } else {
                                format!(
                                    " · last pass finished {} ago",
                                    duration(ago).trim_start_matches("about ")
                                )
                            }
                        })
                        .unwrap_or_default();
                    (
                        format!(
                            "{} of {}{}",
                            count(c.indexed),
                            images(c.images),
                            failed(c.failed)
                        ),
                        format!("Idle{finished}"),
                    )
                }
            };
            coverage_row.set_subtitle(&coverage);
            activity_row.set_subtitle(&activity);
        }
    };
    refresh_visual();
    {
        let dialog = dialog.downgrade();
        glib::timeout_add_seconds_local(1, move || {
            if dialog.upgrade().is_none() {
                return glib::ControlFlow::Break;
            }
            refresh_visual();
            glib::ControlFlow::Continue
        });
    }
    dialog.present(Some(window));
}

fn confirm_rebuild(window: &SharprWindow, parent: &libadwaita::PreferencesDialog) {
    let alert = libadwaita::AlertDialog::new(
        Some("Rebuild the Visual Search Index?"),
        Some(
            "Every image's search data is deleted and indexed again: about 20 minutes on a \
             graphics card, or several hours on the processor. Searches match file names until \
             indexing catches up. Buckets, fingerprints, and images are not touched.",
        ),
    );
    alert.add_response("cancel", "Cancel");
    alert.add_response("rebuild", "Rebuild");
    alert.set_response_appearance("rebuild", libadwaita::ResponseAppearance::Destructive);
    alert.set_default_response(Some("cancel"));
    alert.set_close_response("cancel");
    let window = window.downgrade();
    alert.connect_response(None, move |_, response| {
        if response == "rebuild" {
            if let Some(window) = window.upgrade() {
                window.rebuild_visual_index();
            }
        }
    });
    alert.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_durations_read_naturally() {
        assert_eq!(count(5), "5");
        assert_eq!(count(5854), "5,854");
        assert_eq!(count(1234567), "1,234,567");
        assert_eq!(bytes(7_400_000), "7.4 MB");
        assert_eq!(bytes(18_300_000_000), "18.3 GB");
        assert_eq!(
            duration(std::time::Duration::from_secs(30)),
            "about a minute"
        );
        assert_eq!(
            duration(std::time::Duration::from_secs(754)),
            "about 13 minutes"
        );
        assert_eq!(
            duration(std::time::Duration::from_secs(5 * 3600)),
            "about 5.0 hours"
        );
        assert_eq!(images(1), "1 image");
    }
}
