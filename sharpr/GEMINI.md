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
Before finalizing any execution, ensure you run the project's quality checks. `GSETTINGS_SCHEMA_DIR=data` is only required for `cargo run` — do not prepend it to build, test, clippy, or other commands.
- **Full Sweep:** `./check.sh` (Standard pre-handoff/pre-commit sweep)
- **Lints:** `cargo clippy -- -D warnings`
- **Formatting:** `cargo fmt`
- **Build (Debug):** `cargo build`
- **Build (Release):** `cargo build --release`
- **Tests:** `cargo nextest run` (Preferred behavior-test path; fallback to `cargo test`)
- **Supply Chain:** `cargo deny check` and `cargo machete` (For dependency/supply-chain work)

## Directory Structure & Responsibilities
- `src/main.rs`: Entry point and rexiv2 initialization.
- `src/app.rs`: The `AdwApplication` subclass.
- `src/ui/window/`: Main window module; `AppState`, three-pane layout, action setup (~5,270 lines — do not add new behavior here directly).
- `src/ui/sidebar.rs`: Folder tree explorer (`SidebarPane`).
- `src/ui/filmstrip.rs`: `GtkListView` thumbnail strip (`FilmstripPane`).
- `src/ui/viewer.rs`: Full-resolution image preview, zoom, and panning (`ViewerPane`).
- `src/ui/compare_controller.rs`: Compare mode state — enter/exit/refresh, selection, queue management.
- `src/ui/compare_page.rs`: Before/after comparison page with expandable OSD chip.
- `src/ui/compare_item.rs`: `CompareItem`, `CompareAssetInfo` — data types for compare entries.
- `src/ui/tasks_page.rs`: Tasks dashboard — pipeline/export progress, failures, generated outputs.
- `src/ui/metadata_chip.rs`: Floating EXIF overlay (`MetadataChip`).
- `src/ui/filter_bar.rs`: Quality-class and tag filter bar.
- `src/ui/ops_indicator.rs`: Floating pill showing background-op progress.
- `src/ui/tag_browser.rs`: Tag browser grid/list with `TagCard` tiles.
- `src/ui/tag_card.rs`: Individual tag card widget.
- `src/ui/help_window.rs`: Help window loaded from GResource.
- `src/model/`: Core `GObject` models (`ImageEntry`, `FolderNode`, `LibraryManager`).
- `src/thumbnails/`: Background thumbnail decoding worker (two-queue: visible priority + preload).
- `src/image_pipeline/`: Shared decode pipeline and preview workers.
- `src/metadata/`: The rexiv2 EXIF/XMP wrapper.
- `src/upscale/`: Multiple upscale backends — CLI/ncnn-vulkan (`backends/cli.rs`), ComfyUI (`backends/comfyui.rs`), ONNX (`backends/onnx.rs`); also comparison viewer (`comparison.rs`) and backend detector (`detector.rs`).
- `src/library_index/`: SQLite-backed persistent index.
- `src/tags/`: Tag database, auto-tagger, ONNX smart tagger.
- `src/quality/`: Quality scoring (resolution-based) and blur detection.
- `src/export/`: Export pipeline (JPEG/JXL/PNG/WebP).
- `src/ops/`: Background op progress (`OpHandle`/`OpEvent`).
- `src/config/`: JSON settings/GSettings.

## Git Workflow & Conventions
- **Task Commits:** When a task is completed, commit the task immediately. After committing, always use `cargo build` so the user can immediately test the application.
- **Bug Fixes:** If a bug is found and fixed related to recent work, amend it to the previous commit rather than creating a new one.
- **Pushing Changes:** Only push when explicitly told to do so by the user.
- **Direct Pushes:** Do not create a Pull Request (PR). Push directly to `main` if the user asks to push.
