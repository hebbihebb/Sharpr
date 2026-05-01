# Sharpr Development Guidelines

This file serves as the core set of instructions and mandates for Gemini CLI while working on the Sharpr project.

## Project Overview
Sharpr is a modern GTK4 + Libadwaita image library and viewer built with Rust.

## Core Technologies
- **Rust** (1.75+ stable)
- **GTK4** (4.12+) and **Libadwaita** (1.5+)
- **GExiv2** (0.14+) for EXIF/XMP/IPTC metadata (`rexiv2` wrapper)
- **async-channel** for main-thread to background-thread communication

## Architectural Patterns
Strictly adhere to the following established patterns when adding or modifying code:

1. **GTK Widget Subclassing**: 
   - Use `mod imp { ... }` + `glib::wrapper!` + `#[glib::object_subclass]`
   - Keep boilerplate clean and idiomatic to `gtk-rs` conventions.
2. **Background Work & Concurrency**: 
   - Do NOT use `Arc`/`Mutex` on GTK objects. GTK objects are strictly for the main thread.
   - Use `std::thread::spawn` for heavy background processing (like thumbnail decoding).
   - Use `async_channel` to communicate between workers and the main thread.
   - Dispatch UI updates using `glib::MainContext::spawn_local` on the main thread.
3. **Shared State**: 
   - Use `Rc<RefCell<AppState>>` for state that is only accessed on the main thread.
4. **Adaptive UI Layouts**: 
   - Prefer `AdwNavigationSplitView` / `AdwOverlaySplitView`.
   - Use `AdwBreakpoint` for adaptive, responsive changes.

## Code Quality & Validation
Before finalizing any execution, ensure you run the project's quality checks. Note that cargo commands must be run with the GNOME/GSettings runtime environment (specifically `GSETTINGS_SCHEMA_DIR=data`):
- Lints: `GSETTINGS_SCHEMA_DIR=data cargo clippy -- -D warnings`
- Formatting: `cargo fmt`
- Build (Debug): `GSETTINGS_SCHEMA_DIR=data cargo build`
- Build (Release): `GSETTINGS_SCHEMA_DIR=data cargo build --release`
- Tests: `GSETTINGS_SCHEMA_DIR=data cargo test`

## Directory Structure & Responsibilities
- `src/main.rs`: Entry point and rexiv2 initialization.
- `src/app.rs`: The `AdwApplication` subclass.
- `src/ui/window.rs`: Main `AdwApplicationWindow` containing the 3-pane layout and breakpoints.
- `src/ui/sidebar.rs`: Folder tree explorer (`SidebarPane`).
- `src/ui/filmstrip.rs`: `GtkListView` thumbnail strip (`FilmstripPane`).
- `src/ui/viewer.rs`: Full-resolution image preview, zoom, and panning (`ViewerPane`).
- `src/ui/metadata_chip.rs`: Floating EXIF overlay (`MetadataChip`).
- `src/model/`: Core `GObject` models including `ImageEntry`, `FolderNode`, and `LibraryManager`.
- `src/thumbnails/`: The background thumbnail decoding worker.
- `src/metadata/`: The rexiv2 EXIF/XMP wrapper.
- `src/upscale/`: NCNN subprocess runner.
- `src/config/`: JSON settings/GSettings.

## Git Workflow & Conventions
- **Task Commits:** When a task is completed, commit the task immediately. After committing, always use `cargo build` so the user can immediately test the application.
- **Bug Fixes:** If a bug is found and fixed related to recent work, amend it to the previous commit rather than creating a new one.
- **Pushing Changes:** Only push when explicitly told to do so by the user.
- **Pre-Push Checks:** Before pushing, review git status and ensure the working directory is clean and the branch is properly merged.
- **Direct Pushes:** Do not create a Pull Request (PR). Push directly to `main` unless actively working on a different branch.
