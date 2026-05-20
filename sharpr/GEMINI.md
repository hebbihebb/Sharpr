# Sharpr Development Guidelines (Gemini)

This file serves as the local set of instructions for Gemini CLI while working on the Sharpr project. Shared agent rules live in `../AGENTS.md`.

## Build & Run (Native)

Native development requires compiled GSettings schemas.

```bash
# Compile schemas (from sharpr/ directory)
glib-compile-schemas data/

# Run with local schema
GSETTINGS_SCHEMA_DIR=data cargo run
```

`GSETTINGS_SCHEMA_DIR=data` is only required for `cargo run`.

## Architectural Patterns

1. **GTK Widget Subclassing**:
   - Use `mod imp { ... }` + `glib::wrapper!` + `#[glib::object_subclass]`
   - Use `glib::clone!` with `#[weak]` captures for signal handlers to avoid reference cycles.
2. **Concurrency**:
   - GTK objects stay on the main thread. No `Arc`/`Mutex` on widgets.
   - Heavy work (decoding, hashing) uses `std::thread::spawn` + `async_channel`.
   - Results drain back via `glib::MainContext::spawn_local`.
3. **Stale Result Protection**:
   - Always preserve the **Generation Counter** pattern (`Arc<AtomicU64>`) for async flows (viewer, thumbnails, etc.) to prevent stale results from overwriting new ones.

## Quality & Verification

Run these before handoff or commit:
- **Full Sweep:** `./check.sh` (Runs fmt, clippy, nextest, deny, machete)
- **Lints:** `cargo clippy -- -D warnings`
- **Tests:** `cargo nextest run` (Preferred for better output/isolation)
- **Supply Chain:** `cargo deny check` and `cargo machete` (For dependency changes)

## Module Map

| Module | Responsibility |
|---|---|
| `ui/window/` | Main shell wiring. **Extract logic first** before adding new features here. |
| `ui/viewer.rs` | Full-res display using `glycin::Loader`. |
| `ui/filmstrip.rs` | `GtkListView` thumbnails. |
| `ui/tasks_page.rs` | Dashboard for background ops (Exports, Upscales). |
| `thumbnails/` | Worker with two-queue system (Visible Priority vs Preload). |
| `model/library.rs` | Path indexing and LRU cache management. |
| `library_index/` | Persistent SQLite storage. |

## Product Constraints

- **Non-Destructive**: Never modify source images.
- **Trash**: Delete key = System Trash.
- **Removed Scope**: Do not re-add rotate/flip editing, sharpness backfill, or ONNX downloader.

## Git Workflow

- **Task Commits**: One commit per user task.
- **Amending**: If a bug is found in an unpushed commit, use `git commit --amend`.
- **Handoff**: Always run `cargo build` before declaring a task done.
