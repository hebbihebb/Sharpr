//! Rename… as a Sharpr sheet: the name is a text field and the extension a
//! fixed chip beside it, so typing a new name cannot replace the extension.
//! Clicking the chip turns it back into text, as search chips do (D16).
use super::*;

/// "holiday.photo.png" → ("holiday.photo", Some(".png")). A leading dot
/// (".hidden") or a trailing one ("name.") is part of the name, not an
/// extension.
pub(crate) fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rfind('.') {
        Some(at) if at > 0 && at + 1 < name.len() => (&name[..at], Some(&name[at..])),
        _ => (name, None),
    }
}

/// Whether a proposed name can be used, and what the sheet says if not.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Ready,
    /// The current name; Rename stays off without a message.
    Unchanged,
    Empty,
    Invalid,
    /// Another file in the open folder has this name.
    Taken,
}

/// `typed` is the field's text; `extension` the chip's, while it shows.
pub(crate) fn verdict(
    typed: &str,
    extension: Option<&str>,
    current: &str,
    taken: impl Fn(&str) -> bool,
) -> Verdict {
    let name = format!("{typed}{}", extension.unwrap_or(""));
    let name = name.as_str();
    if name == current {
        Verdict::Unchanged
    } else if typed.trim().is_empty() {
        Verdict::Empty
    } else if crate::file_ops::rename_destination(Path::new("/file"), name).is_err() {
        Verdict::Invalid
    } else if taken(name) {
        Verdict::Taken
    } else {
        Verdict::Ready
    }
}

impl Verdict {
    fn message(&self, name: &str) -> Option<String> {
        match self {
            Self::Ready | Self::Unchanged => None,
            Self::Empty => Some("Type a name".into()),
            Self::Invalid => Some("Not a usable file name".into()),
            Self::Taken => Some(format!("“{name}” is already in this folder")),
        }
    }
}

impl SharprWindow {
    /// Libadwaita's alert dialog focuses its default button when opened from
    /// the image menu, so typing missed the field; this sheet focuses the
    /// field itself.
    pub(super) fn present_rename_sheet(&self, path: PathBuf) {
        if file_actions_blocked(self.upcast_ref()) {
            return;
        }
        let current = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let (stem, extension) = split_extension(&current);
        let extension = Rc::new(RefCell::new(extension.map(str::to_string)));

        let dialog = libadwaita::Dialog::new();
        dialog.add_css_class("sharpr-sheet");
        dialog.add_css_class("sharpr-rename");
        dialog.set_title("Rename");
        dialog.set_content_width(420);
        dialog.set_presentation_mode(libadwaita::DialogPresentationMode::Floating);

        let entry = gtk4::Entry::new();
        entry.set_text(stem);
        entry.set_hexpand(true);
        entry.set_activates_default(true);
        let chip = gtk4::Button::new();
        chip.add_css_class("sharpr-extension-chip");
        chip.set_tooltip_text(Some("Change the extension"));
        chip.set_valign(gtk4::Align::Center);
        match extension.borrow().as_deref() {
            Some(ext) => chip.set_label(ext),
            None => chip.set_visible(false),
        }
        let field = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        field.append(&entry);
        field.append(&chip);

        let problem = gtk4::Label::new(None);
        problem.add_css_class("caption");
        problem.add_css_class("error");
        problem.set_xalign(0.0);
        problem.set_visible(false);

        let cancel = gtk4::Button::with_label("Cancel");
        let rename = gtk4::Button::with_label("Rename");
        rename.add_css_class("suggested-action");
        rename.set_sensitive(false);
        let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        buttons.set_homogeneous(true);
        buttons.append(&cancel);
        buttons.append(&rename);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.append(&field);
        content.append(&problem);
        content.append(&buttons);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let heading = gtk4::Label::new(Some("Rename"));
        heading.add_css_class("heading");
        let header = libadwaita::HeaderBar::new();
        header.set_title_widget(Some(&heading));
        let view = libadwaita::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&content));
        dialog.set_child(Some(&view));
        dialog.set_default_widget(Some(&rename));
        // Focusing the field selects all of it: the name, not the extension.
        dialog.set_focus(Some(&entry));

        // The name the sheet would rename to, with the chip's extension.
        let proposed = {
            let entry = entry.clone();
            let extension = extension.clone();
            move || {
                format!(
                    "{}{}",
                    entry.text(),
                    extension.borrow().as_deref().unwrap_or("")
                )
            }
        };
        let check = {
            let entry = entry.clone();
            let extension = extension.clone();
            let current = current.clone();
            let folder = path.parent().map(Path::to_path_buf);
            let state = self.app_state();
            let rename = rename.clone();
            let problem = problem.clone();
            move || {
                let typed = entry.text();
                let extension = extension.borrow();
                let name = format!("{typed}{}", extension.as_deref().unwrap_or(""));
                let verdict = verdict(&typed, extension.as_deref(), &current, |name| {
                    folder.as_ref().is_some_and(|folder| {
                        state
                            .borrow()
                            .library
                            .index_of_path(&folder.join(name))
                            .is_some()
                    })
                });
                rename.set_sensitive(verdict == Verdict::Ready);
                let message = verdict.message(&name);
                problem.set_visible(message.is_some());
                problem.set_label(message.as_deref().unwrap_or(""));
            }
        };
        {
            let check = check.clone();
            entry.connect_changed(move |_| check());
        }
        {
            let entry = entry.clone();
            let extension = extension.clone();
            chip.connect_clicked(move |chip| {
                let Some(ext) = extension.borrow_mut().take() else {
                    return;
                };
                chip.set_visible(false);
                let stem_end = entry.text().chars().count() as i32;
                // The text changes, so `changed` re-checks the name.
                entry.set_text(&format!("{}{ext}", entry.text()));
                entry.grab_focus_without_selecting();
                entry.select_region(stem_end, -1);
            });
        }

        {
            let dialog = dialog.downgrade();
            cancel.connect_clicked(move |_| {
                if let Some(dialog) = dialog.upgrade() {
                    dialog.close();
                }
            });
        }
        {
            let window = self.downgrade();
            let dialog = dialog.downgrade();
            rename.connect_clicked(move |_| {
                let name = proposed();
                if let Some(dialog) = dialog.upgrade() {
                    dialog.close();
                }
                if let Some(window) = window.upgrade() {
                    window.rename_file(path.clone(), name);
                }
            });
        }
        check();
        dialog.present(Some(self));
    }
}

#[cfg(test)]
mod tests {
    use super::{split_extension, verdict, Verdict};

    #[test]
    fn the_extension_is_split_off_the_name() {
        assert_eq!(split_extension("photo.jpg"), ("photo", Some(".jpg")));
        assert_eq!(
            split_extension("holiday.photo.png"),
            ("holiday.photo", Some(".png"))
        );
        assert_eq!(split_extension("héllo.webp"), ("héllo", Some(".webp")));
        assert_eq!(split_extension("photo"), ("photo", None));
        assert_eq!(split_extension(".hidden"), (".hidden", None));
        assert_eq!(split_extension("name."), ("name.", None));
        assert_eq!(split_extension(""), ("", None));
    }

    #[test]
    fn names_are_checked_before_renaming() {
        let taken = |name: &str| name == "b.png";
        let png = Some(".png");
        assert_eq!(verdict("c", png, "a.png", taken), Verdict::Ready);
        assert_eq!(verdict("a", png, "a.png", taken), Verdict::Unchanged);
        // With the chip turned back into text, the field holds it all.
        assert_eq!(verdict("a.jpg", None, "a.png", taken), Verdict::Ready);
        assert_eq!(verdict("a.png", None, "a.png", taken), Verdict::Unchanged);
        assert_eq!(verdict("", png, "a.png", taken), Verdict::Empty);
        assert_eq!(verdict("  ", png, "a.png", taken), Verdict::Empty);
        assert_eq!(verdict("x/y", png, "a.png", taken), Verdict::Invalid);
        assert_eq!(verdict("..", None, "a.png", taken), Verdict::Invalid);
        assert_eq!(verdict("b", png, "a.png", taken), Verdict::Taken);
    }
}
