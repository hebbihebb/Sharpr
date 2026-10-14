# Sharpr Architecture Reference

Sharpr is a GTK4 + Libadwaita image organizer written in Rust. It builds its UI in code (no Blueprint), keeps settings in GSettings, persists data in two SQLite databases, runs heavy work on background threads, and needs no network access.

The October 2026 browsing and copy modules are committed in the Phase 0 checkpoint. Unfinished experimental wiring is preserved separately; it is not compiled into the default application.

## Module Map (`sharpr/src/`)

| Area | Responsibility |
| --- | --- |
| `app.rs`, `main.rs` | Application startup, resources, and the gexiv2 shim |
| `ui/window.rs` | Main shell and cross-pane wiring. `setup()` is one function of more than 2,000 lines. Do not add features to it. |
| `ui/window/` | Controllers extracted from `window.rs`: `compare_controller`, `search_controller`, `tag_controller` (bucket writes, the automatic-tag cleanup, and turning tags into buckets), and `gtk_tests` |
| `ui/filmstrip.rs` | Filmstrip list, factory and bind callbacks, and the thumbnail scheduler (visible range, buffer, trickle) |
| `ui/viewer.rs` | Full-image viewer (glycin decoding) and the info chip |
| `display_scale.rs` | Screen-sized copies of very large images for the viewer (GTK-free) |
| `library_overview.rs`, `ui/library_overview.rs` | The Library Overview: library size and formats, below-4K count, disabled folders, fingerprints and exact copies, buckets, and visual-search progress. Gathered on a worker when the window opens. |
| `fingerprint.rs` | Content fingerprints (xxh3-128) and the background pass service (GTK-free) |
| `ui/sidebar.rs` | Folder tree and buckets (the Tags page was removed on 2026-10-13, D20) |
| `ui/export_dialog.rs`, `ui/crop_mode.rs`, `export/batch.rs` | Phase 6 output: the "Export as WebP…" dialog and its background batch job, and Crop Copy in the viewer at full size |
| `ui/state.rs` | `AppState` (`Rc<RefCell<_>>`, GTK thread only) and `ViewScope` |
| `model/library.rs` | `LibraryManager`: list store, path index, selection, and metadata caches |
| `model/browse.rs` | Pure aspect, resolution, and orientation filter and sort logic |
| `thumbnails/` | Visible and preload worker pools, and the memory and disk thumbnail caches |
| `image_pipeline/` | Viewer preview and metadata workers |
| `library_index/` | SQLite index of folders, images, and buckets (`schema.rs` sets up new databases) |
| `tags/db.rs` | SQLite manual tags and sharpness store |
| `export/`, `collection_export.rs` | WebP copies (crop and batch export) with safe output reservation, and crop geometry; bucket import and export |
| `copies.rs`, `ui/copies_dialog.rs` | Phase 7 "Find Copies…": exact copies by fingerprint, and resized or converted copies by visual-search vectors (cosine ≥ 0.99, or 0.95–0.99 with a 32×32 pixel check; every pair needs the same aspect), then review and confirmed trash |
| `duplicates/`, `quality/` | Perceptual hashes (still computed by the thumbnail workers, no longer shown) and sharpness |
| `ops/`, `ui/ops_indicator.rs` | Events for transient background operations. There is no persistent queue. |
| `config/settings.rs` | GSettings wrapper. Keys for removed features stay declared but are not used. |
| `bench.rs` | Structured JSONL event log (see `development.md`) |
| `visual_search/` | Visual search (Phase 5, GTK-free): model files (`files`), `llama-server` children and their loopback client (`runtime`), image preparation (`prepare`), the vector file (`store`), background indexing (`index`), and query ranking (`search`) |

## Threading

- GTK objects and `AppState` live only on the main thread. Never put GTK objects behind `Arc` or `Mutex`.
- Heavy work runs on `std::thread::spawn` or `rayon::spawn` and sends results back over `async_channel`. Results are drained in `glib::MainContext::spawn_local`. There is no async runtime; do not add Tokio.
- Every result that can go stale is checked against a generation counter before it is applied. This applies to the viewer, thumbnails, virtual views, search, the viewer's bucket summary, and Compare.
- The viewer never hands GTK a very large image to upload on the main thread. An sRGB image with 8-bit channels and more than about 1.15 times the screen in each direction is shown as a screen-sized copy, made on a worker (about 30–70 ms in release for 33–270 megapixels). Zoom math uses the original size, and full resolution loads only when zooming needs more pixels than the copy has.
- **Visual search** runs EmbeddingGemma 2 in `llama-server` children that listen only on 127.0.0.1 with a random key (D24). They die with the thread that started them (`PR_SET_PDEATHSIG`). Every child runs with `--offline`, and inherited llama.cpp settings (`LLAMA_*`, `HF_*`, `MTMD_*`) are stripped, so an environment that names a remote model can't make it download or replace the installed one.
  - **Indexing:** a `visual-index` thread at nice 19 and idle I/O, requested after every completed fingerprint pass. It embeds each fingerprint without a vector or recorded failure, one image at a time, and writes each vector at once, so passes resume. Its server, with the projector, runs only while a pass has work.
  - **On the graphics card by default:** with a GPU (Vulkan) build in the model folder's `runtime-gpu/` and the `visual-search-gpu` setting on, indexing runs on the first card that `--list-devices` reports with at least 2 GB free. That is about 17 times faster than the processor, with matching vectors. With no such card, or if the GPU server fails to start or dies mid-pass, the pass continues on the processor (2 threads). Queries always use the processor build.
  - **Progress:** the service reports a pace (images per second and time left, once 5 seconds of a pass have run) and when the last pass finished. The header's activity indicator shows a progress ring while a pass runs, polled once a second. "Rebuild Index…" in the Library Overview asks the service to delete the vector file between passes, never during one.
  - **Queries:** a `visual-search` thread owns a text-only server (identical query vectors, faster start). It starts at the first search and stops after 5 minutes without one. The browse worker blocks on it for the query vector, then ranks the candidates left after scope, disabled folders, and chips. File-name matches come first, then the closest 200 ("Show More" adds 200).
  - **Match strength:** the fifth-best score after subtracting each image's closeness to generic prompts, calibrated on the judged Phase 3 sample. It isn't judged below 50 indexed candidates. The order itself stays plain closeness.
  - **Without a model,** or before anything is indexed, words match file names as before, and the status line says so.
- Content fingerprints are computed by one `fingerprints` thread at nice 19 and idle I/O priority, after launch and after folder opens. Passes over the same library are at least 30 s apart, and a pass is cancelled when the library or its disabled folders change. Hashed pages are dropped from the page cache. The table `file_fingerprints` caches results by path, size, and mtime; it is rebuildable.
- Folder open runs scanning and reconciliation on a worker. The GTK side fills the store in chunks of 100 rows and yields to the main loop between chunks.
- Search and browse use a single coalescing worker that keeps only the newest job, with a 150 ms entry delay and a cancellable library walk.

Known exceptions that still block the GTK thread are tracked in `ROADMAP.md`:
- synchronous trash, move, copy, and rename (Backlog)

Bucket and search results replace the filmstrip's rows in one `splice`, so the sorted model and list view update once per view, not once per row. Creating the row objects stays on the GTK thread: about 11 ms for 6,000 rows in a debug build.

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
| `visual-search.sqlite` | `~/.local/share/sharpr/` | visual-search vectors and failures, keyed by fingerprint and a model key (model and projector hashes plus `PREPARATION_VERSION`), and cached model-file hashes | Yes: delete it to reindex; nothing else depends on it |
| Model folder | `~/.local/share/sharpr-models/embeddinggemma-2/` | the two GGUF files and, for native builds, `runtime/llama-server` with its libraries. Installed by the user (D23); Sharpr only reads it | Yes, by reinstalling |
| Thumbnails and logs | `~/.cache/sharpr/` | disk thumbnails, bench logs, and `logs/llama-server-{images,queries}.log` | Yes |

Rules:

- **Buckets store their own members** (`bucket_members`, since Phase 4). Each row holds a bucket, a file's last known path, and its content fingerprint once known.
  - **Who is a member:** the file at that path, or any file whose cached fingerprint matches and whose size and mtime are unchanged. Identical copies are all members.
  - **Missing members:** a member found nowhere is missing. It is counted ("N images in this bucket can't be found") and never dropped.
  - **Inheritance is computed at display time:** a bucket shows its own members plus its sub-buckets'. Hidden members in disabled folders don't count as missing.
  - **Edits:**
    - Adding to a sub-bucket doesn't touch the parent's list.
    - Removing removes from the bucket and its sub-buckets, by path and by fingerprint.
    - Rename and reparent change no members.
    - Deleting a bucket deletes its rows (`ON DELETE CASCADE`).
  - **Re-linking:** after every fingerprint pass, `buckets::relink` updates stored members from cached fingerprints.
    - A member whose file is at its path gets that file's current fingerprint: the first one known, or a new one after an edit.
    - A member whose file is gone moves to an unchanged file with its fingerprint, if one exists.
    - Members are fingerprinted first in each pass. The sidebar counts and badge colours then refresh.
  - **Tags no longer decide membership.** Until the one-time migration, buckets were tag-backed: a member carried every effective tag. Those rows stay in the tag database, untouched.
  - **Migration:** `buckets::migrate` runs at startup until it succeeds. It backs up both databases to `~/.local/share/sharpr/backups/before-bucket-membership-<time>/` and verifies the backup. It writes each bucket's stored effective-tag members, missing ones included, as that bucket's own members. In the same transaction it checks every bucket's count, marks `schema_meta.bucket_membership_migrated_at`, and stamps `user_version` 2. On failure nothing is written; the app reports it, and bucket edits are refused.
- **Tags are keyed by path string.** Renames and moves made inside Sharpr call `rename_path`. Changes made outside Sharpr leave orphaned rows, which are kept. `rename_path` never overwrites them: Rename refuses a name whose missing file still has tags, and Move picks the next free name.
- **Schema versions** live in `PRAGMA user_version` (`src/db_version.rs`): 2 for the library index once bucket membership is migrated (1 before), 1 for the tag database. Older builds refuse a version-2 index; rolling back means restoring the pre-migration backup. Before any setup, a database is opened read-only, and refused untouched if its version is newer than the build supports. Schema changes stay additive. Bump the version for any change an older build must not write to, and add a migration that runs only when the stored version is lower. `schema_meta.schema_version` is legacy, written but never read.
- **Legacy tables and settings keys** (`pipelines`, `pipeline_steps`, `collection_items`, and removed-feature GSettings keys) are never dropped. The code that used them was deleted on October 12, 2026; the tables and keys remain.
  - One runtime migration still reads a legacy table. At startup, `migrate_legacy_collections_to_tags` gives the members in `collection_items` the primary tag of every bucket not yet marked `tag_migrated_at`, then marks it. It is additive: it never deletes rows. It takes no backup and does not verify counts, so it is not a verified upgrade path for old databases (see the `ROADMAP.md` Backlog).
- **Reads never delete.** Tag rows for missing or trashed files are kept and hidden: `paths_for_tag()`, `all_tags()`, and `paths_for_all_tags_checked()` filter missing paths, and `stored_paths_for_all_tags_checked()` includes them. The tags come back if the file does.
- **Bucket reads and edits go through `src/buckets.rs`** (GTK-free). They touch only the library index, so the two-database crash window for buckets is gone.

## Output Files

- Crop copies and batch WebP exports decode the source in its displayed orientation (a 3 GB decode budget) and write to a new file. Only WebP is written: crops are lossless, and exports are lossless by default or lossy at a chosen quality, optionally fitted within a size that only shrinks. Batch export runs at the lowest priority and checks for cancellation between images and before keeping each output. The output name is reserved with `create_new`, the data is written to a hidden partial file, and the partial is renamed into place. Encoder flushes are checked, and failed outputs are removed.
- Originals are only ever opened for reading. The exceptions are explicit trash, move, and rename actions. Rename and Move to… go through `file_ops::rename_no_replace`, which never replaces an existing file.
- Embedded metadata is never written.

## Packaging And Identity

- The app ID is `io.github.hebbihebb.Sharpr`. The desktop file and metainfo are in `sharpr/data/`.
- The Flatpak manifest (`sharpr/packaging/io.github.hebbihebb.Sharpr.yml`, GNOME 50) builds from an rsync of the working tree in `.flatpak-source`. It is not pinned to a Git commit. It has no ONNX or Real-ESRGAN modules.
- The version comes from `build.rs`, which derives it from the commit count. This disagrees with metainfo and with the bundle file names (`ROADMAP.md` Phase 10).
