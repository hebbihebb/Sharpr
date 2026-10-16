# Sharpr 0.5.0 audit

Stage 1 complete; Stage 2 decisions pending. No application code has changed.
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
- Status: proposed

### A2 — Internal Rename/Move can transfer a bucket to an unrelated replacement

- Category: bug
- Severity: high
- File: `sharpr/src/ui/window.rs:4030`; also `:3920`, `sharpr/src/buckets.rs:422`.
- Issue: Rename and Move update legacy tags but leave the bucket member's old path and the destination fingerprint cache unchanged.
- Scenario: add `a.png` to a bucket → rename it inside Sharpr to `b.png` → put an unrelated image at `a.png` → the next fingerprint pass replaces the stored member fingerprint with the new `a.png`'s fingerprint → `b.png` loses its membership and the unrelated image gains it.
- Evidence: confirmed by the temporary rename probe using the same file/tag helpers as the UI, followed by the real fingerprint and re-link pass; the legacy tag did follow `b.png`. This is distinct from the Backlog's delayed discovery of moves made outside Sharpr.
- Proposed fix: carry bucket member paths and fresh fingerprints through successful internal moves/renames alongside legacy tags; protect retained destination memberships and report persistence failures. Test filename reuse as well as normal Rename and Move.
- Status: proposed

### A3 — Re-linking and Info copy counts bypass library exclusions

- Category: bug
- Severity: medium
- File: `sharpr/src/buckets.rs:404`; `sharpr/src/copies.rs:271`; caller `sharpr/src/ui/window.rs:1431`.
- Issue: These fingerprint consumers receive no root/disabled-folder scope, so they probe excluded files and re-linking can write member paths into disabled or outside-root folders.
- Scenario: index two identical images, then disable the second image's folder and remove the first → an enabled-library fingerprint pass invokes unscoped re-linking → the stored member moves into the disabled folder; before removal, Info still counts that disabled copy. Narrowing the root leaves the same unscoped behavior for cached outside-root copies.
- Evidence: confirmed by code trace and a temporary fixture: the scoped fingerprint pass skipped the disabled subtree, but `relink` still changed `last_path` to its file; `identical_elsewhere` has no exclusion input. Bucket display resolution itself correctly filters exclusions before probes.
- Proposed fix: pass the current library scope into re-linking and copy-count queries, filter before filesystem probes or writes, and leave excluded members stored untouched. Add disabled-folder and narrowed-root regressions.
- Status: proposed

### A4 — The clipboard menu decodes full images on the GTK thread

- Category: bug
- Severity: medium
- File: `sharpr/src/ui/filmstrip.rs:2209`; action callback at `:989`.
- Issue: Copy to Clipboard synchronously loads and decodes the source through `gdk4::Texture::from_file` in the menu activation callback.
- Scenario: right-click a large image → Copy to Clipboard → the GTK event loop waits for full file reading/decoding, so navigation and window interaction stall; load failures are silently ignored.
- Evidence: confirmed call path; stall duration was not benchmarked. This is the image clipboard decoder, separate from the Backlog's ordinary file copy/move/trash work.
- Proposed fix: prepare clipboard image data on a bounded decoding worker, publish it on GTK, and report failures. Reuse existing orientation/format handling and test original bytes plus main-loop responsiveness.
- Status: proposed

### A5 — The bucket-membership upgrade blocks window construction

- Category: bug
- Severity: medium
- File: `sharpr/src/ui/state.rs:83`; `sharpr/src/buckets.rs:483`; construction at `sharpr/src/ui/window.rs:606`.
- Issue: The Phase 4 membership migration performs database backups, verification, per-member file probes and bulk membership work synchronously while constructing GTK window state.
- Scenario: launch with an unmigrated database containing many tag-backed bucket members → the first window cannot finish constructing or process UI events until the full backup and migration finishes.
- Evidence: confirmed call path from application activation through `SharprWindow` construction to `AppState::new`; no large-upgrade timing was attempted. The backup/count checks themselves are present. This is separate from the Backlog's unguarded *legacy collection-to-tag* migration.
- Proposed fix: run this upgrade on a worker and show an initializing state, keeping bucket edits disabled until success. Preserve the existing backup, count validation and failure behavior.
- Status: proposed

### A6 — A late bucket update erases a newer search

- Category: bug
- Severity: medium
- File: `sharpr/src/ui/window/tag_controller.rs:213`; related refreshes at `:126` and `sharpr/src/ui/collection_dialogs.rs:301`.
- Issue: Bucket mutation completion checks the bucket scope but not the view/search generation, then clears filters and installs its older full-bucket snapshot.
- Scenario: while viewing a bucket, start an Add/Remove operation → type a new search before its worker's completion is dispatched → completion sees the same bucket ID, clears the new search and replaces the newer view with the full bucket.
- Evidence: confirmed in the off-screen GTK probe by delaying main-context delivery of a bucket-write result until a newer query was submitted; the query text was then cleared. Similar scope-only refreshes exist after reparent and rename.
- Proposed fix: validate the captured view/search generation before applying a refresh; when search has changed, refresh or rerun the current query without clearing it. Test delayed completion against a newer query and navigation.
- Status: proposed

### A7 — F11 and double-click enter different fullscreen modes

- Category: bug
- Severity: medium
- File: `sharpr/src/ui/window.rs:3513`; shared presentation implementation at `:3400`.
- Issue: F11 directly toggles the window's fullscreen flag, whereas double-click uses presentation mode; Escape only exits the latter.
- Scenario: press F11 while browsing → the window becomes fullscreen without entering presentation mode → close Info if needed and press Escape → it stays fullscreen, unlike fullscreen entered by double-click.
- Evidence: confirmed in the off-screen GTK probe: `is_fullscreen()` was true while `presentation_mode` was false, and Escape did not exit; the shared presentation path did hide the sidebar. No proposal here changes D11's approved fullscreen feature.
- Proposed fix: route fullscreen entry/exit through one mode-aware action, preserving the Compare/crop header and Escape's Info-first behavior. Test both entry points and mode transitions.
- Status: proposed

### A8 — Bucket/file editing dialogs miss the shared sheet style

- Category: interface
- Severity: low
- File: `sharpr/src/ui/collection_dialogs.rs:152`, `:241`, `:351`; `sharpr/src/ui/window.rs:3990`.
- Issue: New Bucket, Rename Bucket, Change Bucket Color and Rename File construct unstyled `AlertDialog`s while the shared compact rules require the `sharpr-sheet` class.
- Scenario: open New Bucket, then Preferences or Export → the first dialog bypasses the compact dialog rules applied to the latter surfaces, contrary to the shared-style inventory/D26–D27.
- Evidence: confirmed by source and the off-screen probe inspecting the presented New Bucket dialog; it has no `sharpr-sheet` class. No replacement of confirmation semantics is proposed.
- Proposed fix: apply the existing shared dialog styling consistently to these editing surfaces and check minimum-size layout, keyboard dismissal and focus.
- Status: proposed

### A9 — Active bucket notifications still call buckets “collections”

- Category: interface
- Severity: low
- File: `sharpr/src/ui/collection_dialogs.rs:207`, `:326`, `:398`; `sharpr/src/ui/window/tag_controller.rs:137`, `:226`.
- Issue: User-facing success/error text retains “collection” after the interface standardized on “bucket.”
- Scenario: create a bucket → the dialog says New Bucket but its notification says “Created collection”; reparenting or failed bucket edits also use the retired term.
- Evidence: confirmed active string paths, independent of internal Rust/SQL names, which may legitimately retain `collection`.
- Proposed fix: change active user-facing strings to “bucket” consistently; keep persistence names intact.
- Status: proposed

### A10 — GTK rejects the thumbnail placeholder CSS

- Category: interface
- Severity: low
- File: `sharpr/data/style.css:390`.
- Issue: The placeholder rule uses the unsupported `:empty` pseudo-class, so GTK rejects the selector instead of styling loading thumbnails as promised by D33 and the inventory.
- Scenario: open a folder, Survey or Compare before its thumbnails finish → GTK emits “Unknown name of pseudo-class” and the intended placeholder rule is discarded.
- Evidence: confirmed parser warning during the private GTK probe, with the selector and location reported above.
- Proposed fix: use a supported explicit loading class/state when binding/updating pictures, and validate CSS without parser errors. Leave scheduling and generation logic unchanged.
- Status: proposed

### A11 — Retired feature implementations and wiring remain compiled

- Category: dead code
- Severity: low
- File: `sharpr/src/ui/window.rs:1213`, `:1274`, `:3115`; `sharpr/src/collection_export.rs:45`; `sharpr/src/ui/filmstrip.rs:480`; `sharpr/src/model/library.rs:632`.
- Issue: Removed bucket import/export handlers, the unparented tag-search/save-search widgets and callbacks, and old quality-view query helpers remain after their UI/features were retired.
- Scenario: a maintainer follows these apparently supported handlers while changing bucket/search behavior → they must maintain obsolete paths that the current UI cannot invoke, and the hidden search still describes “Search tags…”.
- Evidence: confirmed reference tracing: import/export handlers have no callers; the old filmstrip SearchBar is never appended to the visible layout or activated by a caller; quality-view helpers have only internal/test references. The active viewer search has its own implementation.
- Proposed fix: remove these retired implementations and exclusive wiring/tests, retaining shared metadata/filter helpers and every legacy table/settings key. Do not include the perceptual-hash cleanup already in Backlog.
- Status: proposed

### A12 — The guide and setup reference send users to removed menu locations

- Category: docs
- Severity: low
- File: `sharpr/data/manual.md:47`; `docs/reference/development.md:126`.
- Issue: The guide locates tag maintenance under Library and development instructions locate visual-search status in Preferences, although these moved to Help and Library Overview.
- Scenario: follow the guide to Library → Turn Tags into Buckets or Remove Old Automatic Tags → neither item is there; follow model setup instructions to Preferences → Visual Search → the documented status section is absent.
- Evidence: confirmed against `ui/help_window.rs:29`, `ui/preferences.rs` and `ui/library_overview.rs`; the manual's own final paragraph already describes the newer Help location.
- Proposed fix: point both maintenance actions to Help → Library Maintenance and model/status instructions to Library → Library Overview; keep Preferences' GPU switch location.
- Status: proposed

### A13 — The architecture reference misclassifies user data and describes obsolete packaging

- Category: docs
- Severity: medium
- File: `docs/reference/architecture.md:81`, `:122`, `:123`; also `:48`.
- Issue: The storage table omits `bucket_members` and calls everything except bucket definitions rebuildable, while packaging/version text still describes working-tree rsync and commit-count versions, and progress text still promises the removed header ring.
- Scenario: use the storage table to decide which index records may be rebuilt → bucket member lists appear disposable even though they are irreplaceable; follow the packaging description → it contradicts the actual committed-source build and Cargo version source.
- Evidence: confirmed against `library_index/schema.rs:71`, the same reference's membership section, `packaging/sync-flatpak-source.sh`, `app.rs:12`, `Cargo.toml:3` and the current interface inventory.
- Proposed fix: explicitly classify bucket membership as non-rebuildable user data; correct packaging/version and indexing-status descriptions to the shipped implementation. No data migration is involved.
- Status: proposed

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
