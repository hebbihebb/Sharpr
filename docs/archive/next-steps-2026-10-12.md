# Sharpr — next steps after the independent audit

> Archived. These findings were folded into `ROADMAP.md` Phases 0–5 during the October 2026 documentation realignment. Kept as historical evidence; they are not instructions.

Approved October 12: deliver audited ordinary browsing and an optional native visual-search trial using the existing 1,000-image sample. User testing comes before a release decision. Preserve original images, manual tags, buckets, and benchmark artifacts. Do not push.

## Audit remediation

- Reproduce Escape after changing sort, then cancel old work before restoring the view.
- Test parent/child/grandchild rename with inherited extra tags; change behavior only if the claimed bug reproduces.
- Retry output names when either final or temporary file reservations collide, preserving existing files.
- Use a single search worker, latest-request precedence, GTK search delay, cancellable scans, and file-identity-aware dimension reuse.
- Move remaining tag-summary and collection persistence operations off GTK; guard stale results.
- Show whole-library results for an empty query; retain and display combined filters on tag activation.
- Verify crop-preview orientation and provide an oriented worker-decoded fallback.
- Remove the unused automatic-tag indexer and synchronize reference documentation.
- Keep compensating rollback across the two SQLite databases. Durable crash recovery is explicitly deferred and remains a documented limitation. Import tag batches must be transactional.

## Sample-first visual-search trial

- Names & tags remains default. Visual search trial uses the same entry, viewer, and filmstrip.
- Import completed native sample vectors into a separate rebuildable local index. Record image identities, vector dimensions, model/projector hashes, runtime identity, and preprocessing; reject incompatible or changed data.
- Load and query the pinned local llama.cpp child in a worker. Use authenticated loopback only; no proxies, redirects, external inference, automatic downloads, or Python application runtime.
- Apply folder/bucket and metadata scope before ranking. Show eligible indexed coverage and distinguish no coverage from a poor match. Keep results stable until scope/query/filter changes.
- Trial delivery is native only. Default and Flatpak builds keep semantic search disabled until runtime packaging is verified.
- Whole-library indexing, incremental updates, Find similar, and granular controls follow user testing. Overnight indexing remains acceptable.

## Evidence and verification

Preserve original benchmark reports. Add provenance/parity tools and separate relevance judgments. All original runs completed with 1,000 images and zero decode errors. Initial Gemma warm p95: reference 51 ms, native 32 ms. Preliminary image-vector median cosine: 0.983; mean top-20 overlap: 79.5%. Changing power/concurrent workloads prevent controlled speed comparisons.

User testing is the practical evaluation. The agreed formal rubric remains at least 10 relevant top-20 results, with one in the top five, for at least 24 of 30 queries. Unjudged results cannot pass. Repeat query timings under stable conditions; require warm p95 under one second and no more than 10% measured browsing impact. Throughput is descriptive, not a gate.

Verify audit regressions, original-byte preservation, EXIF previews, scope/disabled folders, stale results, malformed vectors and runtime failures. Run build, clippy, nextest, deny, machete, isolated GTK tests, and a standard Flatpak build. Record unsupported trial packaging separately.

## Handoff

Self-audit again, write the final handoff summary with findings/dispositions, tests and limits, and commit coherent implementation units without pushing. The execution round ends when the ordinary features and native sample trial are documented, committed and ready for user evaluation; it does not establish full-library semantic-search readiness.
