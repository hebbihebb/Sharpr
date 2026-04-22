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
Before finalizing any execution, ensure you run the project's quality checks:
- Lints: `cargo clippy -- -D warnings`
- Formatting: `cargo fmt`
- Build (Debug): `cargo build`
- Build (Release): `cargo build --release`

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
