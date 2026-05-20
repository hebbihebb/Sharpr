# Sharpr Project — Gemini Agent Instructions

Shared agent rules (workflow, product constraints, engineering rules, Rust guidance, performance) live in `AGENTS.md`. This file adds architecture depth, module details, current implementation state, and Gemini-specific workflow notes.

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

`build.rs` compiles `data/io.github.hebbihebb.Sharpr.gschema.xml` into GSettings and bundles assets via GResource — the schema file must exist for a clean build.

## Master Action Plan

The master plan lives at [MASTER-PLAN.html](file:///home/hebbi/Projects/Sharpr/MASTER-PLAN.html) (project root). Always consult it when starting work, planning tasks, or updating documentation. This file tracks current implementation status, open issues, and the strategic roadmap.

## Session Continuity

On long tasks (any session involving merge resolution, large rewrites, or multi-step plan work): append a checkpoint to `.gemini/RESUME.md` (create it if missing) before any large operation and whenever context usage is high. This ensures state can be recovered if the session context is lost.

## Development Workflow

- **Commit Strategy**: Use a new git commit for each separate user task. Do not push to GitHub unless explicitly asked.
- **Bug Fixes**: If the user reports a bug in an unpushed commit, fix it and amend that task's commit instead of creating noisy follow-up commits.
- **Quality Checks**: Before handing work back for manual testing, run from the `sharpr/` directory:
  1. Build: `cargo build`
  2. Lints: `cargo clippy -- -D warnings`
  3. Formatting: `cargo fmt`
  4. Tests: `cargo nextest run` (preferred; fallback: `cargo test`)
  5. Supply Chain: `cargo deny check` and `cargo machete`
  6. Full sweep (preferred pre-commit): `./check.sh`
- **Handoff**: After implementation, tell the user exactly what to test manually and how the app should behave.

## Structured Logging (`bench.rs`)

Sharpr has a built-in structured logging system in `src/bench.rs`. It is **enabled by default** and writes JSONL to `~/.cache/sharpr/logs/run-<timestamp>-<pid>.jsonl`. 

Key env vars:
- `SHARPR_BENCH=0` — disable logging
- `SHARPR_BENCH_LOG=<path>` — override output file path

Use `bench_event!`, `bench_warn!`, `bench_error!` macros in new code. Do not use `println!`/`eprintln!` for structured output.

## Reference Documents

- [GTK-MANUAL.md](file:///home/hebbi/Projects/Sharpr/GTK-MANUAL.md): GNOME HIG, accessibility (A11y), adaptive layout (1024x600), and interaction patterns (Undo vs. Dialogs). Consult this for all UI work.
*   [MASTER-PLAN.html](file:///home/hebbi/Projects/Sharpr/MASTER-PLAN.html): Strategic roadmap and current implementation status.

## Architecture & Module Map

Sharpr is a GTK4 + Libadwaita image library viewer (~28,500 lines of Rust).

| Module | Role |
|---|---|
| `app.rs` | `SharprApplication` — AdwApplication subclass, about dialog |
| `ui/window/` | Main window module; `AppState`, three-pane layout wiring (~5,270 lines — extract chunks first!) |
| `ui/window/compare_controller.rs` | Compare mode state and queue management |
| `ui/filmstrip.rs` | `GtkListView` thumbnail strip with factory/model binding |
| `ui/viewer.rs` | Full-res display, zoom/pan; uses `glycin::Loader` for async decode |
| `ui/sidebar.rs` | Folder tree navigator using `GtkTreeListModel` |
| `ui/metadata_chip.rs` | Floating EXIF OSD overlay (collapsed = chip, expanded = panel) |
| `ui/filter_bar.rs` | Quality-class and tag filter bar; emits `ActiveFilters` |
| `ui/ops_indicator.rs` | Floating pill button showing background-op progress |
| `ui/tasks_page.rs` | Tasks dashboard — central hub for background work progress |
| `model/library.rs` | `LibraryManager` — O(1) path lookup, LRU thumbnail cache (500 cap) |
| `thumbnails/worker.rs` | Background thumbnail decode; two-queue system (visible + preload) |
| `thumbnails/cache.rs` | Private `~/.cache/sharpr/thumbnails-r1/` LRU cache (non-freedesktop spec) |
| `library_index/` | SQLite-backed persistent index (r2d2 pool) |
| `quality/scorer.rs` | Resolution-based scoring (`Excellent` to `NeedsUpscale`) |
| `quality/blur.rs` | Laplacian-variance sharpness measure on thumbnail buffers |
| `duplicates/phash.rs` | dHash-based duplicate detection with Hamming distance grouping |

## Data Flow

1. **Folder Open**: User selects folder → `LibraryManager` scans paths → `library_index` persists metadata → `GListModel` populates.
2. **Thumbnail Generation**: `FilmstripPane` requests thumb → `ThumbnailWorker` dispatches (visible priority) → `quality/blur.rs` scores sharpness → `Texture` sent back to UI.
3. **Image View**: Select image → `ViewerPane` loads via `glycin` → `MetadataWorker` reads EXIF concurrently.
4. **Curation**: User tags/rates → `TagDatabase` (SQLite) updates → UI reflects changes via GObject property bindings.
5. **Background Ops**: Exports/Upscales emit `OpEvent` → `ops_indicator` and `tasks_page` reflect progress.

## Engineering Rules (Summary)

1. **Threading**: GTK objects stay on the main thread. No `Arc`/`Mutex` on widgets.
2. **Concurrency**: `std::thread::spawn` + `async_channel` + `glib::MainContext::spawn_local`.
3. **Stale Protection**: Use generation counters (`Arc<AtomicU64>`) for all async result flows.
4. **UI**: Prefer GNOME HIG and standard widgets. Every control must have an accessible name.
5. **Non-destructive**: Never modify source images. Exports/upscales create new files.
N settings/GSettings.
