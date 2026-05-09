# Sharpr Master Action Plan

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Plan orientation: owner-corrected, practical, opinionated product and engineering priorities

## Corrected Product Identity

Sharpr is for fast Keep/Trash quality review. Folders are the truth; SQLite exists for speed, stability, cache, task history, and curation state. The app should feel like a fast GNOME-native review desk: open a folder, move through the defining filmstrip, decide what to keep or trash, compare similar images, tag and collect the keepers, review quality and duplicates, and send export/upscale/format work through Tasks.

The non-destructive rule is strict: Sharpr does not modify original image files. Export, upscale, and format conversion create controlled outputs. The user may explicitly trash files. Sharpr does not write embedded metadata, does not export Sharpr tags to IPTC/XMP, and does not add arbitrary scripts. Rotate/flip editing is intentionally out of scope — GNOME Image Viewer (also Glycin-based) handles in-place pixel adjustments and is a natural companion app.

Tasks is a central dashboard, not a hidden log. It should show progress, failures, generated files, accept/discard decisions, and output review. Generated files inherit source tags/collections by default and are also auto-grouped into output collections such as Upscaled, Exports, and Converted.

## Top 10 Priorities

1. Protect thumbnail loading and filmstrip reliability.
2. Make keyboard-only navigation a core acceptance requirement.
3. Make Tasks the central dashboard for queued work, background work, generated outputs, failures, accept/discard choices, and user decisions.
4. Strengthen collections as a central workflow, including default inheritance behavior for generated outputs. *(Current state: collections work with tags and sub-collection tag inheritance. Auto output collections and generated-output lineage are not yet implemented.)*
5. Track generated/upscaled/exported outputs with source lineage and auto-add them to relevant output collections.
6. Keep folders as truth while using SQLite as cache, stability, task history, and curation infrastructure.
7. Harden compare and task-result virtual-folder behavior because both should switch the filmstrip into focused result sets.
8. Keep AI features and allow remote ComfyUI/API backends, but clearly label when an image will leave the machine.
9. Improve GNOME polish: app naming, metadata, screenshots, empty states, accessible labels, shortcuts, and manual QA.
10. Add regression tests even if slower when they protect thumbnail loading, folder switching, task/compare views, generated outputs, collections, or keyboard paths.

## Deferred Or Out Of Scope

- Saved searches.
- Import workflow.
- Batch rename.
- Embedded metadata writing.
- Metadata sidecar systems, except optional user-controlled PNG output sidecars.
- Exporting Sharpr tags to IPTC/XMP.
- Arbitrary scripts/user-defined shell actions.
- Full image editor features.
- Rotate/orientation editing (intentionally out of scope; use GNOME Image Viewer or another app for pixel-level adjustments — Sharpr does not modify originals). The rotate/flip menu items, actions, and save logic have been removed.
- Sharpness backfill (background thread that computed Laplacian sharpness scores) — removed. Quality is now purely resolution-based and sharpness data was not used for classification; the quality scorer now exposes only resolution-based scoring.
- ONNX model downloader — removed as orphaned dead code. ONNX upscale support now expects local model files without an unused downloader path.
- Flathub as a real release target.
- Translations as a near-term priority.
- gThumb-style multi-page property sidebar (EXIF/IPTC/XMP pages in a separate slide-in panel). The `MetadataChip` OSD is the right direction; see expandable chip decision in Default Workflow Decisions.
- Color management (lcms2/colord, ICC-aware save paths) — nice-to-have, not a blocker. Glycin likely handles ICC on decode; thumbnail and export paths are undocumented but not a current correctness concern. Revisit when export quality becomes critical.
- Complete `window.rs` refactor — `src/ui/window.rs` is ~5,270 lines and owns AppState, collection dialogs, compare mode, presentation mode, thumbnail/hash polling, action setup, and layout wiring. A full refactor is a long-term goal to be done when it makes the most engineering sense, not tied to any specific feature or master plan milestone. Until then, targeted extractions happen as prerequisites to features that touch those areas (see Implementation Order).

Format conversion belongs only in export/task workflows. A future lite mode may be useful, but AI features stay for now.

## Default Workflow Decisions

- Optimize the day-to-day flow for fast curation: browse, compare, decide, collect, and send to Tasks.
- **Trash** is an explicit destructive action: Delete key (or right-click) triggers a confirmation dialog and sends the image to the system trash. This is already functional. Trash stays as a file-system operation — it is not a collection or a tag.
- **Keep** is not a button or flag. The default state of an image is kept. A preconfigured "Keepers" collection is a future option that users could also create themselves; it should not be hardcoded as a required review concept.
- The right-click context menu should eventually lose its Trash item in favor of the Delete-key path, making the menu less busy (design direction, not yet implemented).
- Generated outputs inherit source tags/collections by default (planned; not yet implemented).
- Generated outputs are auto-added to output collections such as Upscaled, Exports, and Converted (planned; not yet implemented).
- Task and compare results should switch the filmstrip into a focused result set, like a temporary virtual folder.
- Compare view filmstrip should be driven by a smart collection populated from the Tasks history queue, showing only the items relevant to the current comparison — not the previous folder's contents. Currently the filmstrip bleeds previous folder content below the compared item; this is a known rough edge that needs a design decision and fix.
- Remote ComfyUI/API backends are allowed when Sharpr clearly says the image will leave the machine.
- **MetadataChip — Expandable OSD, No Separate Panel:** The `MetadataChip` OSD in the viewer is expandable on click/tap. Collapsed: compact chip (current behavior). Expanded: taller inline panel showing quality tier, duplicates count if any, tags, and collection membership. Dismissible by clicking outside or a second tap. Both states stay anchored bottom-right of the viewer. No separate property sidebar will be added.
- **Compare Page — Remove Right-Side Panel, Use Expandable OSD Chip:** The compare page's existing slide-out right-side info panel is removed. An expandable OSD chip in the same info-icon position replaces it, with the same expand/collapse behavior as the viewer chip. Panel content (file info, quality, dimensions, tags) migrates to the chip's expanded state. This unifies the interaction model across viewer and compare pages.

## Testing Priorities

- Thumbnail loading: visible/preload scheduling, cache hits/misses, corrupt files, huge files, and rapid scrolling.
- Rapid folder switching while thumbnails, metadata, hashes, quality scoring, and Tasks are active.
- Filmstrip population for folders, collections, quality, duplicates, compare, and task-generated results.
- Compare/task virtual-folder stale result handling.
- Generated-output tracking after export/upscale/format conversion, including source lineage.
- Tag and collection inheritance for generated outputs, plus auto output collections.
- Explicit trash behavior, confirmation or undo, and non-modification of originals.
- Keyboard-only navigation through sidebar, filmstrip, viewer, collections, Tasks, compare, dialogs, and preferences.
- ComfyUI/API backend consent, timeout, error, and non-loopback URL handling.
- SQLite migration/reconciliation tests for user-authored curation state and task history.

Slower tests are acceptable when they catch important regressions in these areas. Manual QA should focus on daily workflows rather than a full app tour: folder review, filmstrip navigation, rapid switching, Tasks, compare, collections, generated outputs, keyboard-only navigation, and trash.

## GNOME Polish Priorities

- Keep the public name Sharpr consistently across metadata, README, about dialog, screenshots, and desktop/appdata files.
- Treat Flatpak/AppStream checks as quality signals, not a release target.
- Keep app metadata honest about local-first behavior and configurable network backends.
- Add accessible names for icon-only controls and keep shortcut help, manual, and README aligned.
- Improve empty/error states for no folder, no images, no tasks, no generated outputs, no duplicates, and hidden AI features.
- Make destructive and privacy-sensitive actions explicit: trash, remote ComfyUI/API upload, and future export metadata behavior.
- Trash confirmation dialog is not yet implemented — Delete key currently sends the file to trash immediately without any prompt. A confirmation dialog is planned.
- ComfyUI non-loopback warning is not yet implemented — no warning is shown in preferences when the ComfyUI URL points outside localhost. This is a planned security polish item.
- Thumbnail cache: Sharpr uses a private `~/.cache/sharpr/thumbnails-r1/` cache with fingerprint-in-filename (`{path_hash}-{size}-{mtime_secs}-{mtime_nanos}.png`) for O(1) validity checking without reading cached files. This is intentionally better for Sharpr's use case than the freedesktop `~/.cache/thumbnails/` spec, which requires reading PNG text chunks per entry and computing URI MD5s. Add a short code comment in `sharpr/src/thumbnails/cache.rs` documenting this choice so the design intent is clear to future contributors.

## Risk List

- Thumbnail loading regression would damage the core experience fastest.
- Treating SQLite as truth could make folder hot swap, removable drives, and reconciliation fragile.
- Task-generated outputs can become disconnected from source images, tags, collections, or user decisions.
- Compare/task virtual-folder bugs can show stale or wrong images in the filmstrip.
- Tasks can become too quiet and fail to act as the user's dashboard for generated-output decisions.
- Metadata/export privacy can be misunderstood if output behavior is not explicit.
- AI backends can violate local-first expectations if remote ComfyUI/API URLs are not clearly labeled.
- Keep/Trash language can become dangerous if the actual trash action is too easy or lacks undo/confirmation.
- Scope creep toward import, batch rename, scripts, metadata writing, or editor tools would dilute the app.
- Keyboard/accessibility gaps would undermine daily usability and GNOME polish.
- `window.rs` complexity: at ~5,270 lines, `window.rs` is a maintainability risk. New feature work that adds to it without targeted extraction will make future refactoring harder. The full refactor is deferred, but extractions should be done as prerequisites to each feature that touches compare mode, collection dialogs, or viewer layout wiring (see Implementation Order, steps 4 and 7).

## Suggested Implementation Order

> **Note:** This is a living draft. The order below is a starting point, not a committed sequence. It should be revisited once the QA checklist and initial regression tests (steps 1–2) are in place and give a clearer picture of what unblocks what.

1. Add a daily-workflow manual QA checklist covering thumbnail loading, rapid folder switching, keyboard-only navigation, collections, Tasks, compare, generated outputs, and explicit trash.
2. Add or strengthen regression tests for thumbnail scheduling, stale generation handling, and rapid folder/view switching.
3. Add a short code comment in `sharpr/src/thumbnails/cache.rs` documenting the private cache design choice (fingerprint-in-filename, O(1) validity, why not freedesktop spec).
4. Extract compare controller from `window.rs` (`enter_compare_mode`, `exit_compare_mode`, `exit_compare_mode_internal`, `refresh_compare_view`, `handle_compare_selection_change`, `remove_from_compare_queue`) → `src/ui/compare_controller.rs`. *Prerequisite for step 5; first targeted `window.rs` extraction.*
5. Compare page: remove right-side info panel, add expandable OSD chip with the same content (file info, quality, dimensions, tags).
6. Viewer MetadataChip: make expandable on click, showing quality tier, duplicates count if any, tags, and collection membership in the expanded state.
7. Extract collection dialogs from `window.rs` (`show_new_collection_dialog`, `show_new_library_dialog`, `switch_active_library`, and related helpers) → `src/ui/collection_dialogs.rs`. *Second targeted `window.rs` extraction.*
8. Action map audit: add a comment-block table at the top of `setup_actions` in `window.rs` listing every registered `win.*`/`app.*` action, its label, default shortcut, and sensitivity rule. Keep menus, `ShortcutController` bindings, and shortcut help aligned going forward.
9. Define generated-output lineage, default tag/collection inheritance, and auto output collections, then route export/upscale/format results through Tasks.
10. Make Tasks a visible dashboard for progress, failures, generated files, accept/discard decisions, and output review.
11. Audit current virtual-folder flows and refactor toward a common reliability layer only for existing folders, collections, quality, duplicates, compare, and task-generated views.
12. Add privacy wording and UI affordances for ComfyUI/API backends, non-loopback URLs, and future export metadata preference behavior.
13. Polish GNOME-facing metadata, naming, empty states, accessible names, and shortcut/manual consistency.
14. Add migration/reconciliation tests before schema changes that affect collections, task history, generated outputs, or curation data.
15. *(Long-term, timing TBD)* Complete `window.rs` refactor: once targeted extractions from steps 4 and 7 are in place and the feature surface has stabilised, do a holistic split of the remaining content into focused page/scope controllers. The trigger is when further feature work in `window.rs` becomes noticeably painful, not a fixed milestone.
