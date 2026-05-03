# Post-Glycin Cleanup Plan

## Objective
Finalize the `glycin` integration by surgically removing dead code, obsolete tests, and correcting outdated documentation that still refers to the legacy image loading and prefetching pipeline.

## Key Files & Context
- `sharpr/src/ui/viewer.rs`: Primary site for outdated comments, dead caching logic, and obsolete tests.
- `sharpr/src/jxl.rs`: Contains unused public JXL helper functions.
- `sharpr/src/export/mod.rs`: Contains an unused `ExportResult` struct.
- `sharpr/src/ops/queue.rs`: Contains an unused `OpEvent` variant.

## Proposed Changes

### 1. `src/ui/viewer.rs` Cleanup
- **Remove Dead Code:** Delete the `can_use_cached_viewer_image` function.
- **Prune Obsolete Tests:** Delete the `viewer_cache_skips_jxl_buffers` test case.
- **Update Documentation:**
    - Rewrite `load_image` docstring to reflect the `glycin` + `spawn_local` implementation.
    - Update `current_rgba` docstring to clarify it is used only for pending in-memory edits.
    - Remove or update all comments referring to "preview cache", "LRU cache", or "prefetching" in the viewer.
- **Refactor `apply_transform`:** Ensure the fallback logic correctly handles the absence of the "preview cache" by relying solely on the active `GtkPicture` texture download.

### 2. `src/jxl.rs` Cleanup
- **Prune Dead Exports:** Remove `pub fn preview_info` and `pub fn decode_embedded_preview` as they are no longer used by the application logic. (Keep private helpers and internal tests if they provide value to the remaining `decode_preview_or_full` logic).

### 3. `src/export/mod.rs` Cleanup
- **Remove Dead Struct:** Delete the `ExportResult` struct and its `#[allow(dead_code)]` attribute.

### 4. `src/ops/queue.rs` Cleanup
- **Remove Dead Variant:** Delete the `OpEvent::Dismissed` variant.

## Verification & Testing
- **Lints:** Run `GSETTINGS_SCHEMA_DIR=data cargo clippy -- -D warnings` to ensure no new dead code warnings were introduced and that all intended removals were successful.
- **Build:** Run `GSETTINGS_SCHEMA_DIR=data cargo build` to verify compilation.
- **Unit Tests:** Run `GSETTINGS_SCHEMA_DIR=data cargo test` to ensure the remaining tests in `viewer.rs`, `jxl.rs`, and `export` still pass.
- **Manual Verification:**
    - Launch the app and verify image loading still works (uses `glycin`).
    - Verify in-memory transformations (rotate/flip) still work (uses the `current_rgba` edit buffer).

## Migration & Rollback
Since this is a cleanup of already-unused code, the risk is minimal. Rollback can be achieved by reverting the cleanup commit.
