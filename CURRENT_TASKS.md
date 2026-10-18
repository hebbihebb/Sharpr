# Current Tasks

## Accepted: more than one library root (Work Order item 25, D41)

Deployed as `90e594e`; accepted by the user October 18 ("deployed and working"),
merged to `main` and pushed. A second root (Downloads beside Pictures) is added
from Main menu → Library → Add Library Root… or Preferences → Library → Add…;
each root is a top-level sidebar entry, and right-click → Remove from Library
forgets it without touching files, buckets or index data (hidden bucket members
come back when the root is added again). Overlapping roots are refused naming the
root in the way. Folder icons are coloured per root (blue, orange, green, pink,
teal, yellow). Buckets, search, Find Copies, the indexer and the Library
Overview cover every root. With one root the saved settings are unchanged and no
database was migrated. `./check.sh` passes (301 tests), 36 GTK workflows pass.
Next: Work Order item 11 (notice changes under the library roots).

## Repository disk cleanup

Done on `codex/repo-cleanup` and merged to `main`. User authorized the checked cleanup plan.
Verified: 69.93 → 20.86 GiB (49.07 GiB recovered). Removed obsolete Flatpak
working builds, Phase 0/prototype compilation output, the main incremental cache
and 186 generated test fixture/cache trees. Models, runtimes, reports,
screenshots, patches and test databases retained; 40,746 retained temporary
files have unchanged sizes/timestamps. Installed launcher/binary unchanged
(SHA-256 checked). Text evidence from removed trees copied and hash-checked in
`.tmp/cleanup-evidence/retained/`; local inventory in its sibling `manifest.json`.
Build instructions now reuse `sharpr/target/`; cleanup guidance added.
`cargo build --offline --locked` and `git diff --check` pass. No deployment/push.

## Accepted: the Rename sheet (D40)

Deployed as `127287c`; accepted by the user October 17 ("it works, i like it";
it looks like the WebP export sheet), merged to `main` by fast-forward; not
pushed. Rename…
(F2, image menu) opens a Sharpr sheet (`ui/window/rename_sheet.rs`): the name
in a field that has the focus with all of it selected, the extension as a fixed
chip beside it (click it to edit the extension as text), Enter renames, and an
empty, invalid or taken name shows why under the field and keeps Rename off.
Replaces the Libadwaita alert dialog, which put the focus on its Rename button
when opened from the image menu, so typing missed the field (the user's report
on Sonnet's `fix/rename-preselect-stem`, which this supersedes). GTK workflow
`rename_from_the_menu_types_over_the_name_and_keeps_the_extension` goes through
the real menu; `./check.sh` passes.

## Accepted: a broad library root such as Home (Work Order item 10)

Deployed in test build `7bb4aa6` (with the Rename fix); accepted by the user
October 17 ("changing root works"), merged to `main` by fast-forward; not pushed.
- One walk rule (`model/walk.rs`): stay on the root's filesystem (device-id
  check) and skip hidden folders. Used by the sidebar scan and by
  `collect_library_image_paths_cancellable`, so the indexer, fingerprints,
  search and Overview no longer crawl `~/Network` or `.cache` either.
- The sidebar scans on a worker and lists folders as it finds them (batches
  every 200 ms, rows inserted in place so selection and scroll survive); the
  first folder opens when the scan ends, unless one was already picked. A new
  scan cancels the old one. Folders seen for the first time start collapsed.
- Unit tests (hidden, nested, disabled, canceled, device rule, tree and row
  merge) and GTK workflow `a_broad_root_lists_image_folders_without_hidden_ones`.
  `./check.sh` parts and all 34 GTK workflows pass.
- The user's test with Home as the root decides acceptance.

## Accepted: file-action results and thumbnail generations (Work Order items 9 and 7)

Deployed as `790e2a4`; accepted by the user October 17 ("everything works"),
both merged to `main` by fast-forward (item 7, then item 9); not pushed.
- Item 9: when a file job finishes, its new files (Duplicate, Copy to Folder,
  Rename, moves into the open folder) are read on a worker and added to the
  open folder's rows in the current sort order, with no reload; the selection,
  thumbnail generation and scheduler are untouched. Renaming the image in view
  keeps it in view. With a search or chips active the file shows when they are
  cleared. Also fixed: removing a row above the selection left `selected_index`
  one too high. GTK workflows `file_actions_add_their_results_to_the_open_folder`
  (failed first) and `file_action_results_wait_for_a_cleared_search`.
- Item 7: thumbnail responses carry their generation, stale ones are dropped
  (audit finding 7), and folder loads hold a token (`LibraryManager::load_token`)
  so A → B → A cannot append two loads into one store.
- From the user's test: Rename should preselect the file name without its
  extension (done as the Rename sheet, D40).

## Accepted: Compare decodes (Work Order item 8)

Deployed as `6bf37cd`; accepted by the user October 17 ("all tested working"),
merged to `main` by fast-forward; not pushed. Each
Compare side decodes one image at a time and keeps only the newest request
waiting (`ui/compare_page/decodes.rs`); Fit shows a 2560-pixel preview and the
full image loads on a worker only when zooming needs it, and is given up again
once back out. `CompareView` now sizes and zooms by the image, not the texture.
GTK workflow `compare_bounds_decodes_and_previews_at_fit` failed first (8
decodes at once), then passed; `./check.sh` and all 31 GTK workflows pass.

## Accepted: file actions off the GTK thread (Work Order item 6)

Trash, Undo, Move/Copy to Folder, Rename, Duplicate and Find Copies' trash run
on one ordered worker (`file_jobs.rs`, `ui/window/file_jobs_controller.rs`).
Deployed as `5f8b8d0`; accepted by the user October 17, merged to `main` by
fast-forward; not pushed. Follow-up in the Backlog: show new files from file
actions in the open folder. Item 7 is next.

## Accepted: audit fixes and source-format crops

Branch: `codex/audit-0.5.0`. All 13 findings agreed; severity order,
one commit per finding. Deployed as `cabda19`. User accepted on October 17, 2026
(PNG and JPEG crops keep format and open elsewhere; batch export still WebP;
originals unchanged). Merged to `main` by fast-forward; not pushed. The
commit hashes below predate the rebase onto `main`; use `git log` for current ones.

## Accepted with notes: 16:9 viewer guide (D36)

Ctrl+G / image menu → 16:9 Guide, deployed as `c724bab`, merged to `main`.

## Accepted: guide follow-ups (D38)

Corner squares resize the guide (16:9 kept), its outline moves it, inside still
pans; it stays put through zoom and image changes; Ctrl+G twice refits it.
Crop Copy of the viewed image (or Ctrl+Shift+G) starts on the guide. A one-tool
merge of guide and crop is in the Backlog for later. User's first test of `2bd5e4a`:
works, but dragging felt laggy; the guide now draws with snapshot nodes and
resizes smoothly along its diagonal. Accepted October 17 ("it works", `a795ae1`),
merged to `main`.

## Accepted: smooth Crop Copy selection

At the user's request, Crop Copy's selection draws on the guide's snapshot
layer (no viewer-sized Cairo repaint per movement), and its 16:9 corner resize
follows the drag along the diagonal, so straight drags shrink as well as grow.
Accepted October 17 ("works", `95da20a`), merged to `main` and pushed.

## Accepted: Crop Copy zoom (D39)

Crop opens on the viewer's scale and centre (held until the first zoom or pan,
because the side panels slide away as it opens); scroll zooms at the pointer,
dragging outside the box (or the middle button) pans, the selection stays on its
image pixels, and the full image loads on a worker when zooming outgrows the
preview. Pure geometry in `ui/crop_view.rs`; GTK workflow
`crop_zooms_with_the_selection_attached`. The user's first test of `9d8c664`
found the opening zoom jumped in; fixed in `e04e361`. Accepted October 17
("it is fixed"), merged to `main` by fast-forward; not pushed.

## Done: cargo deny advisories

`ROADMAP.md` Work Order item 2 is done: `deny.toml` ignores RUSTSEC-2026-0370,
-0371 and -0372 (zvariant, zbus_names, zvariant_derive), all via glycin only.
Config only; `./check.sh` and `cargo build` pass. Merged to `main`; not pushed.
Items 3 (D36), 4 (D38) and 5 (Crop Copy zoom, D39) are done; item 6 (file operations off the GTK thread) is done too.

Committed and verified:
- A1 `cd32145`: exclusive copies, preserving source bytes and permissions.
- A2 `dcc9cbf`: moves/renames preserve buckets, fingerprints and legacy tags.
- A3 `ff4846f`: re-linking and Info exclude disabled/outside-root files.
- A4 `e7c6e0a`: bounded background clipboard decoding and stale-result checks.
- A5 `9301db7`: background bucket upgrade, backup checks and failure UI.
- A6 `1215b16`: bucket updates preserve newer search and navigation.
- A7 `f73623d`: shared fullscreen presentation, Info-first Escape/mode controls.
- A13 `effc5df`: architecture protects membership and describes current builds.
- A8 `262f34a`: shared editing-dialog styles; GTK tour/screenshot checked.
- A9 `5c79227`: active messages consistently say bucket.
- A10 `c72fa97`: valid thumbnail loading CSS; scheduler unchanged.

A11 `ab3e01f`: retired paths removed; 229 default tests, four GTK workflows
and build pass. A12 `463514c`: current maintenance/model-status menu directions.
All 13 findings fixed; final isolated GTK suite passes all 27 workflows.
The final report and all fix hashes are preserved in commit `5c9dd13`;
`AUDIT.md` is removed for merge as requested. No deferred findings.
Final audit checks/build passed after report removal (`7ef00a8`).
Crop Copy source-format follow-up (D37) is implemented and verified: two
regressions failed first, full offline checks pass (236 tests), all 27 GTK
workflows pass, including crop/edit buckets and unchanged batch WebP export.
No dependencies added. Accepted; merging.

## Safety and verification

No real user data or Pictures. No network or Flatpak builds; one heavy job.
Memory GSettings and isolated XDG directories; Cargo/advisories offline.
Logs/unit fixtures: `/tmp/sharpr-audit-050/`. GTK fixtures must use
`.tmp/audit-050/gtk/{data,config,cache,fixtures}` on the home filesystem for
GIO trash tests. Never substitute the real trash. Runner: `tools/gtk-offscreen.py`.
Offline check wrapper `/tmp/sharpr-audit-050/bin/cargo`: deny `--offline`,
nextest two test threads; build jobs 2, `CC=/usr/bin/gcc CXX=/usr/bin/g++`.
GTK runs one test at a time. Bench samples are debug/Cairo; no release/manual
performance claim. Logs: `a4-{before,after}`, `a5-after`, `a6-after`, `a10-after`,
`a11-after` JSONL in the log directory.

## Separate follow-ups

Crop Copy now retains source format, including JPEG at quality 100 (the user
accepted recompression). PNG/TIFF sample depth and alpha are retained; WebP/JXL
crops are lossless. Unsupported sources fail explicitly. The 16:9 viewer guide
(D36) was built separately and is now merged.
Installed native build: `f50cd9f`. Keep archive/prototype branches.
