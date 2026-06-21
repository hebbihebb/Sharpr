# Sharpr Agent Instructions

These instructions are the source of truth for AI coding agents working in this repository. Codex is the primary orchestrator now. Claude, Gemini, Antigravity, and other agents should treat their local files as thin entrypoints that defer here.

## Read First

1. `AGENTS.md` - workflow, constraints, and engineering rules.
2. `CURRENT_TASKS.md` - current project state, next task, and active handoff notes.
3. `ROADMAP.md` - future work and implementation order.
4. `docs/reference/architecture.md` - module map and runtime patterns for non-trivial changes.
5. `docs/reference/ui-gtk.md` - Sharpr-specific GTK/Libadwaita guidance for UI work.

`README.md` is GitHub-facing and should not be used as the project memory. Historical context lives under `docs/archive/`.

## Project Shape

- The Rust application lives in `sharpr/`.
- Sharpr is a GNOME-native, local-first image curation tool built with Rust, GTK4, Libadwaita, GSettings, SQLite-backed local indexes, background thumbnail/hash workers, and optional AI features.
- The core workflow is fast folder review: open a folder, move through the filmstrip, decide what to keep or trash, compare similar images, tag and collect keepers, and route export/upscale/format work through Tasks.
- Folders are the source of truth. SQLite/cache state exists for speed, stability, task history, metadata, collections, and curation state. Do not make SQLite the only truth for a user's library.

## Required Workflow

- Use a new git commit for each separate user task.
- Do not push to GitHub until the user explicitly asks.
- If the user reports a bug in an unpushed task commit, fix it and amend that commit instead of adding a noisy follow-up commit.
- Before handing work back for manual testing, always run:
  ```bash
  cd sharpr
  cargo build
  ```
- For behavior changes, prefer `cargo nextest run`; use `cargo test` only if nextest is unavailable.
- For dependency or supply-chain changes, run `cargo deny check` and `cargo machete`.
- Before committing or handing off larger work, consider `./check.sh` from `sharpr/` to run the standard fmt + clippy + nextest + deny + machete sequence.
- After implementation, tell the user exactly what to test manually and how the app should behave.

## Native Run Path

Native runs require compiled schemas:

```bash
cd sharpr
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR=data cargo run
```

`GSETTINGS_SCHEMA_DIR=data` is only required for `cargo run`. It is not needed for `cargo build`, `cargo test`, `cargo nextest run`, or `cargo clippy`.

## Product Constraints

**Non-destructive rule (strict):** Sharpr never modifies original image files. Export, upscale, format conversion, and future version workflows create new output files or explicitly managed copies. The Delete key sends files to the system trash only as an explicit user action. Never add a feature that silently writes back to source files.

**Do not re-add removed scope:**

- Rotate/flip pixel editing, actions, menu items, or `image_ops.rs` save logic
- Sharpness backfill worker (`quality/backfill.rs` removed; quality scoring is resolution-only)
- ONNX model downloader (`upscale/downloader.rs` removed; ONNX expects local model files)
- Splash screen
- Import workflow, saved searches, batch rename, embedded metadata writing, full image editor features, arbitrary scripting
- Color management as a near-term project unless specifically requested

**Optional/privacy-sensitive features:** AI tagging, ComfyUI upscaling, and any future API-based features must remain opt-in and clearly indicate when an image may leave the machine.

## Research Before Reinventing

Big wins in this project have come from replacing custom work with existing GTK, GNOME, OS, or crate-level facilities. Before implementing new infrastructure, check whether the platform already provides the behavior.

- Prefer standard GTK/Libadwaita widgets, actions, menus, dialogs, portals, and accessibility behavior over custom UI.
- Prefer proven Rust crates or system libraries for image formats, file operations, model/runtime integration, and desktop integration when they fit the constraints.
- Do not add a dependency casually. Justify it against existing code, packaging, Flatpak impact, and maintenance cost.
- Keep custom code for Sharpr-specific workflow logic, performance scheduling, or domain constraints that general libraries do not cover.

## Structured Logging

Sharpr has built-in structured logging in `src/bench.rs`. It is enabled by default and writes JSONL to `~/.cache/sharpr/logs/run-<timestamp>-<pid>.jsonl`. Old files are auto-trimmed.

Key env vars:

- `SHARPR_BENCH=0` disables logging
- `SHARPR_BENCH_LOG=<path>` overrides the output file
- `SHARPR_BENCH_LOG_LIMIT=<n>` overrides retention count

Use `bench_event!`, `bench_warn!`, and `bench_error!` for observable events or errors in new code. Do not use `println!` or `eprintln!` for structured output.

## Engineering Rules

- Keep GTK objects on the main thread. Do not use `Arc`/`Mutex` on GTK objects.
- Use `Rc<RefCell<AppState>>` for main-thread UI state.
- Use background workers plus channels (`async_channel`) for heavy work; drain results back through `glib::MainContext::spawn_local`.
- Protect stale-result flows with generation counters. Viewer, thumbnail worker, virtual-folder loads, compare mode, and task results all rely on this discipline.
- Keep `LibraryManager` store state, path indexes, selected index, and caches in sync.
- Respect disabled folders everywhere: direct folder opens, indexing, smart folders, virtual views, duplicate detection, quality views, collections, metadata, hashes, tags, and future version storage.
- Protect filmstrip and thumbnail responsiveness above theoretical micro-optimizations.
- Never block the GTK main thread with decoding, filesystem scans, hashing, model loading, exports, upscaling, or network calls.
- Preserve visible-thumbnail priority over preload work in `thumbnails/worker.rs`.

## Rust Guidance

- Prefer simple, idiomatic Rust. Avoid cleverness that requires a comment to justify it.
- Use structs, enums, and traits to clarify boundaries and make invalid states harder to represent.
- Use traits as contracts at meaningful architectural boundaries, not as unnecessary abstraction.
- Prefer clear control flow. Use iterator chains when they improve clarity; use `for` loops when they avoid awkward error handling or unnecessary allocation.
- Avoid unnecessary `collect()` when the result is only immediately iterated again.
- Prefer exhaustive `match` and explicit `if let` for stateful logic.
- Avoid unnecessary `.clone()`, but allow it when it improves correctness, ownership clarity, or GTK/GObject/UI state handling.
- Match the repo's current error style before introducing new error types: `Result<T, String>` for simple operations and `Box<dyn std::error::Error>` for serialization/export paths.
- `thiserror` is appropriate for new structured domain/library errors where callers need to distinguish failure modes. `anyhow` is appropriate for app-level orchestration when typed recovery is not needed. Neither is currently a default dependency.
- Avoid `unwrap()`/`expect()` in production paths unless the invariant is obvious. When using `expect()`, state the invariant in the message.
- Do not add `#[inline(always)]` by default or rewrite ordinary UI/domain data into specialized layouts without measured need.
- No async runtime is used. Do not add Tokio by default. Use `std::thread::spawn` plus `async_channel`.
- Do not default to Rayon for image/task work; evaluate oversubscription against existing worker pools first.

## Documentation Updates

- Update `CURRENT_TASKS.md` when the active task, next task, or handoff state changes.
- Update `ROADMAP.md` when priorities or planned feature scope changes.
- Move completed implementation notes to `docs/archive/completed.md`.
- Add historical summaries to `docs/archive/history.md` only when they are useful for future context.
- Keep agent-specific files short and deferential. Do not mirror large architecture or workflow sections into tool-specific docs.
