# Current Tasks

## Active Milestone

**Phase 5: visual search and tag retirement** (`ROADMAP.md`). In use since `c176934`; all later Phase 5 work is deployed in `baa007d` (October 14). Not yet accepted.
- **Built:** indexing and search (`58ac0ef`), tag retirement (`78a0639`), packaging (`b07a226`), and GPU indexing (`c176934`).
- **The user's test so far:**
  - Overall (October 14): Sharpr "seems to run even better than it did before this recent work started, regardless of indexing work".
  - Search while indexing: "working really well, I am totally impressed and happy".
  - Browsing during GPU indexing "feels smooth". This is a manual check; the measured 10% check is still open.
  - The real library is fully indexed: 5,178 vectors (identical copies share one) and 0 failures. It has no AVIF or HEIC files.
- **Done:** the Phase 5 audit fixes (`df82623`..`cf438a9`), the Library Overview, search into buckets, and negative search, all deployed in `baa007d`.
  - **All Phase 5 work is built and deployed.** Left before acceptance: the user's test, and a measured browsing check during indexing.
- **Open user decision:** keep file-name matches first in results. They stay for now; the user will judge after use.

## Next Milestone: Phases 6–8, Then A Proper Install (User, October 14)

The user asked for Phases 6, 7, and 8 to be finished, then a proper install to test; possibly an adversarial `gpt-6.1-sol` audit after that. Phases 9–10 need more thought first. The install is approved for the end of this work, with the usual backup first.

1. **Phase 6, wallpaper output:**
   - Batch WebP export (GTK-free engine, then dialog and background job).
   - Retire Resize Copy and the non-WebP outputs.
   - Crop at full size in the viewer, with a worker-decoded preview fallback.
   - The Wallpapers folder as the default and a quick destination.
2. **Phase 7, cleanup:**
   - Exact copies by fingerprint.
   - Resized or converted copies by visual-search vectors (≥ 0.99; 0.95–0.99 only with matching aspect and a pixel check).
   - A review window, and trash only after confirmation.
3. **Phase 8, Compare:** N images two at a time, with A/B flip (Space), split, and side by side; knockouts; zoom lock; survivors selected on close.

**Status:** Phases 6 (`5e2ad3d`, `9f7a42a`), 7 (`a32227a`), and 8 (`baa007d`) are built, committed, verified by tests, and installed in `baa007d` with a backup first.
- **User test of `baa007d` (October 14):** export and crop work; search into buckets works. Find Copies never got past its spinner, Compare "works for two images only", and `tower -water` looked ignored. All three are diagnosed and fixed in `934b49b` (`ROADMAP.md` Phases 5, 7, 8).
- **Adversarial audit** (`gpt-6.1-sol`, October 14): `docs/archive/audit-2026-10-14-phases6-8.md`, 12 findings. Ten are fixed in the commits after `e9d3c8f`; finding 12's Library Overview parts and finding 6's Compare decode concurrency are in the Backlog.
- **User test of `5044c72` (October 14):** Compare and the selection fixes worked. Asked for: Find Copies limited to the folder with nothing pre-ticked, a per-side image picker in Compare, a bucket picker that adds only on Add, and a stronger minus word. All four are in `8501517`.
- **User test of `ef91d32` (October 15):** 17 of 19 items passed; the results are in `ROADMAP.md` Phases 5–8. The faults and the user's choices (D25) set the fix round below. The library switcher (item 19) is expected until Phase 9's "One library root".

## Next: Fix Round After `ef91d32` (October 15)

**Done: committed and verified, not deployed.** `./check.sh` and all 10 GTK tests pass.

1. **Ctrl+A after clicking a thumbnail** (`9c6eb14`). Cause confirmed by a test: the filmstrip `ListView`'s built-in Ctrl+A took the key first. It is now off, so the window selects every image shown. In the search box, Ctrl+A still selects the text.
2. **Click outside closes** the bucket picker, the "Add Search Results to a Bucket" choice, and the WebP export, like Cancel (`15b5d7e`).
3. **Undo for filmstrip trash** (`475249c`): "Moved N images to the trash · Undo" for 10 seconds. Undo puts each file back by the freedesktop.org trash spec, never over a file now at its name. A file that can't return stays in the trash and is reported. In a folder view the folder reopens with the image selected; a bucket or search view shows it again when reopened.

**Install:** not now (user, October 15). It stays undeployed until the user asks.

## Next: Phase 9, Interface Review (October 15)

Approach agreed with the user (`ROADMAP.md` Phase 9, "The user's direction"): the map and the user's marks first, then research, then an early look decision (Libadwaita kept, hybrid, or plain GTK4), then proposals. No build work until proposals are approved.

- **Step 0, the map: published October 15** at https://claude.ai/artifact/1uJDcNEQbrAcTRZ6GXp6jQ (private). It has 83 controls in 10 areas, 30 screenshots, two diagrams, 9 counted tasks, and 18 observations (N1–N18). The user's marks and notes save in the page's database (`marks/<control id>`, `notes/general`); read them back with ArtifactData before the review.
- **Screenshots** come from the `interface_tour_screenshots` GTK test (`8d6111b`), which writes `tour/` beside the fixtures. Rerun it after interface changes for before-and-after pictures.
- **The user's marks (October 15), 82 of 83:**
  - **Daily:** Filter, Sort, the folder tree, buckets, type to search, chips, minus words, right-click Add to Collection and Compare, scroll zoom and pan, the Compare split divider.
  - **Never:** the Library and Photos titles, Open Folder, the library switcher, the indexing ring, every Show Overlay switch, Fit and 1:1 in ≡, Presentation (F11), right-click Info, Rename, Whole library, Show More, the Manual, Shortcuts, About, the two tag tools, and Preferences' libraries, collections, and cache.
  - **Confusing:** Compare's `[` `]` dismiss keys ("bad shortcut key, not easy to find on keyboard") and its "4 of 4 left" with Undo.
  - **Notes:** "dialogs and right click menus are chunky"; the overlay "would be better to be contextual or appear on hover"; zoom by scroll wheel alone "works well"; double-click for full screen; the window is usually maximized, and closing it quits.
  - **How the filmstrip is used:** to scroll fast through many images with the scroll wheel or by dragging the small scrollbar handle, finding and reviewing by eye. Arrow keys would be far too slow; they suit left-and-right stepping where that applies.
  - **Icons (N16):** the sidebar toggle, the menu button, and the Appearance tab render in the real app, so the broken icons are a test-window artifact. The real menu button shows "…", not ≡ (the test window uses a different icon theme).
- **Step 2a, research: published October 15** at https://claude.ai/artifact/1eX3MVj6LwfafrLeTYyaKd (private). It covers Lightroom, darktable, FastRawViewer, Eagle, Photo Mechanic, Google Photos and Immich, Loupe, Strata and Zed, with sources. It has 18 ideas (I1–I18), with Want / Maybe / No reactions saved in `reactions/<idea id>`. Its reading of the evidence: the hybrid look.
- **The user's reactions (October 15), all 18:**
  - **Want:** I1 fast-scroll handle; I2 arrow keys pick the Compare winner; I3 a thumbnail row instead of "4 of 4 left"; I4 a key for Compare; I5 Survey view; I6 compact menus; I7 pickers as popups; I8 one dialog style; I10 add to the last bucket; I12 the chip on pointer movement; I13 hover grows the chip into Info; I14 a neutral dark canvas; I15 one top row; I17 remove unused controls; I18 one Help item and a slimmer main menu.
  - **Maybe:** I9 a type-to-filter bucket picker; I16 a sort button that names its sort.
  - **No:** I11 number keys for buckets.
- **Step 2b, the look prototype: built October 15** on branch `look-prototype` (`2bcca72`, not merged, not deployed). The hybrid style: one top row (I15), compact menus (I6), pickers as popovers at the pointer (I7), a neutral dark canvas (I14), and the chip shown on pointer movement (I12), from one stylesheet in `sharpr/src/ui/look.rs`. All 11 GTK tests and `check.sh` pass on the branch.
  - Comparison page: https://claude.ai/artifact/NKxHfeUMzB6YCrVo2EZvWs (today beside the prototype, plus Strata). The user's answer saves to `decision/look`: `hybrid`, `plain`, or `adwaita`, with optional extras (`mono`, `dense`, `footer`, `sortname`) and a note.
  - **The user chose the hybrid (D26),** with three extras to try (monospace, tighter lists, a shortcut footer) and three layout notes (search over the viewer, a narrower sidebar, one Filter and Sort control above the filmstrip).
  - **Filters (user, October 15): option A,** provided the chips look good in the new style. Typed chips become the one filter language; a combined Sort and Filter menu adds chips; "closest to 16:9" stays a sort (settles N2). Needs a decision-log entry once the chip design is accepted.
  - **Design language (user, October 15):** "Sharpr needs to be Sharpr." Draft in pass 2: 4 px corners, a JetBrains Mono readout for numbers, chips, the corner pill and key hints, hairlines instead of raised bars, a neutral image canvas. The user disliked the old corner pill.
  - **Pass 2 built** on `look-prototype` (`9ad237c`): the top row follows the columns (all bars 42 px), one Sort and Filter button, a 170–210 px sidebar, the readout face, restyled chips and pill, key hints in Compare and crop, and `SHARPR_LOOK_MONO=all` for the monospace-everywhere variant. 11 GTK tests and `check.sh` pass. The comparison page shows it; answers save to `decision/pass2` (`choice`: numbers, all, or unsure; `extras`: what works; `note`).
  - **The user's pass-2 answers:** monospace everywhere; all seven items work (columns, Sort and Filter, sidebar, chips, pill, key hints, corners). No folder name above the filmstrip: the sidebar shows it (`52595e5`).
  - **Hands-on trial (October 15):** a release build on a copy of the library, data, cache and settings in `.tmp/look-trial/` (copies byte-identical, databases intact). The user: "i like it … i generally like the direction". Decided as D27.
  - **Committed:** the prototype is rebased onto `viewer-reset` (`cb57749`..`52595e5`); the `look-prototype` branch is kept. Not deployed.
  - **Next:** the finishing touches in `ROADMAP.md` Phase 9, then proposals for the behaviour ideas.

## State (October 14, 2026)

- **Git:**
  - `main` is at `1924346`, the accepted Phases 1–2.
  - `viewer-reset` carries everything since.
  - Pushed to GitHub on October 14 at the user's request.
- **Deployed:** `ef91d32` at `~/.local/opt/sharpr/ef91d32`, with `current` pointing to it, installed October 14 at the user's request. It adds Compare from Find Copies to `8501517`.
  - Smoke check: it ran on a copy of the live data and settings in isolated directories, with no panic and all databases intact.
  - It changes no database and no settings schema.
  - **Rollback:** close Sharpr, then run `ln -sfn ~/.local/opt/sharpr/8501517 ~/.local/opt/sharpr/current`. No data restore is needed.
  - The model and both runtimes are installed in `~/.local/share/sharpr-models/embeddinggemma-2/` (`runtime/` for the processor, `runtime-gpu/` for Vulkan). In the Flatpak, the server is `/app/libexec/sharpr-llama/llama-server`.
  - Redeploy only with approval, via `python3 tools/install-native-launcher.py`.
- **Live data:**
  - 9 buckets: the user added 3 on October 14.
  - 7,718 tag rows, unused. Of these, 7,346 are old automatic tags; their cleanup is ready but not run.
  - Bucket membership was migrated by `bf12e07` (151 members across the original 6 buckets).
- **Backups**, each byte-verified and integrity-checked, in `~/.local/share/`:
  - `sharpr.backup-20261014-150245`, taken before `ef91d32`, with a dump of the settings
  - `sharpr.backup-20261014-144931`, taken before `8501517`, with a dump of the settings (12 buckets, 814 members)
  - `sharpr.backup-20261014-135326`, taken before `5044c72`, with a dump of the settings (10 buckets, 410 members)
  - `sharpr.backup-20261014-055753`, taken before `baa007d`, with a dump of the settings (`dconf-sharpr.ini`)
  - `sharpr.backup-20261014-015921`, taken before `c176934`
  - `sharpr.backup-20261013-111041`, taken before `bf12e07`
  - three older ones
- **Archive branches, kept:** `preserve/october-12-wip` and `visual-search-trial-wip`.
- **Machine notes:**
  - GNOME Shell leaked about 6.9 GB of graphics memory until blur-my-shell and the wallpaper slideshow were turned off; which one caused it is not known yet.
  - Run heavy jobs one at a time inside `systemd-run --user --scope -p MemoryMax=9G`. On October 13, parallel jobs ran out of memory and the kernel killed the terminal.

## Waiting On The User

- **Thumbnail memory:** keep whole-folder preload, or cap it (`ROADMAP.md` Phase 2, item 8).
- **Automatic-tag cleanup:** run Main menu → Library → Remove Old Automatic Tags… whenever convenient.

## Performance Baseline (Phase 0, Build `5832010`, October 12)

| Measure | Result |
| --- | --- |
| Startup to first folder ready | 498 ms |
| Folder open, 2,216 images | 116–162 ms; one outlier at 2,449 ms (cause open) |
| Thumbnail worker time | p50 1 ms, p95 4 ms, max 293 ms |
| Memory (RSS) | 243 MB at start, 2,345 MB peak |

## Guardrails

- Test in isolated directories (`docs/reference/development.md`); never touch the live data directory.
- Commit each unit. No merge into `main` and no deployment without approval; push only when the user asks.
