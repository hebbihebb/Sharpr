# Current Tasks

## Active Work

Implement the approved wallpaper-focused roadmap on `viewer-reset`: baseline, scoped ordinary search/tag reliability, aspect/resolution controls, semantic benchmark, and crop/WebP copies.

## Semantic Gate

Visual search must not ship before the 1,000-image/30-query benchmark passes. The user supplied 17 search terms; a private 30-query manifest and 1,000-image sample are prepared. Local reference models and pinned native llama.cpp are installed for evaluation. Both Gemma runs are in progress; native parity, relevance review, browsing impact, and packaging verification remain pending. Do not call the model choice or integration complete without evidence.

The user accepts overnight whole-library indexing. Throughput is measured, not a release gate; folder/subset indexing and on/off or scheduling controls can follow later. Disabled folders must remain excluded.

## Implementation State

Launcher identity is repaired in commit `41a1a19`. Scoped ordinary search, manual-tag persistence, aspect/resolution controls, crop copies, and WebP copies are implemented in the working tree. Build, strict clippy, 131 automated tests, and the isolated GTK search/copy test pass. Final review, documentation, and implementation commit are still required. The optional semantic-search application integration has not started.

## Constraints

Preserve the interface, originals, existing tags, and tag-backed buckets. Stop new automatic labels. Diffusion/SeedVR remain external. Keep legacy task/settings state inert. Build before manual handoff; do not push.
