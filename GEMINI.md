# Sharpr Project - Agent Instructions & Context

This file serves as the core set of instructions and mandates for Gemini CLI and other AI coding agents working on the Sharpr project.

## Project Overview

Sharpr is a high-performance, local-first image curation tool and viewer for Linux. It is designed to browse, organize, and curate large local image libraries without a heavy monolithic database.

- **Technologies**: Rust (1.75+ stable), GTK4 (4.12+), Libadwaita (1.5+), GExiv2 (`rexiv2`), SQLite.
- **Architecture**: Strict separation of Navigation (sidebar) and Viewing (filmstrip/viewer). It uses local caching and background workers for zero-latency navigation.
- **Project Structure**: The core Rust application resides in the `sharpr/` directory.

## Development Workflow

- **Commit Strategy**: Use a new git commit for each separate user task. Do not push to GitHub unless explicitly asked.
- **Bug Fixes**: If the user reports a bug in an unpushed commit, fix it and amend that task's commit instead of creating noisy follow-up commits.
- **Quality Checks**: Before handing work back for manual testing, always run from the `sharpr/` directory:
  - Lints: `cargo clippy -- -D warnings`
  - Formatting: `cargo fmt`
  - Build: `cargo build`
  - Tests: `cargo test` (for behavior changes)
- **Handoff**: After implementation, inform the user exactly what to test manually and how the app should behave.

## Building and Running

**Native Development Path:**
GSettings schemas must be compiled before running natively.
```bash
cd sharpr
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR=data cargo run
```

## Architectural & Engineering Rules

Strictly adhere to these established patterns:

1. **Threading and GTK Objects**:
   - Keep GTK objects strictly on the main thread. Do NOT use `Arc`/`Mutex` on GTK objects.
   - Use `std::thread::spawn` for heavy background processing (like thumbnail decoding).
   - Use `async_channel` to communicate between workers and the main thread.
   - Dispatch UI updates using `glib::MainContext::spawn_local` on the main thread.

2. **State Management**:
   - Use `Rc<RefCell<AppState>>` for state that is only accessed on the main thread.
   - Keep `LibraryManager` store state, path indexes, selected index, and caches in sync.

3. **UI and Layouts**:
   - Keep UI work in the relevant `src/ui/` module when possible. Do not overload `src/ui/window.rs`.
   - Prefer existing GNOME/Libadwaita patterns already present in the app.
   - Use adaptive layouts: prefer `AdwNavigationSplitView` / `AdwOverlaySplitView` and use `AdwBreakpoint`.

4. **GTK Widget Subclassing**:
   - Use `mod imp { ... }` + `glib::wrapper!` + `#[glib::object_subclass]`
   - Keep boilerplate clean and idiomatic to `gtk-rs` conventions.

5. **Application Logic**:
   - Respect disabled folders everywhere: direct folder opens, indexing, smart folders, virtual views, duplicate detection, quality views, collections, metadata, hashes, and tags.

## Directory Structure (Inside `sharpr/`)

- `src/main.rs`: Entry point and rexiv2 initialization.
- `src/app.rs`: The `AdwApplication` subclass.
- `src/ui/window.rs`: Main `AdwApplicationWindow` containing the 3-pane layout and breakpoints.
- `src/ui/sidebar.rs`: Folder tree explorer (`SidebarPane`).
- `src/ui/filmstrip.rs`: `GtkListView` thumbnail strip (`FilmstripPane`).
- `src/ui/viewer.rs`: Full-resolution image preview, zoom, and panning (`ViewerPane`).
- `src/ui/metadata_chip.rs`: Floating EXIF overlay (`MetadataChip`).
- `src/model/`: Core `GObject` models (`ImageEntry`, `FolderNode`, `LibraryManager`).
- `src/thumbnails/`: Background thumbnail decoding worker.
- `src/metadata/`: `rexiv2` EXIF/XMP wrapper.
- `src/upscale/`: NCNN subprocess runner.
- `src/config/`: JSON settings/GSettings.
