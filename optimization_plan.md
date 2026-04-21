# Sharpr Optimization and Refactoring Plan

This document outlines a step-by-step execution plan based on the findings from the `dev research gemini Apr 21.md` audit. These tasks are structured to be executed one by one by a coding agent. They are isolated, clear, and designed to minimize regressions while improving the performance, safety, and architecture of the application.

## Phase 1: High-Priority Safety & Quick Wins

### Task 1.1: Secure the ONNX Model Download
*   **Target File:** `sharpr/src/ui/window.rs`
*   **Objective:** Implement hash verification for the downloaded ONNX model to prevent MITM/RCE attacks.
*   **Execution Steps:**
    1.  Locate `maybe_download_model` in `sharpr/src/ui/window.rs` (around line 292).
    2.  Hardcode the expected SHA-256 hash of the `resnet18-v1-7.onnx` model as a constant.
    3.  After downloading the file to the `.tmp` path, read the file bytes and compute its SHA-256 hash using a crate like `sha2` (add `sha2 = "0.10"` to `Cargo.toml` if not present).
    4.  If the hash matches the hardcoded constant, proceed with `std::fs::rename` to move it to the final model path.
    5.  If the hash fails, delete the `.tmp` file and return an error or log a warning, aborting the smart tagger initialization.
*   **Implementation Comment:** Completed. The download now uses the current Hugging Face ONNX Model Zoo mirror, verifies the temp file against the expected SHA-256 before installing it, removes failed temp downloads, logs the failure reason, and declares `sha2` as a direct dependency.

### Task 1.2: Upgrade Cargo Release Profile & Hashing Dependency
*   **Target Files:** `sharpr/Cargo.toml`, `sharpr/src/model/library.rs`
*   **Objective:** Improve compiler optimizations and data structure lookup speeds.
*   **Execution Steps:**
    1.  In `sharpr/Cargo.toml`, change `lto = "thin"` to `lto = "fat"` (or `lto = true`) under `[profile.release]`.
    2.  Add `rustc-hash = "1.1"` to the `[dependencies]` section.
    3.  In `sharpr/src/model/library.rs`, replace imports of `std::collections::HashMap` with `rustc_hash::FxHashMap`.
    4.  Update the type definitions for `folder_history`, `path_to_index`, `hash_store`, `active_thumbnail_cache`, `thumbnail_cache`, `prefetch_cache`, `preview_cache`, and `metadata_cache` to use `FxHashMap`.
    5.  Ensure any `HashMap::new()` calls are updated to `FxHashMap::default()`.
*   **Implementation Comment:** Completed. Release LTO is now fat, `rustc-hash` is a direct dependency, the specified `LibraryManager` maps and related quality-scan metadata cache flow now use `FxHashMap`, and the crate builds successfully.

### Task 1.3: Eliminate Static String Allocations in Indexing
*   **Target File:** `sharpr/src/library_index/mod.rs`
*   **Objective:** Prevent unnecessary heap allocations in hot paths by returning `&'static str` instead of `String`.
*   **Execution Steps:**
    1.  Locate the match statement around lines 321-326 in `sharpr/src/library_index/mod.rs`.
    2.  Refactor the logic that returns `("missing".into(), "stale".into())` and `("missing".into(), "missing".into())` to return static string slices (`&'static str` or just `"missing"`, `"stale"`) instead of converting them into owned `String`s via `.into()`.
    3.  Update the surrounding function signatures and struct fields if necessary to accept/return `&'static str` or an `Enum` representing these states.
*   **Implementation Comment:** Completed. `upsert_image_basic` now uses `Cow<'static, str>` so database-read statuses remain owned only when needed, while fallback `"missing"` and reset `"stale"` statuses are borrowed static strings.

## Phase 2: Concurrency & Threading Improvements

### Task 2.1: Implement Connection Pooling for SQLite
*   **Target File:** `sharpr/src/library_index/mod.rs` (and `Cargo.toml`)
*   **Objective:** Remove the synchronous `Mutex<Connection>` bottleneck that blocks the GTK UI thread.
*   **Execution Steps:**
    1.  Add `r2d2` and `r2d2_sqlite` to `Cargo.toml`.
    2.  In `sharpr/src/library_index/mod.rs`, replace the `conn: Mutex<Connection>` field in the `LibraryIndex` struct with `pool: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>`.
    3.  Update the initialization logic to create a connection pool instead of a single connection.
    4.  Refactor all methods in `LibraryIndex` that previously locked the mutex (e.g., `let conn = self.conn.lock().unwrap();`) to acquire a connection from the pool (`let conn = self.pool.get().unwrap();`).
*   **Implementation Comment:** Completed. `LibraryIndex` now owns an `r2d2` SQLite pool with per-connection pragma setup, shared schema initialization, pooled access in all query/update paths, and a single-connection in-memory pool for tests.

### Task 2.2: Mitigate Unbounded Thread Spawning (ThreadPool)
*   **Target File:** `sharpr/src/ui/window.rs` (and potentially `Cargo.toml`)
*   **Objective:** Prevent thread exhaustion by replacing bare `std::thread::spawn` with a bounded thread pool for CPU/IO background tasks.
*   **Execution Steps:**
    1.  Add `rayon` to `Cargo.toml` as a lightweight thread pool alternative, OR use `glib::ThreadPool`. Given this is a GTK app, `glib::ThreadPool` might be preferable, but `rayon::spawn` is also a good generic replacement for `std::thread::spawn` for CPU tasks. Let's use `rayon` for simplicity in replacing bare spawns.
    2.  Locate instances of `std::thread::spawn(move || { ... })` in `sharpr/src/ui/window.rs` (e.g., lines 185, 227, 285, 549, 955, 1120).
    3.  Replace these with `rayon::spawn(move || { ... })`.
    4.  *Note: Ensure that any long-running blocking I/O tasks are either handled carefully so they don't exhaust the Rayon pool, or keep a dedicated I/O thread, but avoid unbounded spawning.*
*   **Implementation Comment:** Completed. `rayon` is now a direct dependency and all short-lived `std::thread::spawn` calls in `sharpr/src/ui/window.rs` have been moved to the bounded Rayon pool.

### Task 2.3: Remove Path Cloning in Sidebar Loops
*   **Target File:** `sharpr/src/ui/sidebar.rs`
*   **Objective:** Optimize directory traversal by avoiding $O(N)$ cloning of `PathBuf`.
*   **Execution Steps:**
    1.  Locate the loop around line 394 in `sharpr/src/ui/sidebar.rs`.
    2.  The current logic does: `if directory_contains_images(&root_path) && seen.insert(root_path.clone()) { let row = FolderRow::new(root_path.clone(), &root_name); }`
    3.  Refactor this to minimize clones. Since `seen` needs ownership and `FolderRow` needs ownership, you can do:
        ```rust
        if directory_contains_images(&root_path) && !seen.contains(&root_path) {
            seen.insert(root_path.clone());
            let row = FolderRow::new(root_path, &root_name); // move root_path here
            // ...
        }
        ```
    4.  Apply this optimization to any similar directory traversal loops in `sidebar.rs`.
*   **Implementation Comment:** Completed. Sidebar folder population now checks `seen` before cloning for insertion and moves owned paths into `FolderRow` where possible, covering both root and discovered child folder rows.

## Phase 3: Architectural Decoupling & UI Performance

### Task 3.1: Asynchronous Virtual View Loading
*   **Target File:** `sharpr/src/model/library.rs`
*   **Objective:** Prevent UI freezes caused by synchronous `std::fs::metadata` calls during `load_virtual`.
*   **Execution Steps:**
    1.  Locate the `load_virtual` method in `LibraryManager`.
    2.  Currently, `load_virtual` iterates over paths and synchronously reads metadata for each.
    3.  Refactor `load_virtual` to quickly populate the GTK `ListStore` with "placeholder" entries (containing just the path).
    4.  Offload the metadata extraction (`std::fs::metadata` and `image::image_dimensions`) to a background thread (or Rayon pool).
    5.  As metadata is extracted in the background, send the results back to the main thread via an `async-channel` or `glib::idle_add` to update the placeholder entries in the `ListStore`.
*   **Implementation Comment:** Completed. Virtual views now populate placeholder entries synchronously using only cached metadata and thumbnails, return uncached paths for background hydration, and `window.rs` computes missing file metadata on Rayon before applying it back on the GTK main thread.

### Task 3.2: Optimize GTK Label Formatting
*   **Target Files:** `sharpr/src/ui/sidebar.rs`, `sharpr/src/ui/tag_browser.rs`
*   **Objective:** Eliminate heap allocations when setting text on GTK Labels.
*   **Execution Steps:**
    1.  Locate label creation in `sidebar.rs` (L1023): `let count_label = gtk4::Label::new(Some(&item_count.to_string()));`
    2.  Replace this with format macros or `glib::GString` if supported, or use a stack-allocated buffer (like `itoa` or `ryu` for numbers) if possible. If GTK requires a string, use `format!("{}", item_count)` instead of `.to_string()` for clarity, though it still allocates.
    3.  A better fix for GTK is to use the `label.set_text(&format!("{}", item_count))` or `label.set_markup(...)` and reuse label widgets instead of destroying and recreating them during updates.
*   **Implementation Comment:** Completed. Numeric collection counts and tag counts now use `itoa` stack formatting, and tag chips compose separate labels for tag text and count instead of allocating a combined formatted button label.

### Task 3.3: Refactor Smart Tagger Dynamic Dispatch
*   **Target File:** `sharpr/src/ui/window.rs`
*   **Objective:** Remove unnecessary dynamic dispatch (`Box<dyn SmartTagger>`) if only one implementation exists.
*   **Execution Steps:**
    1.  Check `sharpr/src/tags/smart.rs` to see if there are multiple implementations of `SmartTagger`.
    2.  If only the ONNX tagger exists, refactor `sharpr/src/ui/window.rs` to hold a concrete type: `Option<Arc<OnnxSmartTagger>>` instead of `Option<Arc<dyn SmartTagger + Send + Sync>>`.
    3.  Update the struct definition of `SharprWindow` and the initialization logic to use the concrete type, allowing the compiler to inline function calls to the tagger.
*   **Implementation Comment:** Completed. `LocalTagger` is the only smart tagger implementation, so the trait object was removed and app state now stores `Option<Arc<LocalTagger>>` directly.
