# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

Dependencies (Fedora): `sudo dnf install gtk4-devel libadwaita-devel gexiv2-devel pkg-config gcc`

```bash
cd sharpr

# Native run (requires compiled GSettings schema)
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR=data cargo run

# Release build
cargo build --release

# Flatpak (recommended for distribution testing)
cd packaging
flatpak-builder --force-clean --user --install build-dir io.github.hebbihebb.Sharpr.yml
```

`build.rs` compiles `data/io.github.hebbihebb.Sharpr.gschema.xml` into GSettings and bundles `data/splash.png` via GResource — both must exist for a clean build.

## Tests

```bash
cargo test                          # all tests
cargo test hamming_distance         # run by name prefix
cargo test duplicates::phash::tests # run a module's tests
```

Tests currently live only in `src/duplicates/phash.rs`.

## Architecture

Sharpr is a GTK4 + Libadwaita image library viewer (~6,100 lines of Rust). It uses a three-pane adaptive layout (sidebar / filmstrip / viewer) with background threads for thumbnail decoding.

**Key patterns throughout the codebase:**
- GTK subclassing: every custom widget follows `mod imp` + `glib::wrapper!` + `#[glib::object_subclass]`
- Main-thread state: `Rc<RefCell<AppState>>` (intentionally non-`Send`; lives on GTK main thread only)
- Background→UI messaging: `std::thread::spawn` + `async_channel::unbounded` + `glib::MainContext::spawn_local`

**Module map:**

| Module | Role |
|---|---|
| `app.rs` | `SharprApplication` — AdwApplication subclass, splash screen, about dialog |
| `ui/window.rs` | Main window, `AppState`, three-pane layout wiring |
| `ui/filmstrip.rs` | `GtkListView` thumbnail strip with factory/model binding |
| `ui/viewer.rs` | Full-res display, zoom/pan, before-after comparison slider |
| `ui/sidebar.rs` | Folder tree navigator |
| `ui/metadata_chip.rs` | Floating EXIF overlay |
| `model/library.rs` | `LibraryManager` — O(1) path lookup, LRU thumbnail cache (500 cap), prefetch cache |
| `model/image_entry.rs` | `ImageEntry` GObject (per-image metadata, bound to GtkListView) |
| `model/folder_node.rs` | `FolderNode` GObject (folder tree) |
| `thumbnails/worker.rs` | Background decode threads; separate channels for visible vs preload priority |
| `metadata/exif.rs` | `ImageMetadata` — thin wrapper around `rexiv2` for EXIF/XMP/IPTC |
| `tags/db.rs` | `TagDatabase` backed by SQLite (`rusqlite`) |
| `duplicates/phash.rs` | dHash-based duplicate detection with Hamming distance grouping |
| `upscale/runner.rs` | Spawns `realesrgan-ncnn-vulkan` subprocess for AI upscaling |
| `config/settings.rs` | `AppSettings` serialised to JSON via `serde_json` |

**Data flow:**
1. User opens folder → `LibraryManager` scans paths, populates `GListModel`
2. Filmstrip factory requests thumbnails → `ThumbnailWorker` decodes on background thread, sends `Texture` back via channel
3. Select image → viewer loads full-res; prefetches ±2 adjacent images
4. Metadata chip reads EXIF via `rexiv2`
5. Edit (rotate/flip/upscale) → in-memory transforms or NCNN subprocess, writes back to disk
