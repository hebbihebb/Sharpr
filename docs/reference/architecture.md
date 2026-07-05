# Sharpr Architecture Reference

Sharpr is a GTK4 + Libadwaita image curation viewer written in Rust. It uses code-first UI construction, GObject widgets, GSettings, SQLite-backed local indexes, background workers, and no runtime networking.

## Runtime Shape

- GTK widgets and `AppState` stay on the main thread in `ui/state.rs` using `Rc<RefCell<_>>`.
- Heavy decoding, scanning, hashing, metadata work, and resizing use worker threads plus `async_channel`.
- UI results return through `glib::MainContext::spawn_local`.
- Generation counters reject stale viewer, thumbnail, virtual-view, and Compare work.
- `ops::queue` tracks transient work only; there is no persistent task queue.

## Module Map

| Area | Responsibility |
| --- | --- |
| `src/ui/window.rs` | Main shell and cross-pane wiring; avoid adding feature controllers here |
| `src/ui/state.rs` | Shared `AppState` and `ViewScope` definitions |
| `src/ui/compare_*` | Generic two-path comparison model, page, and controller |
| `src/ui/resize_dialog.rs` | Resize Copy dialog and worker orchestration |
| `src/export/` | Focused Resize Copy service and encoders |
| `src/library_index/` | SQLite curation/index persistence |
| `src/library_index/schema.rs` | Fresh-database schema initialization |
| `src/model/` | `ImageEntry`, folder nodes, and `LibraryManager` |
| `src/thumbnails/` | Visible-first background thumbnail scheduling and cache |
| `src/image_pipeline/` | Viewer preview and metadata workers |
| `src/tags/` | Manual tags plus automatic format/resolution tags |
| `src/duplicates/` | Perceptual duplicate detection |
| `src/quality/` | Resolution-based quality views |
| `src/ops/` | Transient background-operation events |

## Data Rules

- Folders remain authoritative. SQLite accelerates and remembers curation state.
- `LibraryManager` store state, path indexes, selected index, and caches must stay synchronized.
- Disabled folders apply to every direct, indexed, virtual, duplicate, quality, collection, metadata, hash, and tag path.
- Compare receives exactly two ordered paths and holds no persistent source/output model.
- Resize Copy decodes displayed orientation, resamples with Lanczos, writes a unique new file, strips embedded metadata by re-encoding, and removes partial files on failure.
- Legacy task tables and obsolete GSettings keys are never read, written, migrated, or dropped at runtime.

## Follow-Up Boundaries

`window.rs` and `library_index/mod.rs` remain large. Split sidebar/filmstrip orchestration and image/collection/reconciliation persistence when feature work naturally reaches those areas; do not perform a broad rewrite.

## Packaging

The Flatpak contains no Real-ESRGAN or ONNX Runtime module. If `Cargo.lock` changes, regenerate `packaging/cargo-sources.json`.
