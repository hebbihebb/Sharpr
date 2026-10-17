# Current Tasks

## Verified, awaiting acceptance: file-action results in the open folder (Work Order item 9)

Branch: `feature/file-action-results`, from `fix/thumbnail-generations` (item 7,
not yet accepted; its folder-load token guards the insert). When a file job
finishes, its new files (Duplicate, Copy to Folder, Rename, moves into the open
folder) are read on a worker and added to the open folder's rows in the current
sort order. No reload: the selection stays on its image, the thumbnail
generation and scheduler are untouched. Renaming the image in view keeps it in
view under its new name. With a search or chips active the file waits in the
kept view and shows when they are cleared. Also fixed on the way: removing a
row above the selection left `selected_index` one too high (it pointed at the
next image). GTK workflow `file_actions_add_their_results_to_the_open_folder`
failed first (3 rows, 4 expected), then passed, with a second for the narrowed
view and six model tests; `./check.sh` (284 tests) and all 33 GTK workflows
pass. Not deployed.

## Verified, awaiting acceptance: thumbnail generations and folder-load token (Work Order item 7)

Branch: `fix/thumbnail-generations`, rebased onto `main` after item 8. Every
thumbnail response carries its generation; the worker sends no failure from a retired generation and the GTK
side drops stale responses (audit finding 7). Folder loads hold a token
(`LibraryManager::load_token`), so A → B → A cannot append two loads into one
store. Four regression tests (three failed first); `./check.sh` and all GTK
workflows pass. Not deployed. To test: rapid folder switching (especially back
to the same folder) and rapid scroll in a large folder.

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
