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
| `ui/sidebar.rs` | Folder tree and buckets (the Tags page was removed on 2026-10-13, D20) |
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
- synchronous trash, move, copy, and rename (Backlog)
- unchunked `load_virtual` after a bucket's members arrive

The per-row tag lookup, bucket membership query, and virtual-view path scans moved off the GTK thread on October 12 (Phase 2 item 11).

## Filmstrip And Thumbnail Invariants

The filmstrip is Sharpr's most performance-sensitive surface. Keep these rules:

1. Bump the thumbnail generation whenever the list's contents change (folder, bucket, search, or sort).
2. Every path added to `pending_paths` must be removed on every exit path, including failure and stale drops.
3. The GTK side never blocks on a worker channel, and workers never block on their own bounded queue. Follow-up hash requests use `queue_hash` (non-blocking), and stale requests are skipped with `drain_stale`; reuse both.
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
- **Schema versions** live in `PRAGMA user_version` (`src/db_version.rs`), currently 1 for both databases. Before any setup, a database is opened read-only, and refused untouched if its version is newer than the build supports. Schema changes stay additive. Bump the version for any change an older build must not write to, and add a migration that runs only when the stored version is lower. `schema_meta.schema_version` is legacy, written but never read.
- **Legacy tables and settings keys** (`pipelines`, `pipeline_steps`, `collection_items`, and removed-feature GSettings keys) are never dropped or migrated at runtime. The code that used them was deleted on October 12, 2026; the tables and keys remain, inert.
- **Bucket create, rename, and reparent touch both databases.** Each database commits separately, with a compensating rollback if the second fails. A crash between the two commits can leave them inconsistent; this window is accepted, and recorded in the roadmap backlog.
- **Reads never delete.** Tag rows for missing or trashed files are kept and hidden: `paths_for_tag()`, `all_tags()`, and `paths_for_all_tags_checked()` filter missing paths, and `stored_paths_for_all_tags_checked()` includes them. The tags come back if the file does.
- **Bucket edits go through `src/buckets.rs`.**
  - Removal strips only the bucket's primary tag.
  - Rename and reparent update every stored member under the library root, including disabled folders and missing files.
  - Reparent keeps tags that other buckets still need.

## Output Files

- Resize, Crop, and WebP copies decode the source in its displayed orientation and write to a new file. The output name is reserved with `create_new`, the data is written to a hidden partial file, and the partial is renamed into place. Encoder flushes are checked, and failed outputs are removed.
- Originals are only ever opened for reading. The exceptions are explicit trash, move, and rename actions. Rename and Move to… go through `file_ops::rename_no_replace`, which never replaces an existing file.
- Embedded metadata is never written.

## Packaging And Identity

- The app ID is `io.github.hebbihebb.Sharpr`. The desktop file and metainfo are in `sharpr/data/`.
- The Flatpak manifest (`sharpr/packaging/io.github.hebbihebb.Sharpr.yml`, GNOME 50) builds from an rsync of the working tree in `.flatpak-source`. It is not pinned to a Git commit. It has no ONNX or Real-ESRGAN modules.
- The version comes from `build.rs`, which derives it from the commit count. This disagrees with metainfo and with the bundle file names (`ROADMAP.md` Phase 9).
