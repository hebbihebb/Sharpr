# Sharpr Product Vision

Approved by the user on October 12, 2026, after a product-design interview. This document defines *what* Sharpr is and *why*; `ROADMAP.md` defines *when*; `AGENTS.md` defines the rules agents must follow. Change this file only with the user's approval, and record each change in the decision log below.

## Vision

Sharpr is a fast, offline Linux app for finding, curating, and preparing wallpaper-grade images from a large, messy local collection: thousands of downloaded images, generated artwork, and personal photos, often with meaningless filenames and unreliable metadata.

- **Browse:** real folders, in a fast filmstrip and viewer.
- **Find:** describe images in words (visual search) and add exact format, resolution, and aspect filters.
- **Keep:** hand-curated buckets that survive files being moved outside Sharpr.
- **Prepare:** non-destructive crop and batch WebP export.
- **Hand off:** generation, enhancement, and upscaling stay in external tools, which work through ordinary folders.

## Requirements By Firmness

| Non-negotiable | Preference (designed, adjustable) | Experiment (must prove itself) |
| --- | --- | --- |
| Originals are never modified | Start typing anywhere to search, plus Ctrl+F | Visual search: EmbeddingGemma 2 against alternatives |
| Filmstrip and viewer stay fast | Filter words typed in the search box become chips | Retiring tags, which depends on visual search passing |
| Buckets, including nesting with inheritance | Current folder first, with an explicit scope switch | The in-app visual-search trial |
| Bucket membership survives moves and renames made outside Sharpr | Pairwise Compare with A/B, split, and zoom lock | |
| Tag and bucket data are never silently deleted | Info panel; only resolution and buckets always visible | |
| Disabled folders are respected; the app is offline | WebP lossless by default | |

## Workflows

1. **Find.** Type words and filter words (`mountains 4k png 16:9`) while in a folder or bucket. Broaden the scope explicitly if needed. Select the good results and add them to a bucket.
2. **Curate.** Buckets are both long-term collections and short-lived project staging. Creating a bucket from a selection, adding to it, removing from it, and deleting it must all be quick, and none of these touch files.
3. **Prepare a wallpaper.**
   - Find the image and crop a 16:9 copy.
   - Copy it into your own working folder, with subfolders such as `edit/`, `diffusion/`, and `upscale/`, and use external tools there.
   - To compare versions across those subfolders, gather them in a temporary bucket and open Compare.
   - Batch-export WebP to the Wallpapers folder.
4. **Clean up.**
   - Find exact copies, resized or converted copies of the same picture, and images below 4K.
   - Keep the best copy, usually the highest resolution, and trash the rest after confirming.
5. **View.** Double-click for fullscreen. There is no slideshow; the desktop screensaver does that.

## UX Principles

1. **Browsing never waits.** Nothing may stall the filmstrip or viewer.
2. **Current scope first.** Search and filters apply to the current folder or bucket, and broadening is an explicit action.
3. **One search box.** Words go to search; recognized filter words (formats, resolutions, aspects) become removable chips. An ambiguous word becomes a chip you can turn back into text.
4. **Search finds; buckets keep.** No saved searches or smart collections.
5. **Honest results.** Show index coverage and match strength. Never present weak "closest available" matches as real ones.
6. **Quiet interface.** Only resolution with format (plus a below-4K hint) and bucket membership are always visible. Everything else lives in an Info panel opened from a key or the context menu.
7. **Explicit actions.** Nothing destructive happens without confirmation. Dismissing an image from Compare, or deleting a bucket, never touches files.
8. **External tools do the heavy work.** Sharpr makes moving images between folders smooth.

## Non-Goals

- tags as a user-facing concept, once visual search passes
- saved or smart searches
- a hiding or exclusion feature
- version tracking, lineage, or "image families"
- slideshow
- further investment in multiple libraries; existing library data keeps working
- bucket import/export
- JPEG XL or AVIF output, target file sizes, and upscaling (viewing JPEG XL stays)
- detecting crops or near-identical variants as duplicates
- detecting blur or artifacts
- processing queues or job history
- Space as the search shortcut
- a V2 rewrite (see `ROADMAP.md`)

Removed features listed in `AGENTS.md` stay removed.

## Data Commitments

- **Tag tables are never dropped.** The one exception is the approved cleanup of automatic tags (D17), which runs after a backup and the user's confirmation, and deletes only known automatic tag names that no bucket uses.
- **Moving buckets to their own membership lists is non-destructive:**
  - membership is built from today's effective-tag membership
  - the result is verified to have identical member counts per bucket
  - tag rows are left untouched, so the move is reversible
  - it runs only after a backup and after database versioning exists
- **When tags are retired,** every manual tag that no bucket uses is offered to the user as a new bucket. Afterwards, the rows stay in the database, unused.

## Decision Log

| # | Date | Decision |
| --- | --- | --- |
| D1 | 2026-10-12 | Tags are frozen now (no repair work) and removed only after visual search passes Phase 3. Data-safety fixes still apply, because buckets depend on tags until Phase 4. |
| D2 | 2026-10-12 | No hiding feature. The old manual labels were for gathering images, and buckets cover that. |
| D3 | 2026-10-12 | Buckets remember images by content fingerprint and re-link them after moves and renames made outside Sharpr. |
| D4 | 2026-10-12 | Nested buckets keep inheritance: a parent shows its own images plus every image in its sub-buckets. |
| D5 | 2026-10-12 | No saved searches. "Add selected results to a bucket" must be fast. |
| D6 | 2026-10-12 | Search covers the current folder or bucket by default, with an explicit scope switch to broaden it. |
| D7 | 2026-10-12 | Start typing anywhere to search, plus Ctrl+F. Space stays free for viewing and comparing. |
| D8 | 2026-10-12 | One pairwise Compare tool serves both version inspection and choosing the best of several. It works on N selected images, shows two at a time (A/B flip, split slider, or side by side), lets the user dismiss one so the next comes in, and offers a zoom-lock toggle. Survivors stay selected afterwards. |
| D9 | 2026-10-12 | External tools round-trip through an ordinary working folder. Sharpr tracks no versions. |
| D10 | 2026-10-12 | Comparing images from several folders uses a temporary bucket; no new feature is needed. |
| D11 | 2026-10-12 | Fullscreen yes, slideshow no. One library. Bucket export is unused. |
| D12 | 2026-10-12 | Quality views are replaced by resolution filters, including "below 4K". "Low quality" means low resolution. |
| D13 | 2026-10-12 | Sharpr replaces the user's XL Converter use: batch WebP export (lossless by default, lossy as an option), an optional fit-to-size that only downscales, a background job with progress and cancel, a chosen destination, and unique names. |
| D14 | 2026-10-12 | "Duplicates" means exact copies plus resized or converted copies only. |
| D15 | 2026-10-12 | Visual-search evaluation queries must cover subjects, style, mood and colour, and people or adult content. |
| D16 | 2026-10-12 | Filters are typed into the search box. `4k` means at least 3840×2160. |
| D17 | 2026-10-12 | The backed-up, confirm-first cleanup of automatic tags stays approved. |
| D18 | 2026-10-12 | Always visible: resolution and format, and the image's buckets. Everything else moves to the Info panel. |
| D19 | 2026-10-12 | After the data-safety work, finding comes first. Then come visual-search evaluation, file identity and buckets, visual search, wallpaper output, cleanup, and Compare. |
