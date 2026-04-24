# Sharpr Audit Task List

This document breaks the architecture audit in `ui-ux-review.md` into one-PR implementation tasks for Codex in the cloud. Each task is intended to be completed as one focused commit and one pull request.

The tasks are ordered by dependency and risk. Do them in order unless a task explicitly says it can be done independently.

## Global Instructions for Every Task

- Work in the Rust crate under `sharpr/`.
- Follow `AGENTS.md`: one commit per task, do not push unless explicitly asked, and run `cd sharpr && cargo build` before handing off.
- Keep GTK objects on the main thread. Use background workers plus channels for decode, filesystem, export, hash, metadata, and AI work.
- Prefer existing patterns:
  - `Rc<RefCell<AppState>>` for main-thread state.
  - `async_channel` for worker-to-main-thread communication.
  - `glib::MainContext::spawn_local` for draining results onto the GTK main thread.
  - generation counters and stale-result discard, as used by thumbnail workers.
  - `crate::bench_event!` for user-visible latency paths.
- Do not introduce Tokio or another async runtime.
- Do not refactor unrelated UI or product surfaces inside implementation PRs.
- If a task creates a new persisted preference, update both `data/io.github.hebbihebb.Sharpr.gschema.xml` and `src/config/settings.rs`.
- If a task changes behavior, add focused tests where possible and run `cargo test` in addition to `cargo build`.

## Milestone 1: Stabilize the Viewer Pipeline

### Task 1: Extract a Shared Preview Decode Module

**Priority:** Critical

**Goal:** Make direct viewer loading, prefetch, and future comparison/export previews use one decode path.

**Problem:** The current viewer slow path in `src/ui/viewer.rs` uses `decode_image_rgba`, which handles embedded previews, turbojpeg scaled JPEG decode, and EXIF orientation. The prefetch path in `src/ui/window.rs` uses `prefetch_decode`, which calls `image::decode().into_rgba8()` directly. That means a prefetched image can display at a different orientation or resolution than a non-prefetched image and can poison `LibraryManager::preview_cache`.

**Primary files:**

- Add `src/image_pipeline/mod.rs` or `src/image_pipeline/preview.rs`.
- Update `src/main.rs` to include the new module.
- Update `src/ui/viewer.rs`.
- Update `src/ui/window.rs`.

**Implementation outline:**

1. Create a non-UI preview decode module that exposes a function similar to:
   - `decode_preview(path: &Path, mode: PreviewDecodeMode) -> Result<PreviewImage, PreviewDecodeError>`
   - `PreviewImage { rgba: Vec<u8>, width: u32, height: u32, source: PreviewSource }`
2. Move the existing `decode_image_rgba`, `is_jpeg_path`, `decode_jpeg_rgba_scaled`, and `choose_jpeg_scale_factor` logic out of `viewer.rs` into the new module.
3. Preserve current behavior:
   - try EXIF embedded JPEG preview first when large enough;
   - use turbojpeg scaled decode for JPEGs when possible;
   - fall back to `image::ImageReader`;
   - apply EXIF orientation in every path.
4. Replace `ViewerPane::load_image` slow-path decode with the new module.
5. Replace `prefetch_decode` in `window.rs` with the same new module.
6. Keep the public API free of GTK types.
7. Add `bench_event!` source labels for embedded preview, scaled JPEG, and full decode.

**Acceptance criteria:**

- `prefetch_decode` no longer contains independent image decoding logic.
- `viewer.rs` no longer owns low-level JPEG scaling or image crate fallback code.
- Prefetched images and directly loaded images use the same orientation behavior.
- The viewer still displays JPEGs, PNGs, and non-JPEG fallback formats.

**Verification:**

- Add unit tests for `choose_jpeg_scale_factor` in the new module if it remains pure.
- Run `cd sharpr && cargo test`.
- Run `cd sharpr && cargo build`.
- Manual test: open a JPEG with EXIF orientation, navigate away and back so it can be prefetched, and confirm orientation is identical in both paths.

### Task 2: Add a Bounded Preview Worker

**Priority:** Critical

**Goal:** Replace per-image viewer decode threads with a small bounded preview worker pipeline.

**Problem:** `ViewerPane::load_image` spawns one OS thread for decode and one for metadata on each slow-path image load. Rapid arrow-key navigation can produce stale decode work, CPU contention, and memory spikes. Thumbnail workers already have a better pattern in `src/thumbnails/worker.rs`; preview decode needs a similar bounded model.

**Primary files:**

- Add `src/image_pipeline/worker.rs` or similar.
- Update `src/image_pipeline/mod.rs`.
- Update `src/ui/window.rs` or `src/ui/viewer.rs` ownership wiring.
- Update `src/ui/viewer.rs`.

**Implementation outline:**

1. Add a `PreviewWorker` with:
   - a bounded or small queue for visible requests;
   - worker count defaulting to 1 or 2;
   - generation-aware `PreviewRequest { path, gen, request_kind }`;
   - `PreviewResult { path, gen, image: Result<PreviewImage, PreviewDecodeError> }`.
2. Store the worker handle somewhere with window lifetime, likely in `SharprWindow::imp`, similar to `ThumbnailWorker`.
3. Have `ViewerPane::load_image` send decode requests instead of spawning a new thread.
4. Drain results on the GTK main thread with `glib::MainContext::spawn_local`.
5. Discard stale results by comparing the captured generation with `ViewerPane::load_gen`.
6. Preserve spinner and current-image clearing behavior initially; UI refinements can happen in a later task.
7. Keep metadata loading as-is in this PR if folding it in makes the task too large.

**Acceptance criteria:**

- Slow-path image decode no longer calls `std::thread::spawn` from `ViewerPane::load_image`.
- Preview decode worker count is bounded.
- Stale decode results do not update the picture.
- Existing thumbnail worker behavior is unchanged.

**Verification:**

- Run `cd sharpr && cargo test` if worker tests are added.
- Run `cd sharpr && cargo build`.
- Manual test: hold Right Arrow through a folder of large images; the app should remain responsive and should end on the final selected image, not an older stale image.

### Task 3: Make Preview and Prefetch Caches Byte-Budgeted

**Priority:** Critical

**Goal:** Bound decoded preview memory by approximate bytes instead of only entry count.

**Problem:** `LibraryManager` currently stores full decoded RGBA buffers in `preview_cache` and `prefetch_cache` with tiny count limits, but count limits do not protect memory. Three 50 MP RGBA images can still be hundreds of MiB, and cloning those buffers raises peak usage.

**Primary files:**

- `src/model/library.rs`
- `src/config/settings.rs` only if adding a user-facing or persisted budget
- `data/io.github.hebbihebb.Sharpr.gschema.xml` only if adding a persisted setting

**Implementation outline:**

1. Add byte accounting for `preview_cache` and `prefetch_cache`.
2. Compute entry bytes as `width as usize * height as usize * 4`.
3. Set conservative defaults:
   - preview cache: 128 MiB;
   - prefetch cache: 64 MiB or smaller.
4. Evict LRU entries while inserting would exceed the budget.
5. If a single decoded image exceeds the cache budget, do not cache it, but still allow it to display as the current image.
6. Add methods to expose cache stats for debugging or benchmark logging.
7. Avoid adding UI preferences in this task unless strictly necessary.

**Acceptance criteria:**

- Cache insertion cannot push `preview_cache` or `prefetch_cache` indefinitely above its budget except for temporary current-image ownership outside the cache.
- Oversized decoded images are displayed but not retained in preview/prefetch caches.
- Existing thumbnail cache behavior is unchanged.

**Verification:**

- Add focused unit tests for eviction behavior in `model::library` tests.
- Run `cd sharpr && cargo test`.
- Run `cd sharpr && cargo build`.
- Manual test: open a folder with several large images and navigate across them; memory should stabilize instead of growing with every recent image.

### Task 4: Reduce Pixel Buffer Cloning When Creating Textures

**Priority:** Critical

**Goal:** Lower peak memory during viewer loads by transferring ownership instead of cloning decoded RGBA unnecessarily.

**Problem:** `ViewerPane::load_image` clones decoded buffers into `current_rgba`, cache entries, and `glib::Bytes`/`gdk4::MemoryTexture`. Even with byte-budgeted caches, temporary clones can double or triple peak memory for a large image.

**Primary files:**

- `src/ui/viewer.rs`
- `src/model/library.rs`
- possibly the new `src/image_pipeline/` module from earlier tasks

**Implementation outline:**

1. Audit every path that receives `(Vec<u8>, width, height)` in `ViewerPane::load_image`.
2. Avoid cloning before deciding whether the image should be cached.
3. Prefer storing one owned buffer in `current_rgba` only when editing, 1:1, or export requires it.
4. If current display only needs a texture, move the buffer into `glib::Bytes::from_owned`.
5. If a cache entry is needed, consider storing `Arc<[u8]>` or another shared ownership type to avoid repeated `Vec<u8>` clones. Keep the implementation simple and compatible with GTK texture construction.
6. Do not change visible behavior.

**Acceptance criteria:**

- Common viewer load path avoids multiple full-size clones of the same RGBA data.
- The edit transform path still has access to pixels when needed.
- Cache invalidation after edits still works.

**Verification:**

- Run `cd sharpr && cargo build`.
- Manual test: load an image, rotate/flip, save/discard, and verify behavior is unchanged.

## Milestone 2: Improve Failure Handling and Startup Behavior

### Task 5: Return Structured Preview Decode Errors

**Priority:** High

**Goal:** Replace silent `Option` failures in the preview pipeline with actionable errors.

**Problem:** Decode paths often use `.ok()?`, losing whether a failure came from permissions, unsupported format, corrupt files, EXIF preview decode failure, or a decoder error. The viewer can end up blank without explanation.

**Primary files:**

- `src/image_pipeline/`
- `src/ui/viewer.rs`
- `src/thumbnails/worker.rs` only if shared error handling is extended there

**Implementation outline:**

1. Introduce `PreviewDecodeError` with variants or string-backed categories:
   - `OpenFailed`
   - `FormatDetectFailed`
   - `DecodeFailed`
   - `Unsupported`
   - `InvalidDimensions`
   - `ExifPreviewFailed`
2. Return `Result<PreviewImage, PreviewDecodeError>` from the shared preview decode function.
3. Add structured `bench_event!` or `eprintln!` logging for failures.
4. Update `ViewerPane::load_image` to show a compact failure state instead of blanking silently. Use existing GTK patterns; do not create a large redesign.
5. Keep thumbnail worker failure behavior unchanged unless it uses shared decode code.

**Acceptance criteria:**

- Viewer decode failures include an error reason in logs or bench events.
- Selected-image decode failure shows an understandable UI state.
- Stale errors from old generations do not replace the current image.

**Verification:**

- Add unit tests for error mapping where practical.
- Run `cd sharpr && cargo test`.
- Run `cd sharpr && cargo build`.
- Manual test: try an unreadable or corrupted image file and confirm the viewer shows a failure state.

### Task 6: Make Raw Folder Open Populate Before Dimension Hydration

**Priority:** High

**Goal:** Keep folder open responsive when the SQLite index is missing or unavailable.

**Problem:** `LibraryManager::scan_folder_raw` currently calls metadata and `image::image_dimensions` for every image before populating the store. The indexed path is better, but raw fallback still matters for first run, failures, removable drives, network mounts, and large folders.

**Primary files:**

- `src/model/library.rs`
- `src/ui/window.rs`
- `src/library_index/mod.rs` only if reusable basic-info helpers are needed

**Implementation outline:**

1. Split raw folder scan into:
   - fast path: file paths, names, file size, modified time, zero/unknown dimensions;
   - background hydration: dimensions and quality.
2. Populate the `gio::ListStore<ImageEntry>` as soon as the fast path completes.
3. Start a background metadata hydration job using the existing channel pattern.
4. Reuse or mirror `start_metadata_indexer` behavior where possible.
5. Make sure disabled folders and current folder generation are respected.
6. Add `bench_event!` events for raw scan start, fast populate, metadata hydration finish.

**Acceptance criteria:**

- Raw folder open does not require image dimensions for every file before showing entries.
- Dimensions update asynchronously in the filmstrip/list after population.
- Indexed folder behavior remains unchanged.

**Verification:**

- Add tests for raw scan helpers if separable.
- Run `cd sharpr && cargo test`.
- Run `cd sharpr && cargo build`.
- Manual test: temporarily disable or remove the index DB, open a large folder, and confirm entries appear before all dimensions are known.

### Task 7: Add Stale-Aware Metadata Loading for Viewer

**Priority:** Medium

**Goal:** Stop spawning unbounded metadata threads from the viewer and avoid stale metadata work competing with preview decode.

**Problem:** `ViewerPane::load_image` spawns a metadata thread for every load, including cache hits. It discards stale UI updates but still performs stale work.

**Primary files:**

- `src/ui/viewer.rs`
- `src/image_pipeline/worker.rs` or a new metadata worker module
- `src/metadata/mod.rs`

**Implementation outline:**

1. Decide whether metadata should be part of `PreviewWorker` results or handled by a small low-priority `MetadataWorker`.
2. Use generation-aware requests.
3. Replace direct `std::thread::spawn` calls in viewer metadata paths.
4. Preserve current metadata chip updates and quality indicator behavior.
5. Avoid blocking image display on metadata.

**Acceptance criteria:**

- `ViewerPane::load_image` no longer spawns metadata threads directly.
- Metadata for stale selections does not update the UI.
- Metadata remains visible for the current selected image.

**Verification:**

- Run `cd sharpr && cargo build`.
- Manual test: rapidly navigate while metadata overlay is visible; metadata should match the final selected image.

## Milestone 3: Add Focused Export and Reduce Destructive Editing Risk

### Task 8: Add a Minimal Export/Resize Backend

**Priority:** High

**Goal:** Implement non-UI image export logic for downscaling selected images.

**Problem:** Sharpr has AI upscaling but no simple export workflow. Export is a core triage task and should be implemented as copy-based output, not in-place editing.

**Primary files:**

- Add `src/export/mod.rs`.
- Update `src/main.rs`.
- Reuse `src/upscale/runner.rs::save_image` only if it can be moved or shared cleanly without coupling export to upscale.

**Implementation outline:**

1. Add export config:
   - source path(s);
   - destination folder;
   - max edge;
   - output format: JPEG, WebP, PNG;
   - lossy quality.
2. Add export result and error types.
3. Decode with EXIF orientation applied.
4. Resize only when dimensions exceed the max edge. Use a sane default filter such as Lanczos3 for photos.
5. Save to destination without overwriting existing files; generate unique names if needed.
6. Keep metadata copying out of scope for the first backend unless there is already a simple safe helper.
7. Add tests around output naming and resize dimensions.

**Acceptance criteria:**

- Export backend can downscale one or more images to a destination folder.
- It never overwrites source files by default.
- It is independent of GTK UI.

**Verification:**

- Add unit tests for size calculation and unique destination naming.
- Run `cd sharpr && cargo test`.
- Run `cd sharpr && cargo build`.

### Task 9: Add Export UI for Current and Selected Images

**Priority:** High

**Goal:** Expose the export backend through a simple GNOME-style workflow.

**Problem:** Users can inspect and select images but cannot produce resized copies for sharing or delivery.

**Primary files:**

- `src/ui/window.rs`
- possibly add `src/ui/export_dialog.rs`
- `src/export/mod.rs`
- `src/ops/queue.rs` integration

**Implementation outline:**

1. Add an action such as `win.export`.
2. Determine source set:
   - if `AppState::selected_paths` is non-empty, export those;
   - otherwise export the current selected image.
3. Add a compact dialog:
   - destination folder chooser;
   - max edge presets: 1920, 2560, original/custom;
   - format: JPEG, WebP, PNG;
   - quality for lossy formats.
4. Run export in a background worker.
5. Report progress through `ops::queue`.
6. Show completion/failure toasts using existing toast overlay pattern.
7. Keep the dialog simple. Do not add full batch-processing UI in this task.

**Acceptance criteria:**

- User can export current image or selected images.
- Export runs off the GTK main thread.
- Progress/failure is visible.
- Source images are not overwritten.

**Verification:**

- Run `cd sharpr && cargo build`.
- Manual test: select one image and export to a new folder; select multiple images and export; confirm resized files are created and originals are unchanged.

### Task 10: Move Destructive Rotate/Flip Behind an Explicit Edit Boundary

**Priority:** Medium

**Goal:** Reduce accidental lossy source-file edits.

**Problem:** Current rotate/flip transforms operate on decoded RGBA and can save back to the source path, re-encoding JPEGs. That is risky for a viewer and overlaps with export.

**Primary files:**

- `src/ui/viewer.rs`
- `src/ui/window.rs`
- `src/export/mod.rs` if transformed export is used

**Implementation options:**

- Preferred: make rotate/flip visual-only until user chooses export/save-copy.
- Acceptable: keep save, but make the UI copy-oriented and explicit about re-encoding.

**Implementation outline:**

1. Audit current rotate/flip actions and header save/discard buttons.
2. Change labels/tooltips to clarify whether the operation edits the source or creates a copy.
3. If keeping source overwrite, add an explicit confirmation warning for JPEG re-encoding.
4. Consider routing transformed output through export instead of `save_edit_pixels`.
5. Preserve cache invalidation when the source actually changes.

**Acceptance criteria:**

- Users cannot accidentally re-encode a JPEG through a casual viewer action without an explicit warning or copy workflow.
- Existing rotate/flip preview behavior remains usable.

**Verification:**

- Run `cd sharpr && cargo build`.
- Manual test: rotate a JPEG and verify the app makes the save/copy consequence explicit.

## Milestone 4: Isolate Optional AI and Clean Product Surface

### Task 11: Hide Advanced AI Upscaling From the Primary UI

**Priority:** High

**Goal:** Keep AI upscaling available but stop it from defining the primary viewer experience.

**Problem:** The audit concludes Sharpr should be a fast viewer with focused export. The AI upscale action, preferences, quality category, and backend choices are too central for v1.0.

**Primary files:**

- `src/ui/window.rs`
- `src/ui/preferences.rs`
- `src/ui/sidebar.rs`
- `src/config/settings.rs`
- `data/io.github.hebbihebb.Sharpr.gschema.xml`

**Implementation outline:**

1. Add a persisted setting such as `show-advanced-ai-tools`, default `false`.
2. Hide or demote:
   - main menu `AI Upscale` section;
   - upscaler preferences page or advanced backend controls;
   - ComfyUI-specific preference exposure;
   - any primary UI affordance that suggests AI is part of the default workflow.
3. Keep the existing upscale code compiled and reachable when the setting is enabled.
4. Add a preference switch under an Advanced section to reveal AI tools.
5. Do not remove the backend code in this task.

**Acceptance criteria:**

- Fresh/default app UI does not expose AI upscaling as a primary action.
- Users can intentionally enable advanced AI tools.
- Existing upscale flow still works after enabling.

**Verification:**

- Run `cd sharpr && cargo build`.
- Manual test: launch with default settings and confirm AI actions are hidden; enable advanced AI tools and confirm actions return.

### Task 12: Simplify the Upscale Dialog Defaults

**Priority:** Medium

**Goal:** Make the optional upscale workflow less backend-centric.

**Problem:** When upscaling is enabled, the dialog exposes backend/model/scale complexity too early. Most users need a default action plus an advanced section.

**Primary files:**

- `src/ui/window.rs`
- `src/ui/preferences.rs`
- `src/config/settings.rs`

**Implementation outline:**

1. Keep existing saved backend/model settings.
2. In the upscale dialog, show only the essential scale/output choice by default.
3. Move backend, model, tile, GPU, and ComfyUI controls into an "Advanced" expander or preferences.
4. Preserve all existing settings and behavior.

**Acceptance criteria:**

- Upscale dialog is simpler on first open.
- Advanced users can still change backend/model settings.
- No backend behavior regresses.

**Verification:**

- Run `cd sharpr && cargo build`.
- Manual test: enable AI tools, open upscale dialog, run default upscale if backend is configured.

## Milestone 5: Maintainability and UX Follow-Through

### Task 13: Extract Export and Processing Actions Out of `viewer.rs`

**Priority:** Medium

**Goal:** Reduce the central viewer module's responsibility.

**Problem:** `src/ui/viewer.rs` currently owns decode details, edit transforms, upscale flow, comparison state, tag UI, progress UI, and metadata display. After earlier tasks, decode should already be extracted; this task continues the separation.

**Primary files:**

- `src/ui/viewer.rs`
- `src/ui/window.rs`
- `src/upscale/`
- new focused UI modules if needed

**Implementation outline:**

1. Identify upscale-specific methods in `ViewerPane`.
2. Move orchestration that does not require direct viewer internals into a focused helper module.
3. Keep `ViewerPane` responsible for rendering the comparison widget and view state only.
4. Avoid changing user-visible behavior.

**Acceptance criteria:**

- `viewer.rs` is smaller and less responsible for processing policy.
- Upscale and comparison behavior remains unchanged.

**Verification:**

- Run `cd sharpr && cargo build`.
- Manual test: open image, run compare/upscale flow if configured, commit/discard.

### Task 14: Add Viewer Pipeline Bench Events and Cache Stats

**Priority:** Medium

**Goal:** Make future performance work measurable.

**Problem:** The audit is performance-driven. Without consistent events for preview requests, cache hits, decode source, stale results, and memory budget eviction, future agents will guess.

**Primary files:**

- `src/bench.rs`
- `src/image_pipeline/`
- `src/model/library.rs`
- `src/ui/viewer.rs`

**Implementation outline:**

1. Add or standardize events:
   - `preview.request`
   - `preview.cache_hit`
   - `preview.decode_start`
   - `preview.decode_finish`
   - `preview.stale_result`
   - `preview.cache_evict`
2. Include source, dimensions, approximate bytes, generation, and duration where useful.
3. Keep instrumentation cheap when `SHARPR_BENCH` is disabled.

**Acceptance criteria:**

- With `SHARPR_BENCH=1`, preview navigation produces enough structured events to diagnose cache behavior and stale work.
- No visible behavior changes.

**Verification:**

- Run `cd sharpr && cargo build`.
- Manual test: `SHARPR_BENCH=1 SHARPR_BENCH_LOG=/tmp/sharpr.jsonl GSETTINGS_SCHEMA_DIR=data cargo run`, navigate several images, inspect `/tmp/sharpr.jsonl`.

### Task 15: Add Focused Documentation for the Image Pipeline

**Priority:** Low

**Goal:** Preserve the architecture decisions so future PRs do not reintroduce split decode paths or unbounded caches.

**Problem:** The audit captures decisions, but implementation details should live near the code once the pipeline exists.

**Primary files:**

- `sharpr/src/image_pipeline/mod.rs` module docs
- `README.md` or a short `sharpr/docs/image-pipeline.md` if the project already has a docs pattern

**Implementation outline:**

1. Document the preview pipeline responsibilities.
2. Explain why full decoded RGBA is not generally cached.
3. Document thread ownership and GTK-main-thread boundaries.
4. Mention required behavior for EXIF orientation and prefetch consistency.

**Acceptance criteria:**

- Future agents can understand the preview pipeline contract before editing it.
- Documentation references the concrete module and app constraints.

**Verification:**

- Run `cd sharpr && cargo build`.

## Suggested Cloud PR Sequence

1. Extract shared preview decode module.
2. Add bounded preview worker.
3. Make preview/prefetch caches byte-budgeted.
4. Reduce pixel-buffer cloning.
5. Return structured preview decode errors.
6. Make raw folder open populate before dimension hydration.
7. Add stale-aware metadata loading.
8. Add minimal export/resize backend.
9. Add export UI.
10. Move destructive rotate/flip behind explicit edit boundary.
11. Hide advanced AI upscaling from primary UI.
12. Simplify upscale dialog defaults.
13. Extract processing orchestration out of `viewer.rs`.
14. Add viewer pipeline bench events and cache stats.
15. Add image pipeline documentation.

The first four tasks should be treated as the stability foundation. Export work should wait until the decode pipeline is shared, so export does not create a third independent image-loading path.
