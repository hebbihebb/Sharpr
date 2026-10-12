# Current Tasks

## Active Work

Implement the approved wallpaper-focused roadmap on `viewer-reset`: baseline, scoped ordinary search/tag reliability, aspect/resolution controls, semantic benchmark, and crop/WebP copies.

## Semantic Gate

Visual search must not ship before the 1,000-image/30-query benchmark passes. The user supplied 17 search terms; a private 30-query manifest and 1,000-image sample are prepared. Local reference models and pinned native llama.cpp are installed for evaluation. Both Gemma runs are in progress; native parity, relevance review, browsing impact, and packaging verification remain pending. Do not call the model choice or integration complete without evidence.

The user accepts overnight whole-library indexing. Throughput is measured, not a release gate; folder/subset indexing and on/off or scheduling controls can follow later. Disabled folders must remain excluded.

## Implementation State

Launcher identity is repaired in commit `41a1a19`. Scoped ordinary search, manual-tag persistence, aspect/resolution controls, crop copies, and WebP copies are implemented in the working tree. Build, strict clippy, 134 automated tests, the isolated GTK search/copy test, cargo deny, and cargo machete pass. Final review, documentation, and implementation commit are still required. The optional semantic-search application integration has not started.

## Self-Audit Before Final Handoff

Requested by the user on October 12: self-audit before writing the final audit summary. Reviewed today's commits and uncommitted implementation; the final audit artifact must wait until the job is finished.

Corrected findings in the working tree:

- Export could overwrite a pre-existing hidden temporary image. A failing regression reproduced this; exclusive temporary-file reservation now preserves it.
- Buffered encoder flush errors could be discarded. JPEG/PNG/WebP now explicitly check flushing before publishing the copy.
- Creating a bucket with selected images performed bulk writes on GTK. It now runs in a worker and rolls back a newly created bucket when tag assignment fails.
- Bucket rename could treat a failed membership read as an empty result. Checked reads now stop the operation and show errors.
- Choosing closest-to-16:9 could leave the direction menu inconsistent. Action state now follows the applied order.
- Clearing a search could restore old ordering after a sort change. Background re-sorting preserves the new order; the GTK test covers this with a persisted bucket and copy selection.
- Repeated tag-popover reads for the same image could apply old rows after a newer read. A separate refresh generation now rejects those results.
- EXIF handling covered JPEG only. PNG/WebP/TIFF use the existing image decoder's orientation support; a PNG crop regression checks dimensions, pixels, and original-byte preservation.
- The baseline lockfile contained RUSTSEC-2026-0204 and RUSTSEC-2026-0186. Updated only crossbeam-epoch to 0.9.20 and memmap2 to 0.9.11, and regenerated matching Flatpak source checksums.

Open review limits: hierarchy/rename updates span two databases and use compensating rollback, not crash-safe atomic commits; whole-library scan cancellation does not yet interrupt directory traversal; benchmark relevance/parity/navigation/package gates remain unverified. Flatpak source metadata matches the lockfile, but a full Flatpak build has not been run for today's changes. Credential-pattern scan passed and HF token permissions are owner-only, outside the repository.

## Constraints

Preserve the interface, originals, existing tags, and tag-backed buckets. Stop new automatic labels. Diffusion/SeedVR remain external. Keep legacy task/settings state inert. Build before manual handoff; do not push.
