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

## Phase 1 Progress

**Item 1: done (committed, verified, not yet deployed).**
- Reading tags never deletes rows. `paths_for_tag()` and `all_tags()` hide missing files and stat them after releasing the database lock.
- Trash no longer deletes an image's tags, so restoring from the trash brings its tags and bucket membership back.
- Tests: the regression test proves rows survive a vanished library root and reappear when the files return; another covers per-tag counts. `./check.sh` passes, with 136 tests in isolated directories.
- **Deployment:** the deployed build `5832010` still has the bug. Redeploy needs the user's approval.

**Item 2: done (committed, verified, not yet deployed).**
- `file_ops::rename_no_replace` links and then unlinks, so an existing file is never replaced; filesystems without hard links fall back to a check plus rename. `rename_within_dir` also rejects `/`, `.`, `..`, empty names, and NUL.
- Rename uses it, and Move to… uses it too, re-picking a free name if a collision appears.
- `TagDatabase::rename_path` returns errors, which are shown in the toast and logged. Before a move, it clears leftover rows at the destination, so a renamed file can't inherit tags from a vanished file that once had its name.
- `./check.sh` passes, with 140 tests.

**Known limitation (Phase 4 fixes it):** tags are keyed by path. A *new* file that later appears at the exact path of a missing, tagged file inherits that file's tags.

**Next: item 3, bucket edits remove only the bucket's own tags.** Then items 4–6 in order.

**Guardrails:**
- Test in isolated directories only (`docs/reference/development.md`).
- No tagging UX changes; tags are frozen (D1).
- Commit each unit.
