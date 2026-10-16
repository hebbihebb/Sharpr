# Sharpr 0.5.0 audit

Stage 2 complete: the user agreed to fix all 13 findings, in severity order,
with one commit per finding. Stage 3 is active.
Branch: `codex/audit-0.5.0`, based on `main` at `0d81115`.
Do not merge or push this branch. Claude handles merging after acceptance.

## Scope and verification

Read `AGENTS.md`, `CURRENT_TASKS.md`, `ROADMAP.md` including Backlog,
`product.md` including D1–D35, `interface-inventory.md`, `architecture.md`,
`development.md`, `ui-gtk.md`, the bundled manual and README. Traced output,
rename/move/trash, persistence, bucket readers/writers, scope filtering,
asynchronous view updates, managed inference, active UI and retired wiring.
Approved decisions and existing Backlog items are excluded from the findings.

- `cargo build --offline --locked --jobs 2`: passed in `sharpr/`.
- `cargo nextest run --offline --locked --build-jobs 2 --test-threads 2`:
  the sandbox blocked a test's loopback listener after 204 passes. The approved
  rerun with `--no-fail-fast` passed **220/220**, with **23 skipped** (the default
  ignored GTK/model checks). This was an environment restriction, not a code defect.
- Three observational tests in a temporary source copy reproduced A1, A2 and
  A3 using the real application helpers and temporary files/databases.
- A focused GTK probe on `tools/gtk-offscreen.py`'s private compositor passed,
  reproducing A6, A7 and A8. GTK also reported the CSS parse error in A10.
  Initial probe attempts required correcting fixture layout and two assumptions
  about starting sidebar visibility and GTK's built-in clipboard shortcut.
- Probe sources and fixtures are local at `/tmp/sharpr-audit-050/`, outside the
  tracked source. Pure probe names are `audit_probes::audit_copy_can_overwrite_and_follow_a_dangling_link`,
  `audit_probes::audit_internal_rename_can_lose_bucket_membership`, and
  `audit_probes::audit_relink_and_copy_count_ignore_exclusions`; the GTK probe
  is `ui::window::gtk_tests::audit_ui_paths_and_styles`.
- All runs used memory GSettings, isolated XDG data/config/cache directories,
  offline Cargo and at most one heavy job. No real Sharpr data or Pictures
  folders were used; no external network requests, deployment or Flatpak build.
  The existing unit tests exercise localhost HTTP with a synthetic server.

## Findings

### A1 — Ordinary copies do not reserve their destination

- Category: bug
- Severity: high
- File: `sharpr/src/file_ops.rs:270`; also `sharpr/src/ui/window.rs:3834`.
- Issue: Duplicate and Copy to Folder check a name with `exists()` and then call overwriting `std::fs::copy`, instead of exclusively creating a new output.
- Scenario: choose a free copy name → another program creates a file at that name before copying → Sharpr truncates and replaces that file; alternatively a dangling symlink occupies the candidate name → Duplicate follows it and creates its target, treating an occupied name as free.
- Evidence: confirmed by the temporary copy probe, including the actual `duplicate_file` helper; crop/export's safer `Pending` path is not affected.
- Proposed fix: use a shared exclusive copy helper with collision retries and owned-partial cleanup; treat symlinks as occupied names. Keep originals and pre-existing targets byte-identical in regression tests.
- Verification: regression failed before the fix; 223 default tests, fmt, strict clippy, offline deny and machete pass. Copy tests cover dangling links, concurrent destinations, source bytes/permissions and partial cleanup.
- Status: fixed (cd32145)

### A2 — Internal Rename/Move can transfer a bucket to an unrelated replacement

- Category: bug
- Severity: high
- File: `sharpr/src/ui/window.rs:4030`; also `:3920`, `sharpr/src/buckets.rs:422`.
- Issue: Rename and Move update legacy tags but leave the bucket member's old path and the destination fingerprint cache unchanged.
- Scenario: add `a.png` to a bucket → rename it inside Sharpr to `b.png` → put an unrelated image at `a.png` → the next fingerprint pass replaces the stored member fingerprint with the new `a.png`'s fingerprint → `b.png` loses its membership and the unrelated image gains it.
- Evidence: confirmed by the temporary rename probe using the same file/tag helpers as the UI, followed by the real fingerprint and re-link pass; the legacy tag did follow `b.png`. This is distinct from the Backlog's delayed discovery of moves made outside Sharpr.
- Proposed fix: carry bucket member paths and fresh fingerprints through successful internal moves/renames alongside legacy tags; protect retained destination memberships and report persistence failures. Test filename reuse as well as normal Rename and Move.
- Verification: the real Rename-dialog regression failed before the fix and passes afterward. All 227 default tests and full offline checks pass, including moves with/without cached fingerprints, multiple buckets, retained destinations, filesystem collisions and injected tag-write rollback.
- Status: fixed (dcc9cbf)

### A3 — Re-linking and Info copy counts bypass library exclusions

- Category: bug
- Severity: medium
- File: `sharpr/src/buckets.rs:404`; `sharpr/src/copies.rs:271`; caller `sharpr/src/ui/window.rs:1431`.
- Issue: These fingerprint consumers receive no root/disabled-folder scope, so they probe excluded files and re-linking can write member paths into disabled or outside-root folders.
- Scenario: index two identical images, then disable the second image's folder and remove the first → an enabled-library fingerprint pass invokes unscoped re-linking → the stored member moves into the disabled folder; before removal, Info still counts that disabled copy. Narrowing the root leaves the same unscoped behavior for cached outside-root copies.
- Evidence: confirmed by code trace and a temporary fixture: the scoped fingerprint pass skipped the disabled subtree, but `relink` still changed `last_path` to its file; `identical_elsewhere` has no exclusion input. Bucket display resolution itself correctly filters exclusions before probes.
- Proposed fix: pass the current library scope into re-linking and copy-count queries, filter before filesystem probes or writes, and leave excluded members stored untouched. Add disabled-folder and narrowed-root regressions.
- Verification: the excluded-candidate regression failed before the fix. All 229 default tests and full offline checks pass; scoped tests cover disabled/outside-root candidates, unchanged excluded member rows, missing counts and Info counts.
- Status: fixed (ff4846f)

### A4 — The clipboard menu decodes full images on the GTK thread

- Category: bug
- Severity: medium
- File: `sharpr/src/ui/filmstrip.rs:2209`; action callback at `:989`.
- Issue: Copy to Clipboard synchronously loads and decodes the source through `gdk4::Texture::from_file` in the menu activation callback.
- Scenario: right-click a large image → Copy to Clipboard → the GTK event loop waits for full file reading/decoding, so navigation and window interaction stall; load failures are silently ignored.
- Evidence: confirmed call path; stall duration was not benchmarked. This is the image clipboard decoder, separate from the Backlog's ordinary file copy/move/trash work.
- Proposed fix: prepare clipboard image data on a bounded decoding worker, publish it on GTK, and report failures. Reuse existing orientation/format handling and test original bytes plus main-loop responsiveness.
- Verification: the clipboard error UI regression failed before the fix; success/error UI and the 5,000-image navigation workflow pass off-screen. All 231 default tests and full offline checks pass. A gated worker test proves nonblocking requests and superseded-request coalescing; an EXIF fixture checks orientation and original bytes. Debug/Cairo before/after logs: `/tmp/sharpr-audit-050/a4-{before,after}.jsonl`; first-folder load 314/281 ms, browsing stalls ≥50 ms 0/0, peak RSS 164/167 MB. These synthetic debug checks do not establish real-library release performance or user acceptance.
- Status: fixed (e7c6e0a)

### A5 — The bucket-membership upgrade blocks window construction

- Category: bug
- Severity: medium
- File: `sharpr/src/ui/state.rs:83`; `sharpr/src/buckets.rs:483`; construction at `sharpr/src/ui/window.rs:606`.
- Issue: The Phase 4 membership migration performs database backups, verification, per-member file probes and bulk membership work synchronously while constructing GTK window state.
- Scenario: launch with an unmigrated database containing many tag-backed bucket members → the first window cannot finish constructing or process UI events until the full backup and migration finishes.
- Evidence: confirmed call path from application activation through `SharprWindow` construction to `AppState::new`; no large-upgrade timing was attempted. The backup/count checks themselves are present. This is separate from the Backlog's unguarded *legacy collection-to-tag* migration.
- Proposed fix: run this upgrade on a worker and show an initializing state, keeping bucket edits disabled until success. Preserve the existing backup, count validation and failure behavior.
- Verification: the construction/backup regression failed first; successful and failed upgrades preserve image bytes, tags and definitions, verify backups, and gate bucket controls. Full offline checks (231 default tests) and all 24 isolated GTK workflows pass. GTK trash tests require same-filesystem workspace `.tmp` fixtures; an initial `/tmp` run failed for that environment reason. A5 debug/Cairo navigation log `/tmp/sharpr-audit-050/a5-after.jsonl`: 5,000-image open 300 ms (prior 281 ms), peak RSS 171 MB (167), one 126 ms browsing stall during the rapid synthetic navigation loop (prior zero). Release/manual performance is not claimed.
- Status: fixed (9301db7)

### A6 — A late bucket update erases a newer search

- Category: bug
- Severity: medium
- File: `sharpr/src/ui/window/tag_controller.rs:213`; related refreshes at `:126` and `sharpr/src/ui/collection_dialogs.rs:301`.
- Issue: Bucket mutation completion checks the bucket scope but not the view/search generation, then clears filters and installs its older full-bucket snapshot.
- Scenario: while viewing a bucket, start an Add/Remove operation → type a new search before its worker's completion is dispatched → completion sees the same bucket ID, clears the new search and replaces the newer view with the full bucket.
- Evidence: confirmed in the off-screen GTK probe by delaying main-context delivery of a bucket-write result until a newer query was submitted; the query text was then cleared. Similar scope-only refreshes exist after reparent and rename.
- Proposed fix: validate the captured view/search generation before applying a refresh; when search has changed, refresh or rerun the current query without clearing it. Test delayed completion against a newer query and navigation.
- Status: fixed (1215b16)

- Fix: shared background membership refresh retires older query results, updates the saved bucket membership, and reruns the latest search/filter inputs while preserving selection and scroll. Rename, reparent, membership writes and edit buckets use it.
- Regression: failed first when the late completion cleared `a`; passes with the newer query retained, removed members absent after clearing search, and newer folder navigation preserved. Offline `./check.sh` passes (231 default tests). All 25 isolated offscreen GTK workflows pass. Synthetic 5,000-image navigation: 352 ms first-folder readiness, one 85 ms browsing stall, peak RSS 172 MB (debug/Cairo); prior A5 sample: 300 ms, one 126 ms stall, 171 MB. These samples are not a release/manual performance claim.

### A7 — F11 and double-click enter different fullscreen modes

- Category: bug
- Severity: medium
- File: `sharpr/src/ui/window.rs:3513`; shared presentation implementation at `:3400`.
- Issue: F11 directly toggles the window's fullscreen flag, whereas double-click uses presentation mode; Escape only exits the latter.
- Scenario: press F11 while browsing → the window becomes fullscreen without entering presentation mode → close Info if needed and press Escape → it stays fullscreen, unlike fullscreen entered by double-click.
- Evidence: confirmed in the off-screen GTK probe: `is_fullscreen()` was true while `presentation_mode` was false, and Escape did not exit; the shared presentation path did hide the sidebar. No proposal here changes D11's approved fullscreen feature.
- Proposed fix: route fullscreen entry/exit through one mode-aware action, preserving the Compare/crop header and Escape's Info-first behavior. Test both entry points and mode transitions.
- Status: fixed (f73623d)

- Verification: regression failed first on F11/presentation mismatch; passes with shared action, hidden panels, Info-first Escape, panel restoration, and Compare/crop controls. Existing mode-header and small-image/fullscreen resize workflows also pass.

### A8 — Bucket/file editing dialogs miss the shared sheet style

- Category: interface
- Severity: low
- File: `sharpr/src/ui/collection_dialogs.rs:152`, `:241`, `:351`; `sharpr/src/ui/window.rs:3990`.
- Issue: New Bucket, Rename Bucket, Change Bucket Color and Rename File construct unstyled `AlertDialog`s while the shared compact rules require the `sharpr-sheet` class.
- Scenario: open New Bucket, then Preferences or Export → the first dialog bypasses the compact dialog rules applied to the latter surfaces, contrary to the shared-style inventory/D26–D27.
- Evidence: confirmed by source and the off-screen probe inspecting the presented New Bucket dialog; it has no `sharpr-sheet` class. No replacement of confirmation semantics is proposed.
- Proposed fix: apply the existing shared dialog styling consistently to these editing surfaces and check minimum-size layout, keyboard dismissal and focus.
- Status: fixed (262f34a)

- Verification: shared class applied to all four named dialogs, picker New Bucket and Move into Bucket. Existing GTK tour and Rename data-preservation workflow pass; refreshed New Bucket screenshot inspected with unclipped fields, palette and action. Native response/focus handling is retained; build and diff check pass.

### A9 — Active bucket notifications still call buckets “collections”

- Category: interface
- Severity: low
- File: `sharpr/src/ui/collection_dialogs.rs:207`, `:326`, `:398`; `sharpr/src/ui/window/tag_controller.rs:137`, `:226`.
- Issue: User-facing success/error text retains “collection” after the interface standardized on “bucket.”
- Scenario: create a bucket → the dialog says New Bucket but its notification says “Created collection”; reparenting or failed bucket edits also use the retired term.
- Evidence: confirmed active string paths, independent of internal Rust/SQL names, which may legitimately retain `collection`.
- Proposed fix: change active user-facing strings to “bucket” consistently; keep persistence names intact.
- Status: fixed (5c79227)

- Verification: active notification/error/confirmation strings traced and changed, including surfaced validation errors. Internal actions, SQL and log event names retained. Existing repeat-bucket/Info and edit-bucket GTK workflows pass; build and diff check pass.

### A10 — GTK rejects the thumbnail placeholder CSS

- Category: interface
- Severity: low
- File: `sharpr/data/style.css:390`.
- Issue: The placeholder rule uses the unsupported `:empty` pseudo-class, so GTK rejects the selector instead of styling loading thumbnails as promised by D33 and the inventory.
- Scenario: open a folder, Survey or Compare before its thumbnails finish → GTK emits “Unknown name of pseudo-class” and the intended placeholder rule is discarded.
- Evidence: confirmed parser warning during the private GTK probe, with the selector and location reported above.
- Proposed fix: use a supported explicit loading class/state when binding/updating pictures, and validate CSS without parser errors. Leave scheduling and generation logic unchanged.
- Status: fixed (c72fa97)

- Verification: GTK parser test failed first with the unsupported pseudo-class and now passes without errors. Four GTK checks pass (parser, Compare candidates, Survey, rapid folder/selection changes). Paintable notifications update the shared loading class on initial bind, completion and reuse; scheduler unchanged. Full offline checks pass (231 default tests). Synthetic 5,000-image navigation: 338 ms first-folder readiness, one 82 ms browsing stall, peak RSS 170 MB; A6 baseline: 352 ms, 85 ms, 172 MB (debug/Cairo samples, not release performance).

### A11 — Retired feature implementations and wiring remain compiled

- Category: dead code
- Severity: low
- File: `sharpr/src/ui/window.rs:1213`, `:1274`, `:3115`; `sharpr/src/collection_export.rs:45`; `sharpr/src/ui/filmstrip.rs:480`; `sharpr/src/model/library.rs:632`.
- Issue: Removed bucket import/export handlers, the unparented tag-search/save-search widgets and callbacks, and old quality-view query helpers remain after their UI/features were retired.
- Scenario: a maintainer follows these apparently supported handlers while changing bucket/search behavior → they must maintain obsolete paths that the current UI cannot invoke, and the hidden search still describes “Search tags…”.
- Evidence: confirmed reference tracing: import/export handlers have no callers; the old filmstrip SearchBar is never appended to the visible layout or activated by a caller; quality-view helpers have only internal/test references. The active viewer search has its own implementation.
- Proposed fix: remove these retired implementations and exclusive wiring/tests, retaining shared metadata/filter helpers and every legacy table/settings key. Do not include the perceptual-hash cleanup already in Backlog.
- Status: fixed (ab3e01f)

- Verification: retired handlers/module, unparented widgets/callbacks and quality-view helpers have no remaining source references. Shared cancellable scan/metadata/scoring helpers and all persistence definitions retained; the exclusion regression now calls the active scanner. Full offline checks pass (229 tests; two tests exclusive to removed quality views retired); four active GTK search/bucket/navigation workflows pass. Synthetic 5,000-image navigation: 332 ms, one 51 ms browsing stall, peak RSS 167 MB vs A10 338 ms/82 ms/170 MB (debug/Cairo). Build and diff check pass.

### A12 — The guide and setup reference send users to removed menu locations

- Category: docs
- Severity: low
- File: `sharpr/data/manual.md:47`; `docs/reference/development.md:126`.
- Issue: The guide locates tag maintenance under Library and development instructions locate visual-search status in Preferences, although these moved to Help and Library Overview.
- Scenario: follow the guide to Library → Turn Tags into Buckets or Remove Old Automatic Tags → neither item is there; follow model setup instructions to Preferences → Visual Search → the documented status section is absent.
- Evidence: confirmed against `ui/help_window.rs:29`, `ui/preferences.rs` and `ui/library_overview.rs`; the manual's own final paragraph already describes the newer Help location.
- Proposed fix: point both maintenance actions to Help → Library Maintenance and model/status instructions to Library → Library Overview; keep Preferences' GPU switch location.
- Status: fixed (463514c)

- Verification: paths checked against Help’s Library Maintenance rows, Library Overview’s model/status section and Preferences’ GPU switch. Corrected the matching stale AGENTS menu pointer. `git diff --check` passes.

### A13 — The architecture reference misclassifies user data and describes obsolete packaging

- Category: docs
- Severity: medium
- File: `docs/reference/architecture.md:81`, `:122`, `:123`; also `:48`.
- Issue: The storage table omits `bucket_members` and calls everything except bucket definitions rebuildable, while packaging/version text still describes working-tree rsync and commit-count versions, and progress text still promises the removed header ring.
- Scenario: use the storage table to decide which index records may be rebuilt → bucket member lists appear disposable even though they are irreplaceable; follow the packaging description → it contradicts the actual committed-source build and Cargo version source.
- Evidence: confirmed against `library_index/schema.rs:71`, the same reference's membership section, `packaging/sync-flatpak-source.sh`, `app.rs:12`, `Cargo.toml:3` and the current interface inventory.
- Proposed fix: explicitly classify bucket membership as non-rebuildable user data; correct packaging/version and indexing-status descriptions to the shipped implementation. No data migration is involved.
- Status: fixed (effc5df)

- Verification: storage wording checked against `library_index/schema.rs` and bucket migration; packaging/version wording against the sync script, `app.rs`, `build.rs`, Cargo manifest and metainfo regression; progress wording against Library Overview’s one-second timer. `git diff --check` passes.

## Areas checked with no additional findings

- Crop and batch WebP output: exclusive partial creation, no-replace publication,
  collision handling, cancellation/partial cleanup and original-byte preservation
  are covered by passing tests and source tracing.
- Rename/Move filesystem primitive: no-replace rename and occupied symlink
  rejection are tested; A2 concerns the separate bucket persistence path.
- Schema safety: additive setup and newer-schema refusal; Phase 4 migration
  backs up, verifies counts and preserves legacy tags. The already-listed legacy
  migration guard remains Backlog; A5 concerns execution on the GTK thread.
- Bucket inheritance, child removal, migration counts, hidden/missing display
  rules, and one-root identity/settings preservation have passing regressions.
- Viewer/metadata, search, Compare and Survey have stale-result checks at their
  principal completion points; A6 and the existing thumbnail/folder Backlog
  exceptions are kept separate.
- Trash is reached through explicit actions; filmstrip immediate trash/Undo is
  approved D25, and Find Copies retains confirmation and changed-file checks.
- Managed inference binds/connects only to `127.0.0.1`, uses a random key,
  passes `--offline`, strips inherited remote-model settings and has no downloader.
- No reachable Tasks/pipeline, generated-tag, upscaling or remote-inference UI
  was found. The ordinary transient operation indicator is still intentional.

## Final fix verification

All 13 agreed findings have separate fix commits named above. No findings were
rejected or deferred, so there are no deferred additions to Backlog. The final
isolated offscreen GTK suite passes all 27 workflows, including the new CSS
parser, fullscreen, stale-search, migration, clipboard and rename regressions.
The latest full offline checks pass all 229 default tests (the two retired
quality-view tests were removed with their exclusive implementation).
No real user data was accessed, no Flatpak was built, and nothing was merged,
pushed or deployed. Delete this report in the audit cleanup commit, then run
the required final offline check and build.
