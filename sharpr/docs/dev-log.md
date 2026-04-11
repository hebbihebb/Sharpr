# Development Log

## 2026-04-08 Viewer Metadata / Quality OSD Polish

### What changed

- Removed the separate quality indicator block that sat above the preview image in [`src/ui/viewer.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/ui/viewer.rs), so the image remains the dominant visual element in the viewer.
- Redesigned [`src/ui/metadata_chip.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/ui/metadata_chip.rs) into a compact two-line bottom-right OSD chip that shows `dimensions · format · size` on the first line and `IQ NN% · Class` plus a five-step segmented indicator on the second line.
- Kept the existing metadata loading and IQ scoring logic intact; the polish pass only changes layout, hierarchy, and presentation.
- Added a narrow chip-specific CSS pass for rounded corners, softer dark translucency, tighter spacing, and a restrained inline quality indicator that reads as status instead of a control.

### Manual test focus

- Open bright and dark images and confirm the bottom-right OSD remains readable without dominating the frame.
- Navigate between images and confirm the chip updates dimensions, format, size, IQ score, class, and segments without layout jumps.
- Toggle metadata visibility and confirm the compact OSD still follows the existing show/hide behavior.
- Confirm the old top-of-viewer quality bar no longer appears.

### Handoff note for Claude

- The viewer still computes IQ from [`src/quality/scorer.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/quality/scorer.rs); this pass intentionally moved only the presentation into [`src/ui/metadata_chip.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/ui/metadata_chip.rs).
- The OSD chip uses a small application CSS provider installed once from the widget module; keep future polish localized there rather than adding broad app-wide styling.

## 2026-04-08 Viewer Tag OSD Entry Point

### What changed

- Added a bottom-left viewer tag OSD in [`src/ui/viewer.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/ui/viewer.rs) that surfaces the current image tags as a compact pill plus an adjacent `+` button.
- Reused the existing viewer tag popover instead of introducing a second tag-editing flow; clicking either the tag pill or the `+` button opens the same editor.
- Changed the tag-editor keyboard shortcut from bare `T` to `Ctrl+T` in [`src/ui/window.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/ui/window.rs) so tagging no longer conflicts with the global search capture behavior.
- Updated the keyboard shortcut help text in [`data/help-overlay.ui`](/home/hebbi/Projects/Sharpr/sharpr/data/help-overlay.ui) and the empty-state guidance in [`src/ui/tag_browser.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/ui/tag_browser.rs) to match the new shortcut.

### Manual test focus

- Select an image and confirm a bottom-left tag pill appears in the viewer alongside a `+` button styled consistently with the metadata OSD.
- Click the tag pill and the `+` button and confirm both open the existing tag popover.
- Add and remove tags, then confirm the bottom-left summary updates immediately.
- Press `Ctrl+T` and confirm the tag editor opens without activating search.

### Handoff note for Claude

- The bottom-left OSD intentionally reuses `TagDatabase::tags_for_path` and the existing `open_tag_popover()` / `refresh_tag_chips()` path in [`src/ui/viewer.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/ui/viewer.rs); keep that single mutation flow intact.
- The tag summary is intentionally simple for now (`first tag`, or `first tag +N`, or `Add tag`); if users need richer always-on tag display later, expand the summary without replacing the popover-backed editor.

## 2026-04-08 MVP Image Quality Scoring

### What changed

- Added `src/quality/scorer.rs`, a dedicated metadata-only scorer that computes an explainable `0..100` IQ score from width, height, file size, and format.
- Weighted the heuristic toward wallpaper suitability: long-edge resolution and megapixels drive most of the score, bytes-per-pixel works as a compression proxy, and format family adds a smaller quality bias.
- Added a compact viewer indicator under the `Preview` header with a segmented bar, `IQ: NN%`, a class label, and a short reason string.
- Added a `Quality` sidebar section with virtual folders for `Excellent`, `Good`, `Fair`, `Poor`, and `Needs Upscale`, wired through the existing smart-folder selection flow.
- Populated `ImageEntry` dimensions during library scans using lightweight header reads so quality filtering does not require full image decodes.

### Manual test focus

- Open a folder with mixed resolutions and confirm the preview header shows an IQ bar, numeric score, class, and reason for the selected image.
- Click each row in the `Quality` sidebar section and confirm the filmstrip filters to matching images.
- Navigate between images and confirm the quality indicator updates as the selection changes.

### Handoff note for Claude

- The scorer is intentionally heuristic and isolated in [`src/quality/scorer.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/quality/scorer.rs); if you tune thresholds, keep the class boundaries fixed to the product ranges.
- Sidebar quality folders currently score whatever entries are in the active filmstrip before loading the new virtual view, which keeps the integration small and consistent with the existing `load_virtual` flow.

## 2026-04-08 Substring Search Across Tags And Filenames

### What changed

- Added `TagDatabase::search_paths` so search can match tags by substring instead of exact tag equality.
- Updated tag autocomplete to use substring matching, so mid-word matches now appear in existing suggestions without changing the popover wiring.
- Changed the filmstrip search debounce from 200ms to 120ms and kept the per-keystroke search path intact.
- Search results now merge two sources: tag DB substring matches across indexed images, plus current-folder filename substring matches on the main thread so fresh unindexed files still appear instantly.
- Updated the main search activation path in [`src/ui/window.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/ui/window.rs) to use the same substring tag query for consistency.

### Manual test focus

- Open `Clean Wallz`, type `wall`, and confirm results appear without pressing `Enter`.
- Type `aurora` and confirm files with `auroraborealis` in the filename appear even if they are not yet indexed.
- Type `jpg` or `2024` and confirm substring tag matches populate the filmstrip.
- Open a fresh folder that has not been indexed yet and confirm partial filename matches still appear from the current library scan.

### Handoff note for Claude

- The worker thread only queries `TagDatabase`; the filename fallback still runs on the main thread because it borrows `AppState.library` directly in [`src/ui/window.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/ui/window.rs).
- Keep `paths_for_tag` exact-match behavior for the tag browser and any explicit exact-tag flows; `search_paths` is the substring search entry point.

## 2026-04-08 Tags Smart Folder

### What changed

- Added a `Tags` smart-folder entry to the sidebar, alongside `Duplicates` and `Search`.
- Added `TagBrowser`, a dedicated viewer-area replacement that lists tags alphabetically in letter groups and supports click-to-search plus global delete.
- Wrapped the viewer toolbar and tag browser in a shared `GtkStack`, so entering and leaving the tag browser swaps cleanly without changing the surrounding split-view layout.
- Added `TagDatabase::all_tags` and `delete_tag_globally` to support browser refresh and tag removal across the full library.

### Manual test focus

- Click `Tags` in the sidebar and confirm the preview area switches to the tag browser.
- Confirm tags appear grouped under alphabetical headings and clicking a tag loads matching images into the filmstrip.
- Delete a tag from the browser and confirm it disappears immediately and does not return after reopening `Tags`.
- Remove all tags and confirm the empty-state placeholder appears.
- Click a folder, `Duplicates`, or `Search` after opening `Tags` and confirm the normal viewer returns.

### Handoff note for Claude

- `TagBrowser::refresh()` currently rebuilds the full grouped layout on each entry or delete; keep that simple path unless tag volume proves large enough to justify a model-backed list.
- The browser intentionally reuses the existing `FilmstripPane` tag-search activation path from [`src/ui/window.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/ui/window.rs); keep that single search-loading flow rather than duplicating tag-search logic in multiple widgets.

## 2026-04-08 Tag Editor MVP

### What changed

- Added per-path tag DB helpers for read, incremental add, and incremental remove operations without full replacement writes.
- Added a viewer-owned tag editor popover anchored in the preview pane, showing current tags as removable chips with immediate entry focus.
- Wired `T` as a managed window shortcut, with focus guards so typing in the search field or other text inputs does not open the popover.

### Manual test focus

- Select an image, press `T`, confirm existing tags appear as chips and the entry is focused.
- Press `Enter` after typing a tag, confirm the chip appears and persists after reopening.
- Remove a chip with `x`, confirm it disappears immediately and stays removed after reopening.
- Press `Escape`, confirm the popover closes.
- Focus the filmstrip search bar and type `t`, confirm the popover does not open.

### Handoff note for Claude

- The popover is intentionally viewer-local and anchored inside [`src/ui/viewer.rs`](/home/hebbi/Projects/Sharpr/sharpr/src/ui/viewer.rs); keep future tag UI work on that side rather than moving it into the window header.
- `TagDatabase::add_tag` and `remove_tag` are the intended incremental mutation path for this editor; do not regress to `insert_tags` for single-tag edits.

## 2026-04-04 Phase B Hardening: Comparison Safety

### What changed

- Upscale runs now write to a pending file first, then commit moves that file into `upscaled/<name>`.
- `Discard` only deletes the pending file from the current compare session, so it no longer removes a previously committed upscale.
- Normal navigation paths now force the viewer stack back to the standard `"view"` page and hide compare actions.
- `BeforeAfterViewer::load()` now uses a generation counter and clears old textures before starting new decode work.

### Root causes fixed

- Discard previously targeted the final saved output path, so cancelling a later run could delete an already committed result.
- Viewer navigation reused the same widget stack without explicitly exiting compare mode, which left stale compare UI visible during normal browsing.
- Comparison decode threads had no request generation guard, so slow results from an older load could overwrite a newer comparison.

### Handoff note for Claude

- Commit now promotes a pending output into the final `upscaled/<name>` path; keep that temp-to-final flow intact if runner error handling is expanded.
- Any future compare-view work should keep using generation-based invalidation, matching `ViewerPane.load_gen`.

## 2026-04-04 Phase B Subsystem 1: Upscale Detector Wiring

### What changed

- Added an `AI Upscale` button to the viewer header and wired it to `UpscaleDetector`.
- Detection now happens on demand from the toolbar path instead of starting any upscale job.
- When `realesrgan-ncnn-vulkan` is missing, the viewer shows a dismissible `AdwBanner` with a clear install hint.
- When the binary is found, its resolved path is cached in `AppState` for the next Phase B subsystem.

### What this intentionally does not do yet

- no subprocess launch
- no stderr/progress parsing
- no comparison view
- no output-path generation

### Rationale

- This keeps the first Phase B increment manually testable and low-risk.
- The UI now has a real entry point for upscale capability without coupling detection to execution.
- Future subsystems can reuse the cached binary path instead of re-solving UI wiring first.

### Handoff note for Claude

- Next subsystem should consume `AppState.upscale_binary` and start the real upscale run only after an image is selected.
- Reuse the existing toolbar button path; do not add a second upscale trigger.
- Keep the missing-binary banner behavior intact and layer runner/progress work on top of it.

## 2026-04-04 Prototype Bugfix Pass

### Runtime and UX issues addressed

- Sidebar folder rows now respond on normal selection instead of relying on row activation.
  - Root cause: the sidebar used `connect_row_activated`, which is less reliable for the expected single-click navigation behavior.
- Filmstrip thumbnail requests are now wired to the background worker from the window setup path.
  - Root cause: the filmstrip had request logic in place, but no worker sender was ever provided, so rows could not enqueue thumbnail generation.
- Viewer stale async image callbacks are discarded using the `load_gen` token.
  - Root cause: late decode or metadata callbacks could update the viewer after the user had already selected a newer image.
- Viewer zoom now scales from the current paintable's intrinsic size and keeps the picture centered.
  - Root cause: zoom was based on a fixed width request, which caused weak scaling and visible drift.
- Viewer page title changed from `Image Library` to `Preview`.

### Files touched in stabilization so far

- `src/model/library.rs`
- `src/thumbnails/worker.rs`
- `src/ui/filmstrip.rs`
- `src/ui/sidebar.rs`
- `src/ui/viewer.rs`
- `src/ui/window.rs`

### Handoff note for Claude

Current prototype status after this pass:

- app launches
- folder picker path works
- sidebar selection path should now behave like a normal single-click navigator
- thumbnail worker is now connected to the filmstrip request path
- async stale-load protection is in place in `viewer.rs`
- zoom behavior is improved but still MVP-level; there is still no true anchored zoom/pan viewport

Suggested next validation steps:

1. Re-run manual testing on sidebar selection for `Home`, `Pictures`, and `Downloads`.
2. Confirm thumbnails now populate for visible filmstrip rows and continue appearing while scrolling.
3. Check whether `items_changed` notifications are sufficient for thumbnail repaint on all rows.
4. If zoom still feels unstable, the next step is likely a proper scrolled/transform-based viewer rather than more `size_request` tuning.

## 2026-04-04 Sidebar Discovery Adjustment

### Sidebar folder model change

- Sidebar roots are now treated as scan roots instead of directly assuming they are meaningful image folders.
- Discovery logic is intentionally shallow for MVP:
  - inspect each root itself for supported image files
  - inspect immediate child folders for supported image files
  - include only folders that directly contain images
  - skip empty or dead container folders
  - avoid duplicate paths across `Pictures`, `Downloads`, and `Home`

### Current rule set

- Root order is `Pictures`, `Downloads`, then `Home`
- If a root itself contains images, it is shown directly
- Child folders are labeled with context, for example `Pictures / Trip`
- No recursive walk beyond one level in this pass

### Rationale

- This better matches how users actually organize photos in an MVP prototype
- It avoids an expensive recursive scan of all of `Home`
- It keeps the sidebar implementation simple while making the folder list much more actionable

## 2026-04-04 Thumbnail Scheduling And Cache Pass

### Thumbnail scheduling changes

- Thumbnail generation is now scheduled from the filmstrip viewport instead of from every row bind.
- The scheduler estimates the visible row range from the scroll adjustment and requests:
  - currently visible rows
  - a small nearby buffer above and below the viewport
- Pending requests are tracked per folder so the same path is not re-queued repeatedly while scrolling.

### Persistent thumbnail cache

- Added a simple on-disk cache under:
  - `$XDG_CACHE_HOME/sharpr/thumbnails`
  - or `~/.cache/sharpr/thumbnails`
- Cache key strategy:
  - stable hash of the source path
  - source file size
  - source modified timestamp seconds + nanoseconds
- This means a cached thumbnail is reused when the file at the same path has not changed.
- If the source image changes, the cache key changes and the old cache entry is ignored.

### Result path updates

- Completed thumbnail results now use the library path-to-index lookup instead of scanning the whole store.
- After each completed thumbnail, the filmstrip clears the pending marker for that path and schedules the next nearby rows.

### Follow-up work to defer

- cache cleanup / eviction for old disk thumbnails
- more precise visible-row calculation based on actual row measurements instead of an estimated row height
- request prioritization beyond simple viewport-plus-buffer scheduling
- cancellation of worker tasks that are no longer near the viewport

## 2026-04-04 Preview Containment Fix

### Viewer architecture adjustment

- The preview image now lives inside its own `gtk4::ScrolledWindow`.
- The scrolled window is the fixed preview viewport.
- The picture widget is the zoomed content inside that viewport.

### Why this is the correct MVP containment model

- zoom should change only the image content size
- clipping and overflow should be owned by the image viewport
- the surrounding UI layout should remain fixed
- scrolling/panning should happen inside the preview pane, not by resizing parent layout widgets

### Current ownership model

- `ViewerPane` overlay remains the stable container
- `ScrolledWindow` owns the visible preview area and clipping
- `Picture` owns the image paintable and zoomed size request
- metadata chip and spinner remain overlay elements above the fixed viewport

### Follow-up work to defer

- mouse-centered zoom anchored to pointer position
- smoother pan behavior and drag-to-pan
- explicit preview border / framing polish

## 2026-04-04 Viewer Interaction MVP Pass

### Interaction updates

- Added drag-to-pan inside the contained preview viewport.
- Panning works by updating the `ScrolledWindow` adjustments directly during a drag gesture.
- Zoom now uses the last known pointer position in the preview viewport as its focal point.

### Ownership model

- `ScrolledWindow` remains the fixed viewport and owns the visible region
- `Picture` remains the zoomed image content
- drag and zoom only modify viewport adjustments or picture content size
- surrounding UI remains outside the interaction path

### Rationale

- Drag-to-pan is the simplest useful MVP interaction once the preview is contained
- Adjustment-based panning fits naturally with the current `ScrolledWindow` architecture
- Pointer-biased zoom is a practical improvement over top-left anchoring without requiring a custom rendering widget

### Remaining rough edges to defer

- true mathematically exact mouse-anchored zoom in all centering edge cases
- cursor changes for grab / dragging states
- kinetic panning or touch gestures

## 2026-04-04 Git Process Note

### Commit hygiene

- We should be more proactive about using git for meaningful checkpoints during prototype work.
- This stabilization/prototype pass was large enough that it would have been healthier as multiple commits instead of one long-running working tree.

### Suggested checkpoint pattern

1. Commit crash fixes and stale async load protection
2. Commit sidebar discovery and folder-selection behavior
3. Commit thumbnail scheduling and disk cache
4. Commit preview containment and viewer interaction improvements

### Working rule going forward

- Prefer committing after each coherent runtime milestone that:
  - changes one subsystem
  - is manually testable
  - leaves the app in a runnable state
