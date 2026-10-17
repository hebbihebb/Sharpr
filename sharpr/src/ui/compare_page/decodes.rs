//! How Compare decodes (`ROADMAP.md` Work Order item 8): each side runs at
//! most one decode at a time and keeps only the newest request waiting, and
//! Fit shows a screen-sized preview that becomes the full image only once
//! zooming needs its pixels. Pure logic; `compare_page.rs` runs the threads.

use std::path::PathBuf;

use crate::ui::crop_view::needs_full_resolution;

/// How much of the image a decode returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// At most `PREVIEW_EDGE` pixels on the long edge.
    Preview,
    /// Every pixel.
    Full,
}

/// One image to decode for one side. `epoch` names what the side showed when
/// the job was queued; a result from an older epoch is dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub epoch: u64,
    pub path: PathBuf,
    pub tier: Tier,
}

/// One side's decode slot: one job runs, and only the newest request waits.
#[derive(Debug)]
pub struct Gate<J> {
    running: bool,
    waiting: Option<J>,
}

impl<J> Default for Gate<J> {
    fn default() -> Self {
        Self {
            running: false,
            waiting: None,
        }
    }
}

impl<J> Gate<J> {
    /// Returns `job` when it can start now. Otherwise it waits, replacing
    /// (and so dropping) any job already waiting.
    pub fn submit(&mut self, job: J) -> Option<J> {
        if self.running {
            self.waiting = Some(job);
            None
        } else {
            self.running = true;
            Some(job)
        }
    }

    /// The running job is over; returns the waiting one, now running.
    pub fn finish(&mut self) -> Option<J> {
        let next = self.waiting.take();
        self.running = next.is_some();
        next
    }

    /// Drops the waiting job. A running one cannot be stopped; its result
    /// is dropped when it arrives.
    pub fn clear(&mut self) {
        self.waiting = None;
    }
}

/// What a side's texture holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// The preview, or the whole image when it is no larger than one.
    Preview,
    /// The full image is being decoded.
    Loading,
    /// The full image.
    Full,
    /// The full image could not be decoded; the preview stays and is not retried.
    Failed,
}

/// What to do about the texture after a zoom, pan or resize.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Nothing,
    LoadFull,
    /// Back to the preview, to give the memory back.
    DropFull,
}

/// The preview's size for an image of `image` pixels.
pub fn preview_size(image: (u32, u32)) -> (u32, u32) {
    crate::visual_search::prepare::fit_within(image.0, image.1, crate::ui::crop_mode::PREVIEW_EDGE)
}

/// `scale` is screen pixels per image pixel as drawn. The full image is
/// wanted once the preview runs out of pixels, and given up only after the
/// view has zoomed well out again, so crossing the line is no thrashing.
pub fn next_step(held: Held, image: (u32, u32), scale: f64, device_scale: f64) -> Step {
    let preview = preview_size(image).0;
    match held {
        Held::Preview if needs_full_resolution(preview, image.0, scale, device_scale) => {
            Step::LoadFull
        }
        Held::Full if !needs_full_resolution(preview, image.0, scale * 2.0, device_scale) => {
            Step::DropFull
        }
        _ => Step::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_side_keeps_only_the_newest_request() {
        let mut gate = Gate::default();
        assert_eq!(gate.submit(1), Some(1), "an idle side starts at once");
        for job in 2..=6 {
            assert_eq!(gate.submit(job), None, "a busy side makes it wait");
        }
        assert_eq!(gate.finish(), Some(6), "only the newest is left");
        assert_eq!(gate.finish(), None);
        assert_eq!(gate.submit(7), Some(7), "idle again");
    }

    #[test]
    fn clearing_drops_the_waiting_job_but_not_the_running_one() {
        let mut gate = Gate::default();
        gate.submit(1);
        gate.submit(2);
        gate.clear();
        assert_eq!(gate.submit(3), None, "1 still runs");
        assert_eq!(gate.finish(), Some(3));
    }

    #[test]
    fn fit_holds_a_preview_and_zoom_asks_for_the_pixels() {
        let image = (5000, 1000);
        assert_eq!(preview_size(image), (2560, 512));
        // Fit in a 1000 pixel pane: 0.2 screen pixels per image pixel.
        assert_eq!(next_step(Held::Preview, image, 0.2, 1.0), Step::Nothing);
        // 100%: the preview has half the pixels needed.
        assert_eq!(next_step(Held::Preview, image, 1.0, 1.0), Step::LoadFull);
        // A decode in flight, or one that failed, is not asked for again.
        assert_eq!(next_step(Held::Loading, image, 1.0, 1.0), Step::Nothing);
        assert_eq!(next_step(Held::Failed, image, 1.0, 1.0), Step::Nothing);
    }

    #[test]
    fn a_small_image_is_its_own_preview() {
        assert_eq!(preview_size((1600, 900)), (1600, 900));
        assert_eq!(
            next_step(Held::Preview, (1600, 900), 8.0, 2.0),
            Step::Nothing
        );
    }

    #[test]
    fn the_full_image_is_dropped_only_after_zooming_well_out() {
        let image = (5000, 1000);
        // Just under the line that asked for it: kept, so no reload on return.
        assert_eq!(next_step(Held::Full, image, 0.5, 1.0), Step::Nothing);
        assert_eq!(next_step(Held::Full, image, 1.0, 1.0), Step::Nothing);
        // Back at Fit.
        assert_eq!(next_step(Held::Full, image, 0.2, 1.0), Step::DropFull);
    }

    #[test]
    fn a_dense_display_asks_for_the_pixels_sooner() {
        let image = (5000, 1000);
        assert_eq!(next_step(Held::Preview, image, 0.3, 1.0), Step::Nothing);
        assert_eq!(next_step(Held::Preview, image, 0.3, 2.0), Step::LoadFull);
    }
}
