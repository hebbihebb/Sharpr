# Current Tasks

## Active Milestone

`ROADMAP.md` Phase 1, Data-safety repairs. Phase 0 finished on October 12; its summary is in `docs/archive/history.md`.

## State (October 12, 2026)

- **Git:** `main` and `viewer-reset` are both at the Phase 0 close. Development continues on `viewer-reset`, and `main` is fast-forwarded only after user acceptance. Neither is pushed: `origin/main` is still the pre-July version. Pushing is the user's call.
- **Archive branches, kept:**
  - `preserve/october-12-wip` (`68858b3`): the full October 12 snapshot
  - `visual-search-trial-wip` (`48a0539`): the unfinished trial wiring, which lacks `semantic_trial.rs`
- **Deployed:** release build `5832010` at `~/.local/opt/sharpr/current`, accepted by the user. Builds in the checkout don't affect it. Redeploy only with approval: `python3 tools/install-native-launcher.py`.
- **Backup:** `~/.local/share/sharpr.backup-20261012-221640/`, taken with Sharpr closed. Every file is byte-verified, and both databases pass an integrity check. The live data has 4,544 images, 6 buckets (Model, Art › Diffusion, upgrade?, CSR, Trash?), and 7,718 tag rows, 95% of them automatic.
- **Product direction:** `docs/reference/product.md` (D1–D19). Tags are frozen.

## Performance Baseline

Phase 2 and Phase 9 compare against these numbers, from release build `5832010` with the user browsing the real library on October 12. The log is `~/.cache/sharpr/logs/run-1791843839575-1067250.jsonl`.

| Measure | Result |
| --- | --- |
| Startup to first folder ready | 498 ms |
| Folder open, 2,216 images | 116–162 ms. One outlier took 2,449 ms: its scan waited 2.3 s in the busy rayon pool. |
| Folder open, 408 images | 34 ms |
| Thumbnail worker time | p50 1 ms, p95 4 ms, max 293 ms (about 8,000 thumbnails) |
| Memory (RSS) | 243 MB at start, 2,345 MB peak, 1,382 MB at end |
| CPU | 4% median, 528% peak |

The user judged scrolling and switching as "everything works and feels fast". There is no stall probe yet; it comes in Phase 2.

## Next Task: Phase 1, Item 1 (Stop Deleting Tags For Missing Files)

**Goal:** Reading tags never deletes tag rows. `TagDatabase::all_tags()`, `paths_for_tag()`, and `prune_*` in `sharpr/src/tags/db.rs` filter missing paths when reading, and keep the rows.

**Why first:** It happened in real use. When Sharpr restarted at 21:29 on October 12, 81 paths lost their tags, including 21 files that had been moved outside Sharpr. Moved files lost only automatic tags this time, but a bucket member would have dropped out silently.

**Acceptance:**
- A regression test with a tagged path that is missing on disk: after `all_tags()` and `paths_for_tag()`, the rows still exist, and counts and lists exclude the missing path.
- The test that enshrined pruning is replaced.
- A missing library root deletes nothing.
- `./check.sh` passes.

**Guardrails:**
- Don't touch the live data. Test in isolated directories (`docs/reference/development.md`).
- Make no tagging UX changes; tags are frozen (D1).
- Commit when done. Continue with Phase 1 items 2–6 in order, committing each user-testable unit.
