# Sharpr Architecture Reference

Sharpr is a GTK4 + Libadwaita image organizer written in Rust. It builds its UI in code (no Blueprint), keeps settings in GSettings, persists data in two SQLite databases, runs heavy work on background threads, and needs no network access.

The October 2026 browsing and copy modules are committed in the Phase 0 checkpoint. Unfinished experimental wiring is preserved separately; it is not compiled into the default application.

## Module Map (`sharpr/src/`)

| Area | Responsibility |
| --- | --- |
| `app.rs`, `main.rs` | Application startup, resources, and the gexiv2 shim |
| `ui/window.rs` | Main shell and cross-pane wiring. `setup()` is one function of more than 2,000 lines. Do not add features to it. |
| `ui/window/` | Controllers extracted from `window.rs`: `compare_controller`, `search_controller`, `tag_controller`, and `gtk_tests` |
| `ui/filmstrip.rs` | Filmstrip list, factory and bind callbacks, and the thumbnail scheduler (visible range, buffer, trickle) |
| `ui/viewer.rs` | Full-image viewer (glycin decoding), tag popover, and info chips |
| `ui/sidebar.rs`, `ui/tag_browser.rs` | Folder tree and buckets; Tags page |
| `ui/resize_dialog.rs`, `ui/crop_dialog.rs` | Resize Copy, and Crop/WebP Copy dialogs |
| `ui/state.rs` | `AppState` (`Rc<RefCell<_>>`, GTK thread only) and `ViewScope` |
| `model/library.rs` | `LibraryManager`: list store, path index, selection, and metadata caches |
| `model/browse.rs` | Pure aspect, resolution, and orientation filter and sort logic |
| `thumbnails/` | Visible and preload worker pools, and the memory and disk thumbnail caches |
| `image_pipeline/` | Viewer preview and metadata workers |
| `library_index/` | SQLite index of folders, images, and buckets (`schema.rs` sets up new databases) |
| `tags/db.rs` | SQLite manual tags and sharpness store |
| `export/`, `collection_export.rs` | Copy encoders (JPEG, PNG, WebP, JPEG XL) with safe output reservation; bucket import and export |
| `duplicates/`, `quality/` | Perceptual-hash duplicates; resolution-based quality views and sharpness |
| `ops/`, `ui/ops_indicator.rs` | Events for transient background operations. There is no persistent queue. |
| `config/settings.rs` | GSettings wrapper. Keys for removed features stay declared but are not used. |
| `bench.rs` | Structured JSONL event log (see `development.md`) |
| Planned `semantic_trial` | Missing module; unfinished feature wiring is preserved on `visual-search-trial-wip`, not on the default development line |

## Threading

- GTK objects and `AppState` live only on the main thread. Never put GTK objects behind `Arc` or `Mutex`.
- Heavy work runs on `std::thread::spawn` or `rayon::spawn` and sends results back over `async_channel`. Results are drained in `glib::MainContext::spawn_local`. There is no async runtime; do not add Tokio.
- Every result that can go stale is checked against a generation counter before it is applied. This applies to the viewer, thumbnails, virtual views, search, tag popovers, and Compare.
- Folder open runs scanning and reconciliation on a worker. The GTK side fills the store in chunks of 100 rows and yields to the main loop between chunks.
- Search and browse use a single coalescing worker that keeps only the newest job, with a 150 ms entry delay and a cancellable library walk.

Known exceptions that still block the GTK thread are tracked in `ROADMAP.md`:
- per-row `tags_for_path` during filmstrip bind
- synchronous bucket open and unchunked `load_virtual`
- linear `entries_for_path` lookups in virtual views
- synchronous trash, move, copy, and rename
- the Tags page's delete-tag action

## Filmstrip And Thumbnail Invariants

The filmstrip is Sharpr's most performance-sensitive surface. Keep these rules:

1. Bump the thumbnail generation whenever the list's contents change (folder, bucket, search, or sort).
2. Every path added to `pending_paths` must be removed on every exit path, including failure and stale drops.
3. The GTK side never blocks on a worker channel, and workers never block on their own bounded queue. The current thumbnail-to-hash chaining breaks this rule (`ROADMAP.md` Phase 1).
4. Factory bind callbacks do no database or filesystem I/O.
5. Large store mutations are chunked.
6. A result is applied only after both its generation and its path lookup still match.
7. Visible requests take priority over preload. Extend the scheduler; do not rewrite it.

The thumbnail caches are an unbounded per-view memory cache, a global memory cache (500 entries by default), and a disk cache under `~/.cache/sharpr/` keyed by path hash, size, and mtime.

## Persistence

| Store | Location | Contents | Rebuildable? |
| --- | --- | --- | --- |
| `library-index.sqlite` | `~/.local/share/sharpr/` | folders, images (oriented dimensions, quality, phash), bucket definitions (`collections`, including colour and icon), and legacy `pipelines`, `pipeline_steps`, and `collection_items` | Bucket definitions: **no**. Everything else: yes. |
| `tags.sqlite3` | `~/.local/share/sharpr/` | `file_tags(path, tag)` and `image_quality` (sharpness) | Tags: **no** |
| GSettings `io.github.hebbihebb.Sharpr` | dconf | libraries, disabled folders, and preferences | No |
| `~/.config/sharpr/settings.json` | config dir | legacy file; current code does not read it | n/a |
| Thumbnails and logs | `~/.cache/sharpr/` | disk thumbnails and bench logs | Yes |

Rules:

- **Buckets are tag-backed.** A bucket stores only its name, primary tag, extra tags, and parent. Its members are the paths that carry *every* effective tag, meaning the bucket's own tags plus all of its ancestors' tags. Membership is never stored separately. `ROADMAP.md` Phase 4 changes this. Each bucket gets its own membership list, keyed by content fingerprint and keeping the image's last known path. Inheritance is computed at display time. Tag rows are left in place, unused.
- **Tags are keyed by path string.** Renames and moves made inside Sharpr call `rename_path`. Changes made outside Sharpr leave orphaned rows.
- **There is no schema version.** `schema_meta.schema_version` is written but never read, and `user_version` is 0. Changes have so far been additive: `CREATE IF NOT EXISTS` and add-missing-column checks. Keep them additive. `ROADMAP.md` Phase 1 adds real versioning.
- **Legacy tables and settings keys** (`pipelines`, `pipeline_steps`, `collection_items`, and removed-feature GSettings keys) are never dropped or migrated at runtime. Their code is compiled out with `#[cfg(any())]`.
- **Bucket create, rename, and reparent touch both databases.** Each database commits separately, with a compensating rollback if the second fails. A crash between the two commits can leave them inconsistent; this window is accepted, and recorded in the roadmap backlog.
- **Two known destructive paths exist; do not extend them** (`ROADMAP.md` Phase 1 removes them):
  - `TagDatabase::all_tags()` and `paths_for_tag()` delete tag rows for paths that are missing on disk.
  - Removing an image from a bucket strips all of the bucket's effective tags, including inherited ancestor tags.

## Output Files

- Resize, Crop, and WebP copies decode the source in its displayed orientation and write to a new file. The output name is reserved with `create_new`, the data is written to a hidden partial file, and the partial is renamed into place. Encoder flushes are checked, and failed outputs are removed.
- Originals are only ever opened for reading. The exceptions are explicit trash, move, and rename actions. In-app rename still uses an unchecked `std::fs::rename`, which can overwrite an existing file (`ROADMAP.md` Phase 1).
- Embedded metadata is never written.

## Packaging And Identity

- The app ID is `io.github.hebbihebb.Sharpr`. The desktop file and metainfo are in `sharpr/data/`.
- The Flatpak manifest (`sharpr/packaging/io.github.hebbihebb.Sharpr.yml`, GNOME 50) builds from an rsync of the working tree in `.flatpak-source`. It is not pinned to a Git commit. It has no ONNX or Real-ESRGAN modules.
- The version comes from `build.rs`, which derives it from the commit count. This disagrees with metainfo and with the bundle file names (`ROADMAP.md` Phase 9).
