# Sharpr Project — Gemini Agent Instructions

Shared agent rules (workflow, product constraints, engineering rules, Rust guidance, performance) live in `AGENTS.md`. This file adds Gemini-specific workflow notes and a directory reference for navigation.

## Project Overview

Sharpr is a local-first image curation tool and viewer for GNOME/Linux.

- **Technologies**: Rust (stable), GTK4 (4.12+), Libadwaita (1.5+), GExiv2 (`rexiv2`), SQLite.
- **Architecture**: Three-pane adaptive layout (sidebar / filmstrip / viewer). Background workers for thumbnail decoding, quality scoring, and optional AI features. Local caching; no monolithic database.
- **Project Structure**: Core Rust application in `sharpr/`.

## Development Workflow

- **Commit Strategy**: Use a new git commit for each separate user task. Do not push to GitHub unless explicitly asked.
- **Bug Fixes**: If the user reports a bug in an unpushed commit, fix it and amend that task's commit instead of creating noisy follow-up commits.
- **Quality Checks**: Before handing work back for manual testing, run from the `sharpr/` directory:
  1. Build: `cargo build`
  2. Lints: `cargo clippy -- -D warnings`
  3. Formatting: `cargo fmt --check` (use `cargo fmt` only when actively editing code)
  4. Tests: `cargo nextest run` (preferred; fallback: `cargo test`)
  5. Supply Chain: `cargo deny check` and `cargo machete`
  6. Full sweep (preferred pre-commit): `./check.sh`
- **Handoff**: After implementation, tell the user exactly what to test manually and how the app should behave.

## Building and Running

**Native Development Path** — GSettings schemas must be compiled before running natively:

```bash
cd sharpr
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR=data cargo run
```

`GSETTINGS_SCHEMA_DIR=data` is only required for `cargo run`. Do not prepend it to `cargo build`, `cargo test`, `cargo nextest run`, or `cargo clippy`.

## Product Constraints

**Non-destructive rule (strict):** Sharpr never modifies original image files. Export, upscale, and format conversion create new output files. The Delete key sends files to the system trash (explicit and intentional). Never add any feature that writes back to source files.

**Do not re-add removed scope:**
- Rotate/flip pixel editing (actions, menu items, and `image_ops.rs` save logic are gone)
- Sharpness backfill worker (`quality/backfill.rs` removed; quality scoring is resolution-only)
- ONNX model downloader (`upscale/downloader.rs` removed; ONNX expects local model files)
- Splash screen (removed; `build.rs` no longer requires it)
- Import workflow, saved searches, batch rename, embedded metadata writing, full image editor features, color management (ICM/lcms2)

**Optional/privacy-sensitive features:** AI tagging (ONNX ResNet, local), ComfyUI upscaling (local server), and any future API-based features must remain opt-in and clearly indicate when an image may leave the machine.

## Architectural & Engineering Rules

See `AGENTS.md` for the full rules. Summary:

1. **Threading:** Keep GTK objects on the main thread. Do NOT use `Arc`/`Mutex` on GTK objects. Use `std::thread::spawn` + `async_channel` for background work. Dispatch UI updates via `glib::MainContext::spawn_local`.

2. **State:** Use `Rc<RefCell<AppState>>` for main-thread shared state. Keep `LibraryManager` store state, path indexes, selected index, and caches in sync.

3. **Stale-result protection:** viewer, thumbnail worker, virtual-folder loads, compare mode, and task results all use generation counters (`Arc<AtomicU64>`). Preserve this pattern when touching those flows.

4. **UI:** Keep UI work in `src/ui/` modules. Do not add new behavior directly to `src/ui/window/` (~5,270-line module directory); extract the relevant chunk first when touching compare mode, collection dialogs, or viewer layout wiring. Prefer existing GNOME/Libadwaita patterns. Use `AdwNavigationSplitView`/`AdwOverlaySplitView` and `AdwBreakpoint` for adaptive layouts.

5. **GTK Widget Subclassing:** Use `mod imp { ... }` + `glib::wrapper!` + `#[glib::object_subclass]`. Keep boilerplate clean and idiomatic to `gtk-rs` conventions.

6. **Application Logic:** Respect disabled folders everywhere: direct folder opens, indexing, smart folders, virtual views, duplicate detection, quality views, collections, metadata, hashes, and tags.

## Rust Guidance

- Prefer simple, idiomatic Rust. Avoid cleverness that requires a comment to justify it.
- Use structs, enums, and traits to clarify boundaries and make invalid states harder to represent.
- Prefer clear control flow: iterator chains when they improve clarity, `for` loops when they avoid awkward error handling or unnecessary allocation. Avoid unnecessary `collect()` when the result is only immediately iterated again.
- Prefer exhaustive `match`/`if let` for stateful logic.
- For large new subsystems: propose data types, state transitions, and ownership boundaries before implementation. For small bug fixes: keep the change minimal and well-tested.
- Avoid unnecessary `.clone()`, but allow it when it improves correctness, ownership clarity, or GTK/GObject/UI state handling.
- **Error handling:** The repo uses `Result<T, String>` for simple ops and `Box<dyn std::error::Error>` for serialization/export paths. Neither `anyhow` nor `thiserror` is in `Cargo.toml`; justify adding a dependency before doing so. Avoid `unwrap()`/`expect()` in production paths.
- No async runtime (Tokio not used). Use `std::thread::spawn` + `async_channel`. Do not default to Rayon without evaluating oversubscription against existing internal thread pools.

## Performance Guidance

- Protect filmstrip and thumbnail responsiveness above theoretical micro-optimizations.
- Never block the GTK main thread with decoding, filesystem scans, hashing, model loading, exports, upscaling, or network calls.
- Preserve visible-thumbnail priority over preload work (two-queue system in `thumbnails/worker.rs`).
- Be careful with Rayon, ONNX Runtime, image decoders, parallel tile processing, and ComfyUI/API calls — avoid CPU oversubscription.
- Measure or reason from the actual hot path before adding complexity.

## Directory Structure (Inside `sharpr/`)

- `src/main.rs`: Entry point and rexiv2 initialization.
- `src/app.rs`: `AdwApplication` subclass.
- `src/ui/window/`: Main window module; `AppState`, three-pane layout, action setup (~5,270 lines — do not add new behavior here directly).
- `src/ui/sidebar.rs`: Folder tree explorer (`SidebarPane`).
- `src/ui/filmstrip.rs`: `GtkListView` thumbnail strip (`FilmstripPane`).
- `src/ui/viewer.rs`: Full-resolution image preview, zoom, and panning (`ViewerPane`).
- `src/ui/metadata_chip.rs`: Floating EXIF overlay (`MetadataChip`).
- `src/ui/filter_bar.rs`: Quality-class and tag filter bar.
- `src/ui/ops_indicator.rs`: Floating pill showing background-op progress.
- `src/ui/compare_controller.rs`: Compare mode state — enter/exit/refresh, selection, queue management.
- `src/ui/compare_page.rs`: Before/after comparison page with OSD chip.
- `src/ui/tasks_page.rs`: Tasks dashboard — pipeline/export progress and failures.
- `src/ui/tag_browser.rs`: Tag browser grid/list with `TagCard` tiles.
- `src/ui/tag_card.rs`: Individual tag card widget.
- `src/ui/help_window.rs`: Help window loaded from GResource.
- `src/model/`: Core GObject models (`ImageEntry`, `FolderNode`, `LibraryManager`).
- `src/thumbnails/`: Background thumbnail decoding worker (two-queue: visible priority + preload).
- `src/image_pipeline/`: Shared decode pipeline and preview workers.
- `src/metadata/`: `rexiv2` EXIF/XMP wrapper.
- `src/upscale/`: Multiple upscale backends — CLI/ncnn-vulkan (`backends/cli.rs`), ComfyUI (`backends/comfyui.rs`), ONNX (`backends/onnx.rs`); also comparison viewer (`comparison.rs`) and backend detector (`detector.rs`).
- `src/library_index/`: SQLite-backed persistent index (r2d2 + rusqlite).
- `src/tags/`: Tag database, auto-tagger, ONNX smart tagger.
- `src/quality/`: Quality scoring (resolution-based) and blur detection.
- `src/export/`: Export pipeline (JPEG/JXL/PNG/WebP).
- `src/ops/`: Background op progress (`OpHandle`/`OpEvent`).
- `src/config/`: JSON settings/GSettings.
