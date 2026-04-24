# Sharpr Architecture and UX Audit

This audit treats Sharpr as a GNOME-native image library viewer whose first job is fast, predictable triage across large folders and libraries. The existing report is directionally right about scope drift, but it needs sharper technical prioritization and a few corrections based on the current code.

## 1. Validation of the Existing Report

### Scope Drift Toward an AI Lab

**Verdict: correct and well-prioritized.**

Sharpr is currently becoming a hybrid of fast viewer, photo manager, quality classifier, duplicate finder, smart tagger, and AI upscaling workbench. The strongest contradiction is in the upscale surface: `src/upscale/` contains CLI, ONNX, and ComfyUI backends, model downloads, tiling, comparison UI, output format controls, GPU/tile settings, and preferences. That is a large product surface for a feature that does not improve the core navigation loop.

The report is right that this risks burying the main value proposition. It is incomplete because smart tags, quality folders, duplicate views, collections, and metadata overlays also compete with the same interaction budget. Upscaling is the clearest offender, but not the only source of scope pressure.

### Large Image Handling and Prefetching

**Verdict: correct but incomplete.**

The code does consider large images. `src/ui/viewer.rs` first attempts embedded previews through EXIF metadata, then uses turbojpeg scaled decode for JPEGs, and falls back to full `image` crate decode. That is a better implementation than a naive full-resolution decode path.

The problem is that the memory model is still wrong for a viewer. `LibraryManager` stores decoded RGBA buffers in both `prefetch_cache` and `preview_cache`, while `ViewerPane` also stores `current_rgba`. Loading from cache clones those buffers before creating `gdk4::MemoryTexture`. Even when JPEGs are downscaled, this is still uncompressed pixel memory. PNG, WebP, TIFF, AVIF, and fallback paths can still become full decoded RGBA.

There is also a correctness issue the report missed: `prefetch_decode` in `src/ui/window.rs` uses `image::decode().into_rgba8()` without the viewer's EXIF-orientation and JPEG-scaled-preview logic. A prefetched image can therefore display differently from a non-prefetched image, and that decoded result is promoted into `preview_cache`.

### Thread Spawning

**Verdict: partly correct, partly overstated.**

The thumbnail pipeline is not the problem described by the report. `src/thumbnails/worker.rs` already uses long-lived visible and preload worker pools, generation checks, and in-flight path deduplication.

The viewer pipeline is still a problem. `ViewerPane::load_image` spawns one OS thread for image decode and another for metadata on each slow-path image load. The upscale comparison viewer also spawns per-image decode threads. Prefetch uses Rayon, which bounds the global pool but does not cancel stale decode work once a request has started. During rapid keyboard navigation, this can waste CPU and memory bandwidth on images the user will never see.

### Rotate and Flip

**Verdict: misleading as stated.**

Basic orientation handling is core for image triage, but destructive rotate/flip editing is not the same thing as lossless orientation correction. Sharpr currently applies transforms to decoded RGBA and writes pixels back to disk in `save_edit_pixels`, re-encoding JPEGs. That is risky for a viewer because it turns a lightweight viewing action into destructive image editing with quality loss.

The right feature is non-destructive orientation display plus explicit, safe save/export paths. Lossless JPEG rotation or metadata orientation correction can be considered later, but casual in-place re-encoding should not be central to v1.0.

### Upscale Versus Downscale

**Verdict: mostly correct, but the recommendation should be narrowed.**

The app has a heavy AI upscaling system and no simple export/resize workflow. That is backwards for a triage product. However, "batch downscaling" should not become a full image-processing suite. The core workflow should be: select images, export copies, choose max edge or preset, choose JPEG/WebP/PNG and quality, preserve/copy metadata intentionally.

Downscaling/export should be core because it completes the triage loop. AI upscaling should be optional and isolated.

## 2. Correct Architecture for Sharpr

Sharpr should be a **fast viewer with focused export**, not a processing tool. Processing features are allowed only when they do not compete with navigation responsiveness.

### Image Loading Pipeline

Use one coherent decode pipeline for on-screen previews, prefetch, and comparison:

1. Main thread receives selected path and increments a viewer generation.
2. UI immediately shows the last frame or a lightweight loading state; it does not clear to blank unless the new image fails.
3. A bounded preview worker accepts a `PreviewRequest { path, generation, target_kind }`.
4. Worker chooses source in this order:
   - Valid cached preview texture or decoded preview for the same file signature.
   - Embedded preview large enough for the viewport.
   - JPEG scaled decode through turbojpeg.
   - Format-specific full decode only when unavoidable.
5. Worker applies EXIF orientation in every path, including prefetch.
6. Worker returns either a display-sized preview or a full-resolution decode only for explicit 1:1 inspection.
7. Main thread discards stale generation results before creating GTK paintables.

The current split between `decode_image_rgba`, `prefetch_decode`, thumbnail decode, and comparison decode should be collapsed into shared image-pipeline code. The current duplication is already producing inconsistent behavior.

### Caching Strategy

Cache by intent, not by convenience:

- **Disk thumbnail cache:** keep. It is valuable and bounded by file count rather than peak RAM. Continue using GNOME/freedesktop thumbnails when available.
- **In-memory thumbnail cache:** keep, but bound by approximate bytes as well as count. `500` textures is not always cheap if thumbnails become wide panoramas.
- **Preview cache:** keep only small, display-oriented previews and bound it by bytes. A good default is a 128-256 MiB preview budget with LRU eviction.
- **Full-resolution cache:** do not maintain a general full-res RGBA cache. Hold at most the currently inspected full-resolution buffer, and only while 1:1 or edit/export needs it.
- **Prefetch cache:** prefetch decoded display previews or warm file metadata, not arbitrary full decoded RGBA. Prefetch should have a tiny budget and should be canceled or ignored aggressively.
- **Metadata cache:** keep lightweight metadata and dimensions. Persist stable metadata in SQLite where possible.

Do not store multiple cloned `Vec<u8>` copies for the same image. If decoded pixel memory must cross threads, transfer ownership once and avoid retaining both the source vector and a cache clone after the texture is built.

### Threading Model

Sharpr already has the right broad pattern: GTK objects stay on the main thread, work goes to background threads, and results return through channels.

The missing piece is a single bounded job model for previews:

- Thumbnail generation remains on its visible/preload worker pools.
- Preview decode gets its own small pool, usually 1-2 workers, because preview decode is latency-sensitive and memory-heavy.
- Metadata extraction should be folded into the preview job when it touches the same file, or handled by a low-priority metadata pool.
- Prefetch requests should be generation-aware and distance-aware. New navigation should make older prefetch work irrelevant.
- AI jobs should run through the operation queue with explicit resource isolation. They must not consume the same workers used for preview decode.

Do not add an async runtime. The existing Rust thread/channel model is enough; it just needs clearer ownership and bounded queues.

### UI and Processing Separation

The viewer should own interaction state, not image-processing policy. `src/ui/viewer.rs` currently owns decode details, edit transforms, upscale flow, progress UI, comparison state, tag UI, and metadata display. That makes the central viewer module too expensive to reason about.

Move reusable decode/export logic into focused non-UI modules. UI modules should issue jobs, render states, and respond to results. Processing modules should never know about GTK widgets.

## 3. Feature Scope Decision

Sharpr should be primarily **a fast viewer and triage tool with focused export**.

### AI Upscaling

AI upscaling should be an **optional module**, not core. It may remain in the repository if it is clearly isolated, disabled by default in the primary workflow, and unable to steal CPU/GPU/memory from navigation. The UI should expose it as an advanced action for a selected image, not as a first-class organizing concept.

The ONNX and ComfyUI backends are especially heavy for v1.0. The maintainable v1.0 choice is either:

- keep only external CLI integration behind an advanced preference, or
- move the whole feature behind a compile-time or plugin-style boundary.

### Downscaling and Export

Downscaling/export should be **core**. It directly follows from triage: pick images, make copies suitable for sharing, upload, review, or delivery.

The right v1.0 scope is intentionally small:

- export selected image or selected set;
- max edge presets such as 1920, 2560, original;
- JPEG/WebP/PNG output;
- quality control for lossy formats;
- destination folder;
- no in-place overwrite by default.

This gives users real daily value without turning Sharpr into an editor.

## 4. Top Risks

### 1. Memory Blowups During Navigation

This surfaces with high-megapixel images, panoramas, non-JPEG formats, or rapid navigation. The current design can hold current pixels, preview cache entries, prefetched entries, channel payloads, and texture-owned bytes at once. Because these are uncompressed RGBA buffers, small cache counts still produce large memory spikes.

The fix is byte-budgeted caches and display-preview-first decoding.

### 2. Stale Background Work Competing With Visible Work

This surfaces when users hold arrow keys, scroll quickly, or open a large virtual view. Thumbnail workers have generation checks, but viewer decode threads and Rayon prefetch jobs can still complete work for images the user has already skipped. That creates UI latency and unpredictable CPU usage.

The fix is a bounded preview queue with generation discard before expensive work where possible, plus a strict separation between visible decode and speculative prefetch.

### 3. Product Surface Outgrowing the Core Viewer

This surfaces as more menu items, preferences, states, and edge cases pile into `viewer.rs` and `window.rs`. Upscaling, editing, tags, quality, duplicates, collections, and export all want to touch selection, caches, disabled folders, file invalidation, and progress. Without stronger boundaries, every feature will make navigation behavior more fragile.

The fix is to make fast viewing the architectural center and force optional features through narrow job and result interfaces.

## 5. Refined Final Report

### A. Critical Issues

#### 1. Decoded RGBA Caching Is the Wrong Memory Architecture

**What is wrong:** `LibraryManager` caches decoded RGBA in `prefetch_cache` and `preview_cache`, and `ViewerPane` separately stores `current_rgba`. Loading from these caches clones pixel buffers before creating textures.

**Why it matters:** A viewer must have predictable memory use. Uncompressed pixel buffers scale with dimensions, not file size. A few large images can consume hundreds of MiB, and temporary clones raise peak memory further.

**What to do instead:** Cache display-sized previews with a byte budget. Keep only the current full-resolution buffer when explicitly needed for 1:1 or export/edit. Replace count-only cache limits with approximate-memory limits.

#### 2. Prefetch Uses a Different Decode Path Than the Viewer

**What is wrong:** `prefetch_decode` does not apply the same EXIF orientation, embedded-preview selection, or JPEG scaled decode policy as `decode_image_rgba`.

**Why it matters:** The same file can display differently depending on timing. A prefetched result can also poison `preview_cache`, making the wrong display persistent until invalidation.

**What to do instead:** Create one shared preview decode function used by direct load, prefetch, comparison, and eventually export preview generation.

#### 3. Viewer Decode Work Is Not Bounded as a First-Class Pipeline

**What is wrong:** Slow-path viewer loads spawn per-request OS threads for decode and metadata. Prefetch uses Rayon but does not cancel started work.

**Why it matters:** Rapid navigation can create stale CPU and memory pressure exactly when the user expects the app to feel most responsive.

**What to do instead:** Add a small bounded preview worker pool with generation-aware requests, queue replacement for stale paths, and separate low-priority prefetch.

### B. High Priority

#### 4. AI Upscaling Is Too Central for the Product Goal

**What is wrong:** Upscaling has multiple backends, model management, preferences, comparison UI, and output controls embedded in the main app workflow.

**Why it matters:** It adds dependencies, settings, failure modes, and resource contention while not improving browse latency or triage speed.

**What to do instead:** Treat AI upscaling as an optional advanced module or external integration. Keep it out of the primary navigation surfaces and isolate its jobs from preview workers.

#### 5. Missing Export/Resize Workflow

**What is wrong:** Sharpr can run AI super-resolution but cannot perform a simple, predictable export to smaller files.

**Why it matters:** Export is a natural completion step for triage. Without it, users can select and inspect images but not easily produce usable delivery copies.

**What to do instead:** Add a focused export workflow: selected images, max edge, output format, quality, destination, and copy-vs-overwrite safety.

#### 6. File Scanning and Metadata Hydration Can Still Block Perceived Startup

**What is wrong:** Direct folder scanning still calls filesystem metadata and `image::image_dimensions` across every file in the raw path. Indexed loading improves this, but fallback behavior remains important.

**Why it matters:** On large folders, network mounts, slow disks, or corrupt files, initial population can become slow before the user gets useful UI.

**What to do instead:** Prefer indexed rows immediately, then reconcile asynchronously. In raw fallback, populate path/name/file-size first, then hydrate dimensions and quality in background batches.

### C. Medium Improvements

#### 7. Error Reporting Is Too Silent in Decode Paths

**What is wrong:** Many image paths use `Option` and `.ok()?`, losing the reason for failure.

**Why it matters:** Unsupported formats, corrupt files, permission errors, and decoder failures all become blank output or missing thumbnails. That hurts trust at scale.

**What to do instead:** Return typed or at least string errors from decode workers. Log structured failures and show a compact viewer error state for the selected image.

#### 8. Destructive Rotate/Flip Is Product-Risky

**What is wrong:** In-memory transforms can be saved back to the source file, re-encoding JPEGs.

**Why it matters:** A viewer should not make lossy destructive edits feel casual. This can degrade original files and complicate metadata correctness.

**What to do instead:** Keep display orientation automatic. Move destructive transforms into export or an explicit edit mode. If in-place orientation is kept, prefer lossless/metadata-safe operations where possible.

#### 9. Viewer and Window Modules Carry Too Much Behavior

**What is wrong:** `viewer.rs` and `window.rs` coordinate decode, prefetch, metadata, tags, edits, upscale, actions, and virtual views.

**Why it matters:** The app becomes harder to maintain because unrelated features share state and invalidation paths.

**What to do instead:** Extract image pipeline, export, and optional processing jobs into non-UI modules with narrow request/result types.

### D. Low / Optional

#### 10. Scaling Filter Choice Is Not a Core Preference

**What is wrong:** The earlier report suggested exposing interpolation choices because `Lanczos3` is hardcoded in upscale finalization.

**Why it matters:** This matters for specialized pixel art or screenshots, but it is not central to a v1.0 triage viewer.

**What to do instead:** Use sane defaults for viewer previews and export. Add filter selection only if export becomes important for graphic assets, not as a general preference.

#### 11. Smart Tags and Quality Views Need Product Restraint

**What is wrong:** Smart tags and quality classification can be useful, but they also introduce models, background work, and extra navigation concepts.

**Why it matters:** They compete with the fast triage loop if surfaced too aggressively.

**What to do instead:** Keep them opt-in, progress-visible, cancelable, and respectful of disabled folders. Do not let them block folder open or image navigation.

### E. Rejected / Overstated Findings

#### "All Threading Is Broken"

Rejected. The thumbnail subsystem already has a sensible pool model with generation checks and deduplication. The issue is specifically the viewer/full-preview pipeline and stale speculative work.

#### "Large Images Are Ignored"

Rejected. The viewer attempts embedded previews and scaled JPEG decode. The real issue is inconsistent reuse of that logic and caching decoded pixel buffers without a byte budget.

#### "Rotate/Flip Should Simply Be Removed"

Overstated. Orientation correctness is core. What should be rejected is casual destructive in-place pixel re-encoding inside a viewer workflow.

#### "AI Upscaling Must Be Deleted"

Overstated as an engineering mandate, but correct as a product warning. It can survive only if isolated as optional advanced processing. It should not shape the app's core architecture or primary UI.

## 6. Final Verdict

Sharpr is currently becoming a broad image-management and AI-processing application. That direction will make it slower, harder to reason about, and less trustworthy for the core use case.

Sharpr should become a fast GNOME image triage viewer with reliable thumbnails, low-latency preview navigation, lightweight organization, and focused export.

The single most important change is to replace the ad hoc decoded-RGBA viewer/prefetch caches with one bounded, generation-aware preview pipeline that applies the same decode, orientation, cache, and stale-result rules everywhere.
