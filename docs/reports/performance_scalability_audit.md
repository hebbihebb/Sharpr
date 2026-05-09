# Sharpr Performance and Scalability Audit

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Report orientation: large-library behavior for 10k to 100k images

## Executive Summary

Sharpr already has the right architecture for large folders: chunked loading, background thumbnail workers, generation-based stale result handling, pending-path dedupe, SQLite-backed indexing, metadata workers, quality/phash background work, and opt-in benchmark logging. Those are strong foundations for 10k-image libraries.

The 100k-image risks are predictable: too much path-heavy work, unbounded result queues, eager metadata/hash/quality work, expensive virtual views, full-path benchmark logs, image decode memory spikes, and rapid folder switching while workers are still producing results. The next performance work should be measurable stress testing before optimization.

## Current Strengths

- Thumbnail worker has visible and preload pools.
- Worker requests carry a generation counter so stale folder-switch work can be skipped.
- Pending thumbnail paths are deduplicated.
- Visible thumbnail requests are prioritized over preload requests.
- Thumbnail request channels are bounded.
- Folder reconciliation batches SQLite writes in transactions.
- SQLite indexes exist for common folder, quality, phash, collection, and pipeline queries.
- Metadata loading uses a background worker.
- Heavy virtual-view operations use background work and drain results back onto the main context.
- Benchmark logging exists for folder open, index, thumbnails, hashes, virtual views, and other expensive paths.
- Quality and duplicate systems are already separated from UI rendering.

## Most Likely Bottlenecks

- Full folder scans and path list construction for very large/deep trees.
- Large `Vec<PathBuf>` and `HashSet<String>` allocations during reconciliation.
- Thumbnail decode memory spikes for huge images or many large PNG/JXL/WebP files.
- Unbounded result channels for thumbnail, hash, sharpness, metadata, and some virtual-view flows.
- Hash and sharpness work chained after thumbnail generation, which can compete with visible responsiveness.
- Quality and duplicate virtual views that require broad index scans or background metadata completion.
- Benchmark logging of every thumbnail/hash path when enabled, which can become heavy at 100k scale.
- Main-thread list-store updates if rows are appended too aggressively.
- ComfyUI/upscale/export memory usage when finalizing large outputs.
- Network-mounted or slow HDD folders where metadata calls and directory traversal are latency-bound.

## Top 10 Performance Improvements or Tests

1. Add a stress fixture generator.

   Generate fake folder trees with 10k and 100k files, mixed extensions, hidden folders, long names, and nested directories. Use tiny valid images plus corrupt files.

2. Add benchmark scenarios around folder open.

   Measure cold index, warm index, changed mtime, deleted files, ignored folders, and network/slow storage simulation.

3. Cap or backpressure result channels.

   Review unbounded channels in thumbnail, hash, sharpness, metadata, and virtual-view flows. Add bounded queues or coalescing where stale results are expected.

4. Separate visible thumbnails from derived analysis.

   Ensure phash and sharpness never delay visible thumbnail generation or folder switching.

5. Add pixel-count guards before decode.

   Refuse or downsample extreme images before allocating full decoded buffers in thumbnail/export/upscale paths.

6. Batch main-thread UI updates.

   For large row loads and virtual views, append/update in chunks so the GTK main loop keeps repainting.

7. Add SQLite query plans to performance tests.

   Check `EXPLAIN QUERY PLAN` for folder, quality, duplicate, collection, and future saved-search queries.

8. Measure benchmark logger overhead.

   At 100k thumbnails, benchmark logging should remain opt-in and avoid becoming the bottleneck.

9. Add cancellation semantics to virtual views.

   Folder switching has generation counters; duplicates, quality scans, metadata backfills, and search should use similar stale-result rejection.

10. Test rapid folder switching.

   Repeatedly switch between large folders while thumbnails, metadata, phash, and quality tasks are active. Verify no stale images appear and memory returns to baseline.

## Safe Now vs Risky

Safe now:

- Add stress scripts/tests and benchmark scenarios.
- Add query-plan checks.
- Add path redaction option for benchmark logs.
- Add stale-result generation counters to more virtual views.
- Add documentation for expected large-library behavior.

Risky without measurement:

- Rewriting the thumbnail scheduler.
- Combining all SQLite data into one database.
- Adding aggressive prefetch or background indexing for all libraries at startup.
- Running phash/quality/metadata for every file immediately.
- Adding file-monitor recursion before ignored-folder and backpressure behavior is solid.

## Suggested Benchmark Scenarios

- 10k JPEG/PNG/WebP/JXL mixed folder, cold cache.
- 10k folder, warm SQLite index, warm thumbnail cache.
- 100k files across deep folder tree, first scan.
- 100k files with 1 percent changed mtimes.
- 100k files with 10 percent deleted/moved.
- Rapid switch across five 5k-image folders.
- Huge single image: very large dimensions, normal file size.
- Corrupt image batch: 1k files that fail decode.
- Slow storage: artificial latency around metadata reads.
- ComfyUI/export/upscale on large inputs and outputs.
- Benchmark enabled vs disabled on the same workloads.

## Files and Modules to Watch

- `sharpr/src/thumbnails/worker.rs`
- `sharpr/src/thumbnails/cache.rs`
- `sharpr/src/library_index/mod.rs`
- `sharpr/src/model/library.rs`
- `sharpr/src/ui/window.rs`
- `sharpr/src/image_pipeline/worker.rs`
- `sharpr/src/quality/*`
- `sharpr/src/duplicates/*`
- `sharpr/src/export/mod.rs`
- `sharpr/src/upscale/*`
- `sharpr/src/bench.rs`

## Practical Position

Sharpr is architecturally prepared for large libraries, but it needs hard numbers before broad optimization. The best next step is a repeatable stress harness that makes 10k and 100k image workflows measurable across folder open, thumbnail display, virtual views, memory, and cancellation.
