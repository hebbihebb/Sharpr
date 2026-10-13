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

**Item 6: done (committed, verified, not yet deployed).**
- Delete tag on the Tags page asks first, names every bucket that would lose its images (inherited ones included), and deletes on a worker thread, with errors in a toast.
- Bucket names that differ only by case are rejected with a readable message.
- Delete, F2, Ctrl+T, `[` and `]` never fire while a text field has focus (`text_input_has_focus`).
- Both trash paths share `trash_paths`, which reports files that could not be trashed.
- **Verification:** `./check.sh` (152 tests) and both GTK tests pass.

**Phase 1 is implemented and verified, but not deployed.** On October 12 the user chose to defer deployment and continue. Their manual check runs at the next deployment.

**Manual check plan:**
1. Scroll the filmstrip fast and switch between large folders.
2. Press Delete in the search box; no images may be trashed.
3. Rename an image onto an existing name; it must be refused.
4. Open Delete Tag and cancel; the affected buckets must be listed.
5. All 6 buckets must be intact.

## Phase 2 Progress

**Item 1: done (committed, verified, not yet deployed).**
- A main-loop stall probe in `bench.rs` logs `mainloop.stall` with `late_ms`.
- `tools/bench-summary.py` summarizes a log and reproduces the Phase 0 baseline exactly. It reports startup stalls and browsing stalls separately.
- A first debug-build run showed startup stalls of 639 ms and 487 ms, and one 222 ms stall while loading the first folder. Release builds will be measured at deployment.

**Item 2: done (committed, verified, not yet deployed), except suggestions while typing.**
- `model/search_query.rs` parses formats, resolutions (`4k` means at least 3840×2160 in either orientation; `below 4k` or `<4k` mean the opposite), and aspects (`16:9`, `21:9`, `16:10`, `4:3`, `3:2`, square, landscape, portrait). A quoted word stays text.
- A filter word moves into a chip once followed by a space. A recognized word still being typed already filters.
- The chips sit in a wrapping bar under the header, so they never widen the window past 1024 px. The first version did, and the GTK test caught it with Adwaita's width warning.
- **Tests:** 6 unit tests, plus a GTK scenario covering typing `png 16:9 `, removing a chip, and closing search.
- **Screenshot:** checked at 1024×600.
- `./check.sh` passes, with 158 tests.

**Item 3: done (committed, not yet deployed).**
- Type-to-search and Ctrl+F already worked, and the "Whole library" switch is visible.
- Space, `[` and `]` no longer open search: Space stays free for viewing and Compare (D7), and the brackets keep cycling pages.
- **Manual check at deployment:** press Space and the bracket keys while browsing.

**Item 8, user findings: two of four done (committed, not yet deployed).**
- **Filter popover:** the aspect drop-down nested a second popover, which broke click-away closing, so it is now radio buttons. Manual check at deployment: click outside the Filter popover.
- **Indicator:** the background-activity indicator is hidden when idle and slides in for work lasting over 0.4 s.
- **2.3 s folder wait: fixed.** Every perceptual hash was saved by its own job in the shared rayon pool, about 7,900 per session, and folder scans queued behind them. Hashes are now batched on one writer thread (`spawn_phash_writer`, one transaction per batch), and folder scans run on their own thread. Measure at deployment with `tools/bench-summary.py`.
- **Thumbnail memory: analysed, not changed.** It is the cost of whole-folder preload plus sharp 2× thumbnails, not a leak. The decision is the user's, after measuring the deployed build (see `ROADMAP.md`).

**Item 7: implemented and verified, not yet run (the user runs it after deployment).**
- **Entry point:** main menu → Library → "Remove Old Automatic Tags…".
- **What it removes:** tags the old auto-tagger produced, meaning the file's own extension, or 720p, 1080p, or 4k. A format tag goes only where it equals that file's real extension, and tags used by any bucket are never touched.
- **Process:** it previews counts and asks first. It backs up with `VACUUM INTO` to `~/.local/share/sharpr/backups/tags-before-automatic-cleanup-<time>.sqlite3`, then deletes in one transaction.
- **Preview on a copy of the live data:**
  - 7,346 rows would be removed: jpg 2,817, 4k 2,048, 1080p 1,342, jxl 462, 720p 248, png 209, jpeg 128, webp 92.
  - 372 manual rows would be kept.
  - 18 `png` tags on non-PNG files are conservatively kept.
- The dead `tags/indexer.rs` and the test-only `insert_auto_tags` were deleted.

**Item 5: done (committed, not yet deployed).**
- Ctrl+B opens the bucket picker for the current selection; the picker can also create a new bucket.
- Dragging a selection onto a sidebar bucket already worked.
- The shortcuts window and manual are updated. The manual no longer claims Sharpr adds format tags.
- **Manual check at deployment:** select several images, press Ctrl+B, and pick a bucket.

**Item 6: done (committed, not yet deployed).**
- Quality views are retired: the Quality Filter section of the sort menu, the unreachable `scan-quality` action, `ViewScope::Quality`, and the unused `ui/filter_bar.rs`.
- Typed `below 4k` and `4k` replace them, and the manual says so.
- The viewer's resolution chip (the `quality` scorer) stays.

**Item 11, main-thread work: done (committed, not yet deployed).**
- **Filmstrip badge colours:** binding a row no longer queries SQLite. A worker builds a path → colour map whenever bucket colours change, a viewer tag edit lands, or a tag is deleted. Until the first map is ready, binding falls back to the old query.
- **Manual check at deployment:** the coloured position badges of bucket members still appear, and update after adding an image to a bucket.
- **Duplicate groups:** they are reused until the stored hash list changes, checked by a fingerprint of the list. Changing images costs one linear query instead of a regrouping that compares every pair of images.
- **Bucket open:** the member query and per-file checks run on a worker using the checked query, so a database error shows a toast instead of an empty bucket. Stale results are dropped by a generation counter and a scope check. A GTK step now opens a bucket the way a sidebar click does.
- **Virtual-view lookups:** `entries_for_path` uses the path index in bucket and search views, so each applied thumbnail is O(1) instead of a scan of the whole view. The scan remains only for a view that lists a path twice, which is flagged when the view loads.
- **Sidebar bucket counts:** already ran on a worker with a generation guard in the October 12 work. No change.
- **Whole-library search:** the library file list is cached for one search session, keyed by session, root and disabled folders, so typing no longer re-walks the folder tree per keystroke. A cancelled walk is never cached, and stale entries are dropped by the per-file disk check.

**Found:** at 1024×600 the viewer header overflows, and the window close button is partly off-screen. This predates the chip work; it is tracked under item 4.

**Guardrails:**
- Test in isolated directories only (`docs/reference/development.md`).
- No tagging UX changes; tags are frozen (D1).
- Commit each unit.
