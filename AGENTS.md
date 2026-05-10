# Sharpr Agent Instructions

These instructions apply to all AI coding agents working in this repository (Codex, Claude, Gemini, and others). `CLAUDE.md` and `GEMINI.md` add tool-specific depth but defer to this file for shared rules.

## Project Shape

- The Rust application lives in `sharpr/`.
- Sharpr is a GNOME-native image library viewer built with Rust, GTK4, Libadwaita, GSettings, SQLite-backed local indexes, background thumbnail/hash workers, and optional AI features.
- Keep UI work in the relevant `src/ui/` module when possible. Do not add new behavior directly to `src/ui/window/` (a ~5,270-line module directory); extract the relevant chunk first when touching compare mode, collection dialogs, or viewer layout wiring.

## Required Workflow

- Use a new git commit for each separate user task.
- Do not push to GitHub until the user asks for it.
- Before handing work back for manual testing, always run:
  ```bash
  cd sharpr
  cargo build
  ```
- For behavior changes, prefer `cargo nextest run`; use `cargo test` only if nextest is unavailable.
- For dependency or supply-chain changes, run `cargo deny check` and `cargo machete`.
- Before committing or handing work off, consider `./check.sh` from `sharpr/` to run the standard fmt + clippy + nextest + deny + machete sequence.
- After implementation, tell the user exactly what to test manually and how the app should behave.
- If the user reports a bug in an unpushed commit, fix it and amend that task's commit instead of creating noisy follow-up commits.

## Native Run Path

Native runs require compiled schemas:

```bash
cd sharpr
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR=data cargo run
```

`GSETTINGS_SCHEMA_DIR=data` is only required for `cargo run`. It is not needed for `cargo build`, `cargo test`, `cargo nextest run`, or `cargo clippy`.

## Product Constraints

**Non-destructive rule (strict):** Sharpr never modifies original image files. Export, upscale, and format conversion create new output files. The Delete key sends files to the system trash (explicit and intentional). Never add any feature that writes back to source files.

**Do not re-add removed scope:**
- Rotate/flip pixel editing (actions, menu items, and `image_ops.rs` save logic are gone)
- Sharpness backfill worker (`quality/backfill.rs` removed; quality scoring is resolution-only)
- ONNX model downloader (`upscale/downloader.rs` removed; ONNX expects local model files)
- Splash screen (removed; `build.rs` no longer requires it)
- Import workflow, saved searches, batch rename, embedded metadata writing, and full image editor features
- Color management (ICM/lcms2) is deferred/nice-to-have, not a near-term target

**Out of scope:** IPTC/XMP export of Sharpr tags, arbitrary scripting.

**Optional/privacy-sensitive features:** AI tagging (ONNX ResNet, local), ComfyUI upscaling (local server), and any future API-based features must remain opt-in and clearly indicate when an image may leave the machine.

## Structured Logging (`bench.rs`)

Sharpr has a built-in structured logging system in `src/bench.rs`. It is **enabled by default** and writes newline-delimited JSON (JSONL) to `~/.cache/sharpr/logs/run-<timestamp>-<pid>.jsonl` on every launch. Old files are auto-trimmed (default: 20 kept).

Key env vars:
- `SHARPR_BENCH=0` (or `false`/`no`/`off`) — disable logging entirely
- `SHARPR_BENCH_LOG=<path>` — override the output file path
- `SHARPR_BENCH_LOG_LIMIT=<n>` — override the number of old log files to keep

Use the macros `bench_event!`, `bench_warn!`, and `bench_error!` when adding observable events or errors to new code. Do not use `println!`/`eprintln!` for structured output; route it through `bench.rs` instead.

## Engineering Rules

- Keep GTK objects on the main thread. Do not use `Arc`/`Mutex` on GTK objects.
- Use `Rc<RefCell<AppState>>` for main-thread UI state.
- Use background workers plus channels (`async_channel`) for heavy work; drain results back through `glib::MainContext::spawn_local`.
- Protect stale-result flows: viewer, thumbnail worker, virtual-folder loads, compare mode, and task results all use generation counters (`Arc<AtomicU64>`). Preserve this pattern when touching any of those flows.
- Keep `LibraryManager` store state, path indexes, selected index, and caches in sync.
- Respect disabled folders everywhere: direct folder opens, indexing, smart folders, virtual views, duplicate detection, quality views, collections, metadata, hashes, and tags.
- Folders are the source of truth. SQLite/cache state exists for speed, stability, task history, metadata, collections, and curation state — it must not become the source of truth for the user's library.
- Prefer existing GNOME/Libadwaita patterns already present in the app.

## Rust Guidance

**Style:**
- Prefer simple, idiomatic Rust. Avoid cleverness that requires a comment to justify it.
- Use structs, enums, and traits to clarify boundaries and make invalid states harder to represent.
- Use traits as contracts at meaningful architectural boundaries, not as unnecessary abstraction.
- Prefer clear control flow. Use iterator chains when they improve clarity; use `for` loops when they avoid awkward error handling or unnecessary allocation.
- Avoid unnecessary `collect()` when the result is only immediately iterated again.
- Prefer exhaustive `match`/`if let` for stateful logic.

**Design approach:**
- For large new subsystems: propose data types, state transitions, and ownership boundaries before writing implementation code.
- For small bug fixes: keep the change minimal and well-tested.

**Cloning:** Avoid unnecessary `.clone()`, but allow it when it improves correctness, ownership clarity, or GTK/GObject/UI state handling.

**Error handling:**
- The repo currently uses `Result<T, String>` for simple operations and `Box<dyn std::error::Error>` for serialization/export paths. Match this style before introducing new error types.
- `thiserror` is appropriate for new structured domain/library errors where callers need to distinguish failure modes.
- `anyhow` is appropriate for app-level orchestration when typed recovery is not needed.
- Neither `anyhow` nor `thiserror` is currently in `Cargo.toml`; justify adding a dependency before doing so.
- Avoid `unwrap()`/`expect()` in production paths unless the invariant is obvious. When using `expect()`, state the invariant in the message.

**Inlining and data layout:**
- Do not add `#[inline(always)]` by default; use inline hints only for small hot-path functions when justified by measurement.
- Do not rewrite ordinary UI/domain data into SoA layouts without measured need.
- Use borrowed serde data only where lifetime complexity is justified; do not force zero-copy deserialization into simple settings/config code.

**Concurrency:**
- No async runtime (Tokio is not used). Use `std::thread::spawn` + `async_channel` for background work.
- Do not default to Rayon for image/task work; evaluate oversubscription against the existing internal thread pools first.

## Performance Guidance

- Protect filmstrip and thumbnail responsiveness above theoretical micro-optimizations.
- Never block the GTK main thread with decoding, filesystem scans, hashing, model loading, exports, upscaling, or network calls.
- Preserve visible-thumbnail priority over preload work (two-queue system in `thumbnails/worker.rs`).
- Be careful with Rayon, ONNX Runtime, image decoders, parallel tile processing, and ComfyUI/API calls — avoid CPU oversubscription.
- Measure or reason from the actual hot path before adding complexity.
- Prefer small, testable changes over large performance rewrites.
