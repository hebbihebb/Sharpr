# Audit 0.5.0-2 — stage 1

Scope: `577defb..bed0278` (26 commits, 27 files), reviewed at
`bed0278b7bdbadc68a7b415b1410a7376c07c042`, on
`codex/audit-0.5.0-2` from `main`.

Stage 1 changed only this report. Stage 2: the user agreed to A01–A05,
then explicitly agreed to A06–A09. All nine fixes are authorized on this
branch; user acceptance, merge and push remain separate.
Approved decisions, including retiring the old migration and D42's automatic
cleanup, are not findings. Neither are existing Backlog requests, item 19,
or the known Compare timeout.

Severity: **High** risks losing usable bucket membership; **Medium** changes
observable behavior or keyboard usability; **Low** is obsolete work/documentation.
“Confirmed” below means established by the cited control/data flow; it does
not imply a new runtime regression test. Race scenarios were not forced in a
live session. Backup recovery remains possible; automatic restoration of
deleted membership does not exist.

## Findings

### A01 — An available ancestor root does not protect an unavailable nested mount

- **Category / severity / evidence:** data preservation / **High** / **Confirmed**.
- **Location:** `sharpr/src/buckets.rs:527`, `sharpr/src/buckets.rs:537`;
  `sharpr/src/model/walk.rs:34`.
- **Scenario:** Home is a root and contains ordinary files. A bucket member's
  stored path is under `Home/Network/drive/`. Unmount that drive. Home remains
  nonempty, the stored file cannot be statted, and no cached copy passes its
  stamp check. `drop_missing` deletes the member. The check only establishes
  availability of the configured ancestor, not the filesystem that held the
  image. The walker deliberately stays on its starting filesystem, so a clean
  pass is not evidence of coverage of a nested filesystem either.
- **Proposed fix:** retain members whose containing filesystem is unavailable
  or was outside positive scan coverage; preserve enough location/device
  provenance to distinguish an unmounted subtree from permanent loss. Keep the
  approved rule against crawling nested mounts. Add nested-mount regressions.
- **Status:** Agreed; implemented and committed as `6f44cba`. Final verification below; user acceptance pending.

### A02 — `failed == 0` can describe an incomplete search for a moved member

- **Category / severity / evidence:** data preservation / **High** / **Confirmed**.
- **Location:** `sharpr/src/ui/window/background_indexing.rs:75`;
  `sharpr/src/fingerprint.rs:115`; `sharpr/src/model/library.rs:853`.
- **Scenario:** the pass collects `a.jpg`, then it is moved outside Sharpr to
  a new visible folder before its metadata read. The metadata failure is
  skipped without incrementing `failed`; the new path was not in the snapshot
  and has no cached fingerprint. The hook can delete the old member before
  the next pass discovers its replacement. A same-scope root-watch request
  queues another pass but does not cancel this one (`fingerprint.rs:319`).
  Directory enumeration, directory-entry and file-type failures are also
  silently skipped. Moving a member into a previously unindexed hidden folder
  is a deterministic coverage gap: the walk intentionally omits the new copy,
  yet cleanup treats it as absent everywhere.
- **Proposed fix:** distinguish complete coverage from skipped/failed work;
  disqualify uncertain passes from destructive cleanup and invalidate absence
  decisions when filesystem changes occur. Define conservative retention for
  moves into excluded/unsearched locations without widening the approved walk.
  Add tests for a move after collection, unreadable subtrees, and hidden moves.
- **Status:** Agreed; implemented and committed as `6f44cba`. Final verification below; user acceptance pending.

### A03 — A scope change cannot cancel deletion once the completion hook starts

- **Category / severity / evidence:** concurrency, data preservation / **High** /
  **Confirmed control-flow race**.
- **Location:** `sharpr/src/ui/window/background_indexing.rs:78`;
  `sharpr/src/fingerprint.rs:272`; `sharpr/src/buckets.rs:568`.
- **Scenario:** a pass enters `pass_done`, then the user disables a folder or
  removes/changes a root while re-linking, trash enumeration, or backup runs.
  The service sets cancellation for the new scope, but checks it only before
  entering the hook. The hook and deletion keep using `job.roots` and
  `job.disabled`; a member newly excluded by current settings can still be
  deleted. This violates the disabled-folder invariant and D41/D42 retention.
- **Proposed fix:** carry a scope generation into cleanup and synchronize
  its validity with the final membership mutation, including after backup;
  cancel stale re-link writes too. Test scope changes during the hook.
- **Status:** Agreed; implemented and committed as `6f44cba`. Final verification below; user acceptance pending.

### A04 — A file inside a trashed directory is not protected

- **Category / severity / evidence:** data preservation / **High** / **Confirmed**.
- **Location:** `sharpr/src/buckets.rs:539`; `sharpr/src/file_ops.rs:153`.
- **Scenario:** a file manager trashes `Pictures/Set`, which contains a bucket
  member `Pictures/Set/a.jpg`. The trash record names `Pictures/Set`, while
  cleanup checks exact set membership for `Pictures/Set/a.jpg`. If Pictures
  still contains something else, the member is dropped even though restoring
  Set brings the image back. Existing tests inject an exact file path and do
  not cover a trashed parent directory.
- **Proposed fix:** recognize descendant files of restorable directory trash
  entries, using component-aware paths and validating the corresponding trash
  payload; test trashing and restoring a directory containing bucket members.
- **Status:** Agreed; implemented and committed as `6f44cba`. Final verification below; user acceptance pending.

### A05 — An unreadable trash is treated as proof that no restore is possible

- **Category / severity / evidence:** data preservation / **High** / **Confirmed**.
- **Location:** `sharpr/src/file_ops.rs:172`, `sharpr/src/file_ops.rs:184`;
  `sharpr/src/ui/window/background_indexing.rs:82`.
- **Scenario:** a relevant trash's `info` directory or a member's `.trashinfo`
  record is temporarily unreadable (permissions or I/O failure). Enumeration
  silently returns fewer entries; its `HashSet` return type cannot report
  uncertainty. Cleanup then drops the member although the trash payload is
  still recoverable. Missing trash directories and unreadable existing ones
  are indistinguishable to the caller.
- **Proposed fix:** return trash coverage/errors and withhold cleanup for
  affected locations on read failure. Distinguish a nonexistent trash from a
  failed read; test unreadable metadata and recovery. Preserve error feedback.
- **Status:** Agreed; implemented and committed as `6f44cba`. Final verification below; user acceptance pending.

### A06 — The next startup upgrade hides unconverted version-1 membership

- **Category / severity / evidence:** migration, data preservation / **High** /
  **Confirmed**.
- **Location:** `sharpr/src/buckets.rs:665`;
  `sharpr/src/library_index/identity.rs:240`;
  `sharpr/src/library_index/mod.rs:1962`;
  `docs/reference/architecture.md:113`.
- **Scenario:** a version-1 bucket has `tag_migrated_at = NULL` and members
  only in `collection_items`. Retiring the old conversion leaves those rows
  intact, but normal startup still runs `buckets::migrate`. It derives the new
  membership exclusively from tags, validates the resulting empty list against
  itself, and stamps version 2. The bucket becomes empty in the app, and later
  startups skip migration. Raw legacy rows and the backup survive; usable
  membership does not carry forward. The new “opens untouched” test only
  reopens the index and never invokes this startup membership upgrade.
- **Proposed fix:** preserve unconverted membership directly during the
  membership upgrade, or refuse that unsafe upgrade with actionable feedback.
  Keep the retired tag-writing conversion retired and retain legacy rows.
  Test the full startup upgrade with an unconverted version-1 fixture. Explain
  the actual support/upgrade outcome in architecture and task documentation;
  saying merely that old rows survive omits the active-membership loss.
- **Status:** Agreed; implemented and committed as `f8509f1`. Final verification below; user acceptance pending.

### A07 — Show More re-sorts the strong block and mixes weak results in early

- **Category / severity / evidence:** search behavior / **Medium** / **Confirmed**.
- **Location:** `sharpr/src/ui/window/search_controller.rs:71`,
  `sharpr/src/ui/window/search_controller.rs:77`.
- **Scenario:** with 401 strong visual matches, page one shows 200. The next
  click selects 400 strong matches, sorts that enlarged block again, and also
  appends up to 200 weak matches while one strong match remains hidden.
  Previously shown strong rows can move. With 201 strong and no weak matches,
  the button still says it adds weaker matches but adds a strong one instead.
  `more` correctly reports remaining rows for this algorithm; the algorithm
  disagrees with the stated fixed-good-block/weak-appending behavior. “Strong
  Matches” receives the displayed good subset, so its set also expands here.
- **Proposed fix:** settle the complete sorted good block before weak
  pagination, or explicitly define separate strong pagination and accurate
  status text with the user. Add a test exceeding PAGE strong matches. Update
  the stale filename-first rule in `docs/reference/ui-gtk.md:20` alongside the
  final rule; architecture currently also promises weaker-only Show More.
- **Status:** Agreed; implemented and committed as `eddd30d`. Final verification below; user acceptance pending.

### A08 — Preferences destroys the focused switch after a root toggle

- **Category / severity / evidence:** keyboard usability / **Medium** /
  **Confirmed removal path; exact resulting focus target not exercised**.
- **Location:** `sharpr/src/ui/preferences.rs:47`.
- **Scenario:** focus a root switch and toggle it with Space. The next
  one-second refresh sees its changed enabled state and removes every root row,
  including the focused control, then builds new widgets without transferring
  focus. Keyboard users lose their place. This contradicts the focused-row
  guard in `docs/reference/ui-gtk.md`. The GTK test toggles switches directly
  and asserts settings, not keyboard focus after the refresh.
- **Proposed fix:** retain root rows and update state in place; when actual
  root removal requires replacement, restore focus to an appropriate control.
  Test a keyboard toggle across the timer refresh.
- **Status:** Agreed; implemented and committed as `6c62560` and `86b6167`. Final verification below; user acceptance pending.

### A09 — Retired perceptual hashes still incur index bookkeeping

- **Category / severity / evidence:** dead work, documentation / **Low** /
  **Confirmed**.
- **Location:** `sharpr/src/library_index/mod.rs:281`,
  `sharpr/src/library_index/mod.rs:380`,
  `sharpr/src/library_index/mod.rs:410`;
  `docs/reference/architecture.md:84`.
- **Scenario:** after removing all perceptual-hash consumers/writers,
  reconciliation still updates `phash`/`phash_status`, and basic upserts still
  execute an extra status SELECT for an unchanged file. Architecture says the
  columns are “no longer written”, which is false. No measured latency
  regression is asserted.
- **Proposed fix:** remove obsolete status reads and conflict-update work,
  preserving the legacy columns and stored values; align documentation.
- **Status:** Agreed; implemented and committed as `59e1f9e`. Final verification below; user acceptance pending.

## Other inspected behavior and limits

- Empty or nonexistent configured roots retain members. Outside-root and
  disabled members are excluded with the supplied job scope; A03 concerns
  that scope becoming stale. Cached copies require matching size and mtime.
- The deletion backup precedes `remove_members`, and its member count is
  checked. No backup is made on a no-op pass. Each dropping pass retains a
  separate full index backup without pruning or deduplication: disk growth is
  unbounded across such passes. D42 explicitly requires these backups and
  specifies no retention policy; this is an operational limit, not a finding
  or permission to delete backups.
- Trash discovery includes the home trash and each configured root's drive
  (`.Trash/<uid>` and `.Trash-<uid>`); it does not enumerate unrelated drives or
  independently discover nested filesystems. Drive-relative `.trashinfo`
  paths are joined to the drive top; percent decoding preserves raw bytes.
  Host-trash selection in Flatpak matches the reported portal experiment.
  That experiment manually restored a file; it did not exercise the app's
  actual Flatpak Undo button end to end. No new Flatpak build was run here.
- Worker extraction preserves the two queues/pools, pool sizing, generation
  checks, stale draining, pending-set cleanup and shutdown. Visible work keeps
  its separate priority lane. No behavioral regression found by comparison.
- `sort_indexed` runs on the folder-open worker; the shared comparator uses a
  full-path final tie-break. Unicode lowercasing allocates per comparison, but
  no measured regression supports a separate finding. No manual large-folder
  rapid-scroll/folder-switch benchmark with before/after logs was recorded
  for the merged worker work or performed in this audit. The navigation GTK
  test proves final folder rows/selection, not throughput or thumbnail latency.
- All three setup extractions were compared with their original blocks:
  bucket-sidebar statements match, the search-chip bar only borrows its header
  parameter, and background indexing's `let ... else` returns only from the
  helper. No lost statement, harmful reordering or borrow-lifetime regression
  found.
- Search limit resets and the sort-aware snapshot key were checked. Without
  strong visual results the closest results retain closeness order, with file
  name matches retained. Strong Matches uses shown file-name and strong visual
  matches, excluding shown weak matches. A07 covers the large strong set.
- Overview counts on a worker, one count per dialog at a time, displays
  gathering errors, and separates unsupported formats. Timers use weak dialog
  references. Closing does not cancel an already-running walk; reopening can
  overlap that finishing walk with a new dialog's count. No permanent worker
  leak established. The full walk every 30 seconds remains unbenchmarked on a
  large library. Traversal errors can still produce partial successful counts
  through the shared walker noted in A02.
- Preferences' switch emits the settings callback even before its sidebar row
  exists; last-root removal is guarded in the UI and settings path.
- The model GTK test's new bucket section only inspects Strong Matches/All
  Shown button labels; it no longer completes the bucket picker and checks the
  resulting memberships. Its small fixture does not cover A07.
- Documentation drift also remains in ROADMAP's duplicate completed Backlog
  entries (15/16/17/18/20 and earlier work), and architecture's setup line
  count/module map. These are recorded here as housekeeping observations, not
  renewed findings against already approved/completed tasks.

## Verification

- `./check.sh` from `sharpr/`: **passed** — formatting, strict all-target/all-
  feature Clippy, 300 nextest tests (43 skipped), cargo-deny and cargo-machete.
  Deny reports non-fatal duplicate-version and unused-license-allowance warnings.
- GTK off-screen suite: **passed** — all 40 tests, 175.674 seconds,
  nextest run `1ab74b48-5c25-4223-a010-39ee57f27324`. The content-search test
  in this run exercised its filename fallback, without model variables.
  Private-session accessibility/portal warnings did not fail the suite.
- Real-model content-search check: **passed**, 25.309 seconds, nextest run
  `9eba875c-0a1b-4cc7-9d51-69d835d7c529`. Ran the same off-screen wrapper
  with `SHARPR_TEST_MODEL_DIR=.tmp/native-benchmark/models` and
  `SHARPR_LLAMA_SERVER=.tmp/native-benchmark/runtime/llama-b11457/llama-server`
  (absolute repository paths), separate `.tmp/audit-0502-model` directories,
  and the existing local model/runtime. It tested actual indexing/ranking,
  the good first page and Show More, not just filename fallback.
- Final `cargo build --jobs 4`: **passed** (12.12 seconds).
- `git diff --check` and staged report whitespace check: **passed**.
- Heavy jobs run sequentially through `systemd-run --user --scope
  -p MemoryMax=9G -p MemorySwapMax=0`, with `CARGO_BUILD_JOBS=4` (and explicit
  `--jobs 4` for GTK nextest), memory GSettings, compiled local schema and
  isolated `.tmp/audit-0502*` data/config/cache/fixtures. The GTK suite uses
  `tools/gtk-offscreen.py`, a private compositor and D-Bus session.

## Stage-3 fix evidence

All nine agreed findings are implemented and committed on the audit branch.
No findings were rejected or deferred. The live library/settings/trash were not
used for trials, and no installed binary was replaced.

- A01–A05: session-only positive observations include every containing directory
  through the configured root, with device/inode identities. Unknown locations,
  excluded coverage, read failures, moved/replaced ancestors, changed directory
  stamps and stale epochs retain members. Source bytes in simulated offline and
  restored trash fixtures are checked. A shared short SQL commit gate defers
  settings changes through GTK timers rather than blocking GTK. Existing backups
  and legacy rows are preserved. Trash error notices are deduplicated.
- A06: union unconverted legacy rows directly with stored effective-tag members;
  verify backup legacy counts, preserve source rows/tags, do not revive converted
  history. Full startup upgrade and repeat-open cases are covered.
- A07: 401 strong matches plus a filename match are shown/sorted together and
  remain fixed while weak matches append; no Show More when only strong remain.
- A08: the same root switch retains toplevel focus through timer and sidebar
  changes. The headless test checks `is_focus()` and the root's focus widget;
  inactive off-screen windows do not have desktop-wide `has_focus()`.
- A09: changed and unchanged basic upserts/reconciliation preserve prior
  perceptual hash values/status while metadata invalidation still works.

Red evidence before fixes: A01/A02 unknown-location deletion and failed-walk
coverage; A05 unreadable record incorrectly claimed absence; A06 startup lost
an unconverted member (isolated copy of `bed0278` plus the new test while the
main checkout's APIs were being integrated); A07 showed 201 instead of 402
strong/name results; A08 replaced the focused switch; A09 replaced legacy hash
values/status. During the one independent review, a same-filesystem nested
location case was reproduced red and fixed with ancestor provenance. Staged
transaction rollback, stale backup completion, mid-pass moves, cached copies,
empty/missing/removed/disabled roots and trashed parents have positive tests.

Final `./check.sh`: passed, 315 tests (44 skipped), run
`fc2f7237-f712-4c56-b51c-b49dded4655d`; formatting, strict Clippy, deny and machete
also passed. The complete off-screen GTK run `9aca0e5c-7929-489c-bbb6-8cd702eab181`
ran 41 workflows: 40 passed, one failed on a rapid switch/sidebar synchronization
case. Removed the stale snapshot fast path (`86b6167`); its regression then
passed in `b9b7ccc0-8176-4cb7-ab8f-db554176cc71` (4.518 seconds). The same run
passed real-model content search (23.944 seconds), using the existing local
model/runtime and isolated `.tmp/audit-0502-final-model` data through the
private compositor. No unresolved GTK failure remains; a second full 41-test
suite was not run after this three-line UI-only correction. Formatting and
whitespace checks passed again. `cargo build --jobs 4` passed in 11.38 seconds under the same memory cap.

Limits: session-only provenance intentionally retains already-missing members
after a restart. Skipped hidden/disabled/symlink/other-filesystem locations
withhold cleanup rather than being crawled. External filesystem changes cannot
be committed atomically with SQLite; final metadata rechecks and prompt event
invalidation reduce that race but do not lock arbitrary external writers.
No manual release-build scroll benchmark or new Flatpak Undo trial was added.

Final verification is complete. Remove this report in its own commit as AGENTS.md
requires. Its stage-1 findings, review decisions and fix evidence remain in git.
Acceptance, merge, deployment and push remain pending.
