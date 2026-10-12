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

**Item 3: done (committed, verified, not yet deployed).** The bucket edits now live in the GTK-free `src/buckets.rs`, with tests.
- **Remove:** strips only the bucket's primary tag, so the image leaves this bucket and its sub-buckets but stays in its parent and siblings.
- **Rename and reparent:** cover every stored member under the library root, including disabled folders and missing files.
- **Reparent:** keeps any tag a sibling or unrelated bucket still needs.
- **Verification:** `./check.sh` passes (144 tests), and both GTK tests pass.
- **Known interim limit:** with tag-backed buckets, a direct member of an old ancestor is indistinguishable from a member of the moved bucket, so it leaves the ancestor on reparent. Phase 4's explicit membership fixes this.

**Item 4: done (committed, verified, not yet deployed).**
- Hash follow-ups use a non-blocking send (`queue_hash`). When the queue is full, the hash is skipped and logged as `hash.skipped_queue_full`.
- One shared `drain_stale` helper, used by both worker loops, drops stale thumbnails, puts current hash jobs back on the queue, and honors Shutdown instead of swallowing it. Scheduling order and priorities are unchanged.
- **Stress test:** `overfilled_queue_never_stalls_workers_or_leaves_paths_pending`, with a 2-slot queue and 64 requests. It was confirmed to fail on the old blocking send ("workers stalled after 1 of 64 results").
- **Verification:** `./check.sh` (146 tests) and both GTK tests pass.
- **Pending:** the manual rapid-scroll and folder-switch check on the real library happens at Phase 1 deployment, compared against the baseline.

**Item 5: done (committed, verified, not yet deployed).**
- `src/db_version.rs` stamps `PRAGMA user_version = 1` on both databases.
- Before any setup, each database is opened read-only and refused untouched if it is newer than the build supports.
- A tags-database failure now shows a persistent toast.
- **Tests:** a refused database file is byte-identical afterwards.
- **Round-trip:** a copy of the live databases went through the new build with identical tag rows, buckets, and image list (SHA-256), and is now stamped version 1.
- **Dead code:** 1,304 lines of `#[cfg(any())]` pipeline and upscaler code were deleted; the legacy tables and keys remain.
- `./check.sh` passes, with 150 tests.

**Compatibility note:** the deployed build `5832010` ignores `user_version`, so going back to it after deploying Phase 1 is safe.

**Next: item 6, guard broad actions.**
- Tags-page "Delete tag" confirms and names affected buckets, and runs off the GTK thread.
- Reject bucket names that differ only by case.
- Single-key shortcuts don't fire while a text entry has focus.
- Report trash failures.

**Guardrails:**
- Test in isolated directories only (`docs/reference/development.md`).
- No tagging UX changes; tags are frozen (D1).
- Commit each unit.
