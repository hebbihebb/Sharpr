//! Compare's replace mode (D49): each upgraded image is shown beside its
//! original, and the user chooses Replace, Skip or Keep Both. Nothing
//! changes files here; the choices go to whoever opened the review, which
//! asks for confirmation before anything is replaced.

use std::path::PathBuf;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use super::ComparePage;

/// One upgraded image and its possible originals, closest first.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewItem {
    pub new: PathBuf,
    pub candidates: Vec<PathBuf>,
    /// No strong original was found; these are weak matches the user
    /// asked to see.
    pub weak: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Replace {
        original: PathBuf,
        also_trash: Vec<PathBuf>,
    },
    Skip,
    KeepBoth,
}

type ReviewDone = Box<dyn FnOnce(Vec<(ReviewItem, Decision)>)>;

pub(crate) struct Review {
    items: Vec<ReviewItem>,
    decisions: Vec<Decision>,
    done: Option<ReviewDone>,
}

/// The Replace / Skip / Keep Both bar, in the Keep bar's place.
pub(crate) struct ReplaceBar {
    pub root: gtk4::Box,
    pub note: gtk4::Label,
    pub replace: gtk4::Button,
    pub skip: gtk4::Button,
    pub both: gtk4::Button,
    pub also: gtk4::CheckButton,
}

impl Default for ReplaceBar {
    fn default() -> Self {
        Self {
            root: gtk4::Box::new(gtk4::Orientation::Vertical, 4),
            note: gtk4::Label::new(None),
            replace: gtk4::Button::new(),
            skip: gtk4::Button::new(),
            both: gtk4::Button::new(),
            also: gtk4::CheckButton::new(),
        }
    }
}

fn action_with_key(button: &gtk4::Button, text: &str, key: &str) {
    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    content.append(&gtk4::Label::new(Some(text)));
    content.append(&crate::ui::look::keycap(key));
    button.set_child(Some(&content));
    button.add_css_class("sharpr-compare-keep");
    button.update_property(&[gtk4::accessible::Property::Label(text)]);
}

pub(super) fn build(page: &ComparePage) {
    let bar = &page.imp().replace_bar;
    bar.root.set_margin_top(8);
    bar.root.set_margin_bottom(6);
    bar.root.set_visible(false);
    bar.note.add_css_class("sharpr-compare-last");
    bar.note.set_wrap(true);
    bar.note.set_justify(gtk4::Justification::Center);
    bar.root.append(&bar.note);
    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    buttons.set_halign(gtk4::Align::Center);
    action_with_key(&bar.replace, "Replace", "R");
    action_with_key(&bar.skip, "Skip", "S");
    action_with_key(&bar.both, "Keep Both", "B");
    bar.replace.set_tooltip_text(Some(
        "The right image takes the left image's place; the left one goes to the trash once you confirm at the end",
    ));
    bar.skip
        .set_tooltip_text(Some("Leave both as they are and don't ask again this time"));
    bar.both.set_tooltip_text(Some(
        "Keep both images where they are; neither is a replacement for the other",
    ));
    for (button, decision) in [
        (&bar.replace, None),
        (&bar.skip, Some(Decision::Skip)),
        (&bar.both, Some(Decision::KeepBoth)),
    ] {
        let weak = page.downgrade();
        button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                match &decision {
                    None => page.review_replace(),
                    Some(decision) => page.review_decide(decision.clone()),
                }
            }
        });
        buttons.append(button);
    }
    bar.root.append(&buttons);
    bar.also.set_halign(gtk4::Align::Center);
    bar.also.set_visible(false);
    bar.root.append(&bar.also);
    page.imp().root.append(&bar.root);
}

impl ComparePage {
    /// Reviews `items` one at a time: the original on the left, the
    /// upgrade on the right. `done` gets every choice once the review ends
    /// (images not reached count as skipped).
    pub fn start_review(
        &self,
        items: Vec<ReviewItem>,
        done: impl FnOnce(Vec<(ReviewItem, Decision)>) + 'static,
    ) {
        *self.imp().review.borrow_mut() = Some(Review {
            items,
            decisions: Vec::new(),
            done: Some(Box::new(done)),
        });
        self.show_review_item();
    }

    pub fn reviewing(&self) -> bool {
        self.imp().review.borrow().is_some()
    }

    fn show_review_item(&self) {
        let item = {
            let review = self.imp().review.borrow();
            let Some(review) = review.as_ref() else {
                return;
            };
            review.items.get(review.decisions.len()).cloned()
        };
        let Some(item) = item else {
            self.finish_review();
            return;
        };
        let mut paths = Vec::with_capacity(item.candidates.len() + 1);
        paths.extend(item.candidates.first().cloned());
        paths.push(item.new.clone());
        paths.extend(item.candidates.iter().skip(1).cloned());
        self.imp().replace_bar.also.set_active(false);
        self.start(paths);
    }

    /// The image under review, and the pair on screen: (original, upgrade)
    /// when one side is the upgrade and the other one of its candidates.
    fn review_pair(&self) -> Option<(ReviewItem, Option<PathBuf>)> {
        let imp = self.imp();
        let review = imp.review.borrow();
        let review = review.as_ref()?;
        let item = review.items.get(review.decisions.len())?.clone();
        let shown = imp.shown.borrow().clone();
        let failed = imp.failed.get();
        let original = match (&shown[0], &shown[1]) {
            (Some(a), Some(b)) if *b == item.new && item.candidates.contains(a) && !failed[0] => {
                Some(a.clone())
            }
            (Some(a), Some(b)) if *a == item.new && item.candidates.contains(b) && !failed[1] => {
                Some(b.clone())
            }
            _ => None,
        };
        Some((item, original))
    }

    /// Updates the bar and captions for the pair on screen; hides the bar
    /// outside a review.
    pub(super) fn update_review(&self) {
        let imp = self.imp();
        let bar = &imp.replace_bar;
        let Some((item, original)) = self.review_pair() else {
            bar.root.set_visible(false);
            return;
        };
        imp.keep_bar.set_visible(false);
        imp.last_line.set_visible(false);
        bar.root.set_visible(true);
        let (at, total) = {
            let review = imp.review.borrow();
            let review = review.as_ref().unwrap();
            (review.decisions.len() + 1, review.items.len())
        };
        imp.counter.set_text(&format!("Image {at} of {total}"));
        imp.undo.set_sensitive(at > 1);
        bar.replace.set_sensitive(original.is_some());
        bar.note.set_text(if item.weak {
            "No strong original was found: these are only the closest weak matches. Replace only if the other image really is this one's original."
        } else if original.is_none() {
            "Show the upgrade beside one of its possible originals to replace it."
        } else if item.candidates.len() > 1 {
            "Several possible originals: put the one to replace beside the upgrade (L or R on its thumbnail)."
        } else {
            "The upgrade takes the original's place and name; the original goes to the trash after you confirm."
        });
        // Label weak matches on their own caption.
        if item.weak {
            let shown = imp.shown.borrow().clone();
            for (side, path) in shown.iter().enumerate() {
                if let Some(path) = path.as_ref().filter(|p| **p != item.new) {
                    imp.captions[side].name.set_text(&format!(
                        "{} · weak match",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
            }
        }
        let others: Vec<String> = item
            .candidates
            .iter()
            .filter(|c| Some(*c) != original.as_ref())
            .map(|c| {
                c.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let offer = !item.weak && original.is_some() && !others.is_empty();
        bar.also.set_visible(offer);
        if offer {
            bar.also.set_label(Some(&format!(
                "Also trash the other matched copies: {}",
                others.join(", ")
            )));
        }
    }

    fn review_replace(&self) {
        let Some((item, Some(original))) = self.review_pair() else {
            return;
        };
        let also_trash = if self.imp().replace_bar.also.is_active() && !item.weak {
            item.candidates
                .iter()
                .filter(|c| **c != original)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        self.review_decide(Decision::Replace {
            original,
            also_trash,
        });
    }

    fn review_decide(&self, decision: Decision) {
        if let Some(review) = self.imp().review.borrow_mut().as_mut() {
            if review.decisions.len() < review.items.len() {
                review.decisions.push(decision);
            }
        }
        self.show_review_item();
    }

    /// Goes back to the previous image and forgets its choice.
    pub(super) fn review_undo(&self) {
        let popped = self
            .imp()
            .review
            .borrow_mut()
            .as_mut()
            .and_then(|r| r.decisions.pop())
            .is_some();
        if popped {
            self.show_review_item();
        }
    }

    /// Ends the review: images not reached are skipped. Compare closes,
    /// then the choices go to whoever started the review.
    pub fn finish_review(&self) {
        let Some(mut review) = self.imp().review.borrow_mut().take() else {
            return;
        };
        let done = review.done.take();
        let mut decisions = std::mem::take(&mut review.decisions);
        decisions.resize(review.items.len(), Decision::Skip);
        self.imp().replace_bar.root.set_visible(false);
        self.close();
        if let Some(done) = done {
            done(review.items.into_iter().zip(decisions).collect());
        }
    }

    /// Replace mode's keys: R, S and B choose; Ctrl+Z goes back; Escape
    /// ends the review. The arrows don't keep a side here.
    pub(super) fn review_key(
        &self,
        key: gdk4::Key,
        modifiers: gdk4::ModifierType,
    ) -> Option<glib::Propagation> {
        if !self.reviewing() || self.imp().candidates.has_keyboard_focus() {
            return None;
        }
        let control = modifiers.contains(gdk4::ModifierType::CONTROL_MASK);
        let plain = !modifiers.intersects(
            gdk4::ModifierType::CONTROL_MASK
                | gdk4::ModifierType::ALT_MASK
                | gdk4::ModifierType::SUPER_MASK
                | gdk4::ModifierType::META_MASK,
        );
        match key {
            gdk4::Key::r | gdk4::Key::R if plain => self.review_replace(),
            gdk4::Key::s | gdk4::Key::S if plain => self.review_decide(Decision::Skip),
            gdk4::Key::b | gdk4::Key::B if plain => self.review_decide(Decision::KeepBoth),
            gdk4::Key::z | gdk4::Key::Z if control => self.review_undo(),
            gdk4::Key::Escape => self.finish_review(),
            gdk4::Key::Left
            | gdk4::Key::Right
            | gdk4::Key::bracketleft
            | gdk4::Key::bracketright => {}
            _ => return None,
        }
        Some(glib::Propagation::Stop)
    }
}
