# Sharpr Agent Instructions

These instructions apply to AI coding agents working in this repository.

## Project Shape

- The Rust application lives in `sharpr/`.
- Sharpr is a GNOME-native image library viewer built with Rust, GTK4, Libadwaita, GSettings, SQLite-backed local indexes, background thumbnail/hash workers, and optional AI features.
- Keep UI work in the relevant `src/ui/` module when possible. Do not turn `src/ui/window.rs` into the default place for every new behavior.

## Required Workflow

- Use a new git commit for each separate user task.
- Do not push to GitHub until the user asks for it.
- Before handing work back for manual testing, always run:
  ```bash
  cd sharpr
  cargo build
  ```
- For code changes, also run focused checks appropriate to the change. Prefer `cargo test` for behavior changes.
- After implementation, tell the user exactly what to test manually and how the app should behave.
- If the user reports a bug in an unpushed commit, fix it and amend that task's commit instead of creating noisy follow-up commits.

## Native Run Path

Native runs require compiled schemas:

```bash
cd sharpr
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR=data cargo run
```

## Engineering Rules

- Keep GTK objects on the main thread.
- Use `Rc<RefCell<AppState>>` for main-thread UI state.
- Use background workers plus channels for heavy work; drain results back through `glib::MainContext::spawn_local`.
- Keep `LibraryManager` store state, path indexes, selected index, and caches in sync.
- Respect disabled folders everywhere: direct folder opens, indexing, smart folders, virtual views, duplicate detection, quality views, collections, metadata, hashes, and tags.
- Prefer existing GNOME/Libadwaita patterns already present in the app.
