# Sharpr Roadmap

This is the active roadmap and implementation order. Historical completed work lives in `docs/archive/`.

## Product Direction

Sharpr is a fast, local-first image curation desk for Linux. The core workflow is folder review: browse, compare, decide, collect, tag, detect duplicates, and send export/upscale/format work through Tasks.

Folders are the truth. SQLite supports speed, stability, task history, metadata, collections, and curation state, but must not become the only source of truth for the user's library.

## Priorities

1. Protect thumbnail loading, filmstrip responsiveness, and rapid folder switching.
2. Make keyboard-only navigation a real acceptance path.
3. Keep Tasks as the visible dashboard for queued work, failures, generated outputs, and review.
4. Strengthen collections and tag inheritance around generated outputs.
5. Track generated, upscaled, exported, and externally edited outputs with source lineage.
6. Keep AI features opt-in and clear about local vs remote processing.
7. Prefer GTK, GNOME, OS, and proven library capabilities over custom infrastructure.
8. Add regression tests when behavior touches thumbnails, folder switching, Tasks, compare, generated outputs, collections, or keyboard flows.

## Out Of Scope

- In-place image editing, rotate/flip pixel changes, and full editor tools
- Import workflow, saved searches, batch rename, embedded metadata writing, IPTC/XMP export of Sharpr tags
- Arbitrary scripts or user-defined shell actions
- ONNX model downloader or automatic model downloads
- Sharpness backfill worker and sharpness-based quality scoring
- Splash screen
- Near-term color management work

## Current Implementation Order

### 17. Code-Bloat Survey Follow-Ups

These are independently shippable cleanup tasks from the May 2026 Codex audit. Do them when they naturally pair with touched code, not as a broad rewrite.

- 17a. Remove confirmed unused `FilterBar`.
- 17b. Remove unused `upscale_banner` placeholder.
- 17c. Restore `cargo fmt --check` in a whitespace-only commit.
- 17d. Fix clippy `too_many_arguments` in `show_rename_collection_dialog`.
- 17e. Remove unused ops progress/dismissal stubs.
- 17f. Resolve stale settings: wire `comfyui-enabled`; stop using `show-upscale-ui` in Rust while leaving the schema key unless a migration policy is added.
- 17g. Extract shared collection/tag color presentation helpers.
- 17h. Extract small repeated GAction/action-wiring helpers where simple.
- 17i. Extract file-operation selection restore and toast helpers in `window.rs`.
- 17j. Extract small Tasks-page helpers without splitting the file.
- 17l. Replace production `println!`/`eprintln!` paths with structured logging or UI feedback.

Do not use cleanup as an excuse to rewrite filmstrip scheduling, source-image safety, full-image Glycin loading, disabled-folder behavior, collection/tag coupling, or dependency architecture.

### 18. External Edit Workflow And Image Families

This is the next main feature track. Full design: `docs/features/external-edit-workflow.md`.

- 18a. Migrate tags and collections toward id-backed keys. This is a prerequisite for safe promotion because paths can change.
- 18b. Add `image_families` and `image_versions` schema. Opening an existing library should create no family rows.
- 18c. Add managed storage layout and scan exclusion for `~/Pictures/Sharpr Edits/`.
- 18d. Add manifest write and recovery action.
- 18e. Add attach workflow with one confirmation sheet and no original overwrite.
- 18f. Add version filmstrip and version-count badge using `FocusedImageSet` semantics.
- 18g. Replace compare page info chip with a centered floating OSD toolbar and split Standard vs Version Compare controllers.
- 18h. Add promotion workflow with archive verification before replacing the folder original.
- 18i. Add Tasks row output mode: Create Version vs Export Copy.
- 18j. Update Tasks history for version-created results.
- 18k. Wire MVP actions and tombstone behavior.

## Deferred Research

- Model and workflow discoverability for local AI/upscale backends: ComfyUI workflow files, local ONNX model scanning, and clearer user guidance.
- Next-generation local smart tagger evaluation, such as WaifuDiffusion or SmilingWolf ONNX taggers.
- Visual similarity grouping in the filmstrip. Research algorithm choice, performance, and UI model before implementation.
- Larger `window.rs` and `library_index` splits. Extract only when feature work makes the boundary necessary.

## Focused Image Sets

A Focused Image Set is any non-folder source that temporarily drives the filmstrip: compare queues, task results, generated outputs, duplicate groups, quality views, collection views, tag-filtered views, or future version sets.

Focused Image Sets must have a stable identity, carry a generation token, replace filmstrip contents atomically, drop stale async results, keep selection/path indexes consistent, and never bleed entries from a previous folder or focused view.

Do not degrade the current filmstrip visible/preload scheduling, generation-aware stale-drop, or LRU cache behavior while adding focused views.

## Testing Priorities

- Thumbnail loading, cache behavior, corrupt files, huge files, rapid scrolling
- Rapid folder switching while background work is active
- Filmstrip population for folders, collections, quality, duplicates, compare, and task results
- Generated-output lineage, source inheritance, and auto output collections
- Explicit trash behavior and non-modification of originals
- Keyboard-only navigation through sidebar, filmstrip, viewer, collections, Tasks, compare, dialogs, and preferences
- ComfyUI/API consent, timeout, error, and non-loopback URL handling
- SQLite migration/reconciliation tests for curation state and task history
