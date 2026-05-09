# Sharpr Master Action Plan

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Plan orientation: owner-corrected, practical, opinionated product and engineering priorities

## Corrected Product Identity

Sharpr is for fast Keep/Trash quality review. Folders are the truth; SQLite exists for speed, stability, cache, task history, and curation state. The app should feel like a fast GNOME-native review desk: open a folder, move through the defining filmstrip, decide what to keep or trash, compare similar images, tag and collect the keepers, review quality and duplicates, and send export/upscale/format work through Tasks.

The non-destructive rule is strict: Sharpr does not modify original image files. Export, upscale, and format conversion create controlled outputs. The user may explicitly trash files. Sharpr does not write embedded metadata, does not export Sharpr tags to IPTC/XMP, and does not add arbitrary scripts.

Tasks is a central dashboard, not a hidden log. It should show progress, failures, generated files, accept/discard decisions, and output review. Generated files inherit source tags/collections by default and are also auto-grouped into output collections such as Upscaled, Exports, and Converted.

## Top 10 Priorities

1. Protect thumbnail loading and filmstrip reliability.
2. Make keyboard-only navigation a core acceptance requirement.
3. Make Tasks the central dashboard for queued work, background work, generated outputs, failures, accept/discard choices, and user decisions.
4. Strengthen collections as a central workflow, including default inheritance behavior for generated outputs.
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
- Rotate/orientation editing unless reintroduced as export-only behavior.
- Flathub as a real release target.
- Translations as a near-term priority.

Format conversion belongs only in export/task workflows. A future lite mode may be useful, but AI features stay for now.

## Default Workflow Decisions

- Optimize the day-to-day flow for fast Keep/Trash sorting.
- Use `Keep` and `Trash` as product language, while keeping file trash explicit and protected.
- Generated outputs inherit source tags/collections by default.
- Generated outputs are auto-added to output collections such as Upscaled, Exports, and Converted.
- Task and compare results should switch the filmstrip into a focused result set, like a temporary virtual folder.
- Remote ComfyUI/API backends are allowed when Sharpr clearly says the image will leave the machine.
- Trash should have confirmation or a clear undo path.

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

## Suggested Implementation Order

1. Add a daily-workflow manual QA checklist covering thumbnail loading, rapid folder switching, keyboard-only navigation, collections, Tasks, compare, generated outputs, and explicit trash.
2. Add or strengthen regression tests for thumbnail scheduling, stale generation handling, and rapid folder/view switching.
3. Define generated-output lineage, default tag/collection inheritance, and auto output collections, then route export/upscale/format results through Tasks.
4. Make Tasks a visible dashboard for progress, failures, generated files, accept/discard decisions, and output review.
5. Audit current virtual-folder flows and refactor toward a common reliability layer only for existing folders, collections, quality, duplicates, compare, and task-generated views.
6. Add privacy wording and UI affordances for ComfyUI/API backends, non-loopback URLs, and future export metadata preference behavior.
7. Polish GNOME-facing metadata, naming, empty states, accessible names, and shortcut/manual consistency.
8. Add migration/reconciliation tests before schema changes that affect collections, task history, generated outputs, or curation data.
