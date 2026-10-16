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
| D18 | 2026-10-12 | Always visible: resolution and format, and the image's buckets. Everything else moves to the Info panel. Confirmed on 2026-10-13: the corner chip is trimmed to resolution, format with a below-4K hint, and buckets; file size, the quality meter, and the rest go to an Info panel (i key or right-click → Info). |
| D19 | 2026-10-12 | After the data-safety work, finding comes first. Then come visual-search evaluation, file identity and buckets, visual search, wallpaper output, cleanup, and Compare. |
| D20 | 2026-10-13 | The Tags page and the page switcher are removed now, ahead of D1, to calm the header. The tag popover (Ctrl+T), tag-backed buckets, bucket colours, and the automatic-tag cleanup remain until visual search ships. No tag data is deleted. |
| D21 | 2026-10-13 | Image actions (Resize, Crop, and WebP Copy; Compare) live in the right-click menu and shortcuts, not in the ≡ main menu. ≡ is an app menu (View, Library, Preferences, Shortcuts, Manual, About). Right-click actions must also be reachable from the keyboard. |
| D22 | 2026-10-13 | Visual search goes ahead with EmbeddingGemma 2 running natively through llama.cpp, the user's choice. In blind judging it passed all 30 queries and every category (88% of its top 20 relevant, against SigLIP 2's 85%). This triggers tag retirement (D1) in Phase 5. |
| D23 | 2026-10-13 | The visual-search model is not bundled with Sharpr. The user installs its files once into a model folder Sharpr checks, separate from the irreplaceable data directory; the native installer can place them. This keeps the app about 870 MB smaller and lets the model change without a rebuild. The app still downloads nothing. The user deferred to Claude's judgement. |
| D24 | 2026-10-13 | Sharpr runs EmbeddingGemma through `llama-server` as a child process it manages. The process talks only over loopback with a random key, starts when indexing or searching needs it, and stops when idle. Its 1.3 GB of memory isn't held all the time, and a runtime crash can't take Sharpr down. The benchmark already proved this path. The user deferred to Claude's judgement. |
| D25 | 2026-10-15 | The Delete key and right-click → Move to Trash in the filmstrip trash at once, without a question, then show "Moved N images to the trash · Undo" for a few seconds. Undo puts the files back where they were. Find Copies keeps its confirmation, because it trashes many files at once. Quick pickers (Add to Bucket, WebP export) close on a click outside them, like Cancel. Ctrl+A in the search box selects the typed text; elsewhere it selects every image shown. The user's choices after testing `ef91d32`. |
| D26 | 2026-10-15 | Sharpr's look is a hybrid (Phase 9). Libadwaita stays for light and dark, the accent colour, and notices; Sharpr's own stylesheet replaces its header layout, menu sizes, and centred dialogs. Chosen from the `look-prototype` screenshots beside today's Sharpr and Strata. The next pass also tries a monospace font, tighter lists, and a shortcut footer in Compare and crop. The user's notes: the top row's parts should sit over their columns (search centred over the viewer), the sidebar should be narrower and tighter, and Filter and Sort should become one control above the filmstrip in place of the "Photos" title. |
| D27 | 2026-10-15 | Sharpr's own design language, chosen from look-prototype pass 2 and a hands-on trial: one flat 42 px bar per column (an empty bar over the sidebar; the sidebar toggle and one Sort and Filter button over the filmstrip, with no folder name; search centred over the viewer); 4 px corners; JetBrains Mono for the whole interface; compact menus; quick pickers as popovers; a narrow, tight sidebar; a neutral dark canvas behind images; the corner pill as a one-line readout that shows while the pointer moves; one line of key hints in Compare and crop. Filters are chips (option A): the Sort and Filter menu adds the same chip as typing it, so what is active always shows, and "closest to 16:9" stays a sort. This settles N2. |
| D28 | 2026-10-15 | Compare and crop replace search in the viewer's existing top bar while active; returning restores search and filters. Compare: Side by Side/Split/Flip, Linked zoom, Fit/100%, Undo, Done; per-side pickers and clearly labelled Keep Left/Keep Right buttons accompany the images. Left/Right arrows keep that side and dismiss the other; Ctrl+Z undoes, Space flips, Escape finishes, with survivors selected and no file changes. Crop: 16:9/Free, live pixel dimensions, destination, Cancel and Save Copy in the top bar; quiet drag/Enter/Escape hints below. Save Copy creates a new lossless WebP and never changes the original. Candidate thumbnail navigation needs its own proposal. Approved by the user after discussion. |
| D29 | 2026-10-15 | Compare replaces the two dropdown pickers with a compact horizontal candidate thumbnail row below the canvas, in initial filmstrip order. L/R badges identify the pair. An explicit Replace Left/Right selector starts on Right. Choosing changes that side without dismissing either image; choosing the opposite image swaps them. Keep removes the loser; Undo restores its original position. Per-side filenames/details/Keep buttons and a quiet remaining count stay. Ctrl+Shift+C opens Compare for two or more selected images, guarded while typing, in dialogs or cropping, with context-menu and Shortcuts hints. Approved by the user after discussion. |
| D30 | 2026-10-15 | The user authorizes completing Phase 9 and then installing its test build; off-screen screenshots are preferred. Finish the wanted fast-scroll handle, Survey, add-to-last-bucket and hover-expanded Info, the one-library-root change and the Phase 9 polish/inventory requirements. Survey narrows the selected images in a grid through dismiss/Undo, returning survivors selected; the user explicitly confirmed that workflow. Retain originals, legacy settings, bucket data and the proven thumbnail scheduler. Routine UI details follow D26–D29. |
| D31 | 2026-10-15 | Within the authorized Phase 9 completion (D30), Ctrl+Shift+B repeats the last successful bucket destination, persisted with its library identity and named in the image menu. Deleted/foreign/disabled destinations cannot receive writes. Hover expands the existing readout into Info after a short delay; leaving collapses it, clicking/Ctrl+I pins it, and Escape or another click closes it. All entry points reuse the existing writers and metadata. |
| D32 | 2026-10-15 | One-root implementation within D30/D11: edit the active library root in place, retaining its identity and every inactive library configuration and legacy option. Browse subfolders recursively; retain exclusions outside a narrowed root for later widening. Outside-root or disabled bucket members are hidden without missing counts or file probes there. Remove the library list, switcher, folder-mode editor and unused bucket import/export controls; stored data stays intact. |
| D33 | 2026-10-15 | Phase 9 finishing within D30/D27: one stylesheet/provider, one shared bucket palette/drawing tokens, original namespaced symbolic icons and CSS thumbnail feedback. The large viewer uses the filmstrip image menu and action group. Retire unused overlay/cache controls and the indexing ring; preserve their settings, with contextual Info always available. Keep indexing/fingerprint status in Library Overview. One Help sheet groups the guide, shortcuts, About and legacy tag maintenance actions; existing confirmation/backup behavior stays. |
| D34 | 2026-10-15 | Phase 10 release choices (the user's): the release is 0.5.0, with `Cargo.toml` as the one version source. "Open with Sharpr" is removed (no `%F`/`MimeType`); Sharpr stays library-only. The Flatpak bundles a Vulkan build of `llama-server` for GPU indexing now, with the processor as fallback. The Flatpak may write the home folder, so Rename, Move, Trash and exports work as in the native build. |
| D35 | 2026-10-15 | Edit buckets (the user's request, after 0.5.0): Crop Copy, WebP export and Duplicate add both the original and the new copy to a per-action bucket ("Cropped", "Exported", "Duplicated") under one "Edits" parent, created when needed, so a copy can be compared with its original wherever it was saved. On by default, with a Preferences switch to turn it off. Nothing moves or changes files. |
| D36 | 2026-10-16 | The user requests a toggleable 16:9 landscape border/ruler in the viewer as a visual aid for judging whether an image is wider or taller than wallpaper proportions. It changes no image pixels or files. Planned in `ROADMAP.md`; control placement and interaction are settled during implementation. |
