# Sharpr Roadmap

Historical completed work lives in `docs/archive/`.

## Product Direction

Sharpr is a fast, local-first image curation viewer for Linux. Folders are the source of truth. The primary loop is open a folder, review the filmstrip, inspect metadata, compare two selections, tag or collect keepers, and explicitly trash rejects.

Sharpr does not run persistent processing queues, AI upscalers, model downloads, smart tagging, or runtime network clients. Resize Copy is deterministic local resampling that always creates a new file.

## Priorities

1. Protect thumbnail loading, filmstrip responsiveness, and rapid folder switching.
2. Make keyboard-only review and curation a complete acceptance path.
3. Keep viewer, Compare, tags, collections, duplicates, and quality views reliable across stale background work.
4. Improve boundaries in `window.rs` and `library_index/mod.rs` only as feature work reaches them.
5. Prefer GTK, GNOME, OS, and proven crate capabilities over custom infrastructure.

## Current Implementation Order

### 1. Viewer Reliability

- Add focused integration coverage for folder switching, viewer navigation, Compare close/return, and corrupt images.
- Audit generation counters and selection restoration around virtual views.

### 2. Curation Efficiency

- Complete keyboard-only flows for tags, collections, duplicates, quality views, Compare, Resize Copy, and trash.
- Review bulk selection feedback and action sensitivity.

### 3. Targeted Architecture Boundaries

- Extract sidebar and filmstrip coordination from `ui/window.rs` when touched.
- Split image reconciliation and collection persistence from `library_index/mod.rs`.
- Remove inert legacy settings and index declarations once branch-switch compatibility is no longer needed.

## Out Of Scope

- Persistent Tasks/queue/history and generated-output lineage
- AI upscaling, ComfyUI, ONNX inference, model downloaders, and runtime networking
- Local AI tag suggestions
- In-place pixel editing or source-file metadata writing
- Import workflows, saved searches, batch rename, full editor tools, or arbitrary scripts
- External edit/version-family workflows unless the product direction is explicitly revisited

## Testing Priorities

- Thumbnail loading, corrupt files, huge files, and rapid scrolling
- Rapid folder switching while background work is active
- Folder, collection, duplicate, quality, and tag-filtered filmstrips
- Exactly-two selection behavior and missing/corrupt images in Compare
- Resize enlargement/reduction, orientation, formats, unique naming, and partial cleanup
- Explicit trash behavior and non-modification of originals
- Fresh/existing SQLite migration and compatibility behavior
