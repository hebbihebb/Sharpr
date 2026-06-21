# Sharpr Architecture Reference

Sharpr is a GTK4 + Libadwaita image curation app written in Rust. It uses code-first UI construction, GObject widgets, GSettings, SQLite-backed local indexes, background workers, and optional local/remote AI backends.

## Runtime Shape

- GTK widgets and `AppState` live on the main thread.
- UI state uses `Rc<RefCell<AppState>>`.
- Background work uses `std::thread::spawn` plus `async_channel`.
- UI application of background results happens through `glib::MainContext::spawn_local`.
- Stale async results are prevented with generation counters, especially for viewer loads, thumbnails, virtual views, compare mode, and task results.
- Long-running visible work should use `ops::queue` instead of one-off progress UI.

## Module Map

| Area | Responsibility |
| --- | --- |
| `src/main.rs` | Startup, `rexiv2` init, GResource registration |
| `src/app.rs` | `adw::Application` subclass, app actions, about dialog |
| `src/ui/` | Main window, panes, dialogs, Tasks, compare, preferences, help |
| `src/ui/window/` | Main shell wiring and `AppState`; avoid adding new behavior directly here |
| `src/model/` | `ImageEntry`, folder nodes, `LibraryManager` |
| `src/library_index/` | Persistent SQLite library index and migration logic |
| `src/tags/` | SQLite-backed tags, tag indexing, local smart tagging |
| `src/thumbnails/` | Background thumbnail decode, two-queue scheduling, cache |
| `src/image_pipeline/` | Shared preview decode and metadata workers |
| `src/quality/` | Resolution-based quality scoring and thumbnail sharpness helper |
| `src/duplicates/` | Perceptual duplicate detection |
| `src/export/` | Export pipeline for JPEG, JXL, PNG, WebP |
| `src/upscale/` | CLI, ONNX, and ComfyUI upscale backends |
| `src/ops/` | Background operation progress model |
| `src/config/` | GSettings-backed app settings |
| `src/bench.rs` | Structured JSONL logging macros and writer |
| `data/` | GSettings schema, desktop metadata, GResource manifest, help assets, icons |
| `packaging/` | Flatpak manifest and vendored source metadata |

## Data Flow

1. User opens a folder.
2. `LibraryManager` scans image paths without blocking the UI and keeps the `gio::ListStore<ImageEntry>` plus path indexes in sync.
3. `library_index` persists metadata and curation state for speed and reconciliation.
4. Filmstrip requests thumbnails through the two-queue thumbnail worker: visible thumbnails first, preload work second.
5. Viewer loads full-resolution images asynchronously through Glycin.
6. Metadata, tags, duplicate hashes, quality views, exports, and upscales run in background paths and apply results only if their generation is current.
7. Tasks records long-running export/upscale/format work and should be the visible dashboard for generated outputs.

## State Rules

- `LibraryManager` is the central image list store and cache coordinator.
- Keep store state, path indexes, selected index, and caches consistent after move, rename, trash, virtual view, and collection changes.
- Disabled folders must be respected by every scanning/query surface.
- Focused Image Sets are temporary views, not replacement libraries.
- SQLite may accelerate and remember curation state, but folder contents remain authoritative.

## UI Boundaries

- Keep UI work in the relevant `src/ui/` module.
- Do not add new behavior directly to `src/ui/window/` when touching compare mode, collection dialogs, or viewer layout wiring. Extract the relevant chunk first or use an existing focused module.
- `tasks_page.rs` is large; extract helpers when useful, but do not split it preemptively unless a Tasks feature makes the boundary necessary.
- Prefer existing action, dialog, popover, overlay, and preferences patterns already present in the app.

## Persistence

- Add persisted preferences in both `data/io.github.hebbihebb.Sharpr.gschema.xml` and `src/config/settings.rs`.
- Clamp and sanitize loaded values in Rust even when the schema provides defaults.
- Prefer focused setters on `AppSettings`.
- For index work, prefer loading existing SQLite rows before filesystem reconciliation and batch writes in transactions.

## Assets And Packaging

- Runtime assets belong under `data/`.
- If an asset must be embedded, update `data/io.github.hebbihebb.Sharpr.gresource.xml`.
- Check whether `build.rs` needs `cargo:rerun-if-changed` updates for new schemas/resources.
- Native and Flatpak behavior should remain aligned.
- If `Cargo.lock` changes, regenerate `packaging/cargo-sources.json`.
