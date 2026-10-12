# Sharpr Roadmap

## Product Direction

Keep the viewer-reset interface, filmstrip, viewer, manual tags, and tag-backed buckets. Discovery should narrow a large folder without requiring manual labeling. Originals remain immutable; preparation creates new copies. Diffusion and SeedVR remain external.

## Delivery Order

### Phase 0 — Trustworthy baseline

Correct the installed Sharpr launcher and icon, rebuild the launched binary, and record performance before indexing. Preserve settings and curation data.

### Phase 1 — Tags and ordinary search

Search within the current folder/bucket by default, with an explicit whole-library option. Restore scope, selection, and scroll position when clearing search. Surface persistence failures; keep work off the GTK thread. Stop creating automatic format/resolution tags while preserving every existing assignment and bucket.

### Phase 2 — Aspect and resolution

Sort by aspect in both directions and closeness to 16:9. Filter landscape, portrait, square, 16:9 ±2%, and minimum displayed width/height. Respect EXIF orientation; resolve unknown dimensions in the background. Combine filters with scope and search.

### Phase 3 — Semantic-search feasibility gate

Evaluate EmbeddingGemma 2 against SigLIP 2 on a representative 1,000-image sample and 30 predefined queries with expected examples. Compare Google's reference implementation with pinned native llama.cpp. Start at normalized 768 dimensions. Reference inference uses BF16 or float32, never float16.

Require useful top-20 results for at least 80% of queries, warm queries below one second on this laptop, and no more than 10% measured navigation slowdown during indexing. CPU operation is required; GPU is optional. Select Gemma if it passes, otherwise SigLIP only if it passes. If neither passes, do not integrate semantic search.

### Phase 4 — Offline visual search (gated by Phase 3)

Reuse the search entry/filmstrip with Visual search and Names & tags modes. Filter scope/metadata before ranking; display closest 200 with load-more and Find similar. Keep results stable until refreshed. Opt-in indexing is incremental, resumable, lower priority than browsing, and reports coverage/errors.

Vectors are rebuildable SQLite data, versioned by model/preprocessing and file-change identity. Reject incompatible vectors and stale results; respect disabled folders. Use local model files and an app-managed native subprocess. Loopback HTTP is permitted only for that local child, never external inference or automatic downloads. Verify packaging; do not bundle Python/PyTorch as a fallback.

### Phase 5 — Wallpaper preparation

Crop Copy defaults to a movable 16:9 selection and offers free aspect. Save intermediate PNG copies. WebP Copy defaults to original dimensions and quality 90, with optional resizing. Choose a destination including Wallpapers, use unique names, clean failed outputs, and inspect copies in the viewer/Compare. No remote orchestration or persistent processing queues.

Phase 5 can ship independently while the semantic benchmark awaits relevance judgments.

## Verification

Test search restoration, compound filters, tag/bucket persistence, disabled folders, stale results, corrupt files, indexing recovery/model mismatch, EXIF crops, collision safety, and original-byte preservation. Measure real collections plus 10,000/50,000-entry ranking fixtures. Build before manual handoff; nextest for behavior changes, deny/machete for dependency changes.

## Boundaries

No generated descriptive tags, AI upscalers, server orchestration, model downloaders, persistent queues/history, source metadata writes, or full editor. Extract focused controllers when touched; avoid a broad rewrite. Old queue tables/settings remain inert. Historical work lives in docs/archive/.
