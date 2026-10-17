# Current Tasks

## Accepted: audit fixes and source-format crops

Branch: `codex/audit-0.5.0`. All 13 findings agreed; severity order,
one commit per finding. Deployed as `cabda19`. User accepted on October 17, 2026
(PNG and JPEG crops keep format and open elsewhere; batch export still WebP;
originals unchanged). Merged to `main` by fast-forward; not pushed. The
commit hashes below predate the rebase onto `main`; use `git log` for current ones.

## Active: 16:9 viewer guide (D36), awaiting acceptance

Branch `feature/viewer-16x9-guide`: Ctrl+G / image menu → 16:9 Guide, inside the
image with the outside dimmed. Geometry tests and a GTK check pass. Not deployed.

## Done: cargo deny advisories

`ROADMAP.md` Work Order item 2 is done: `deny.toml` ignores RUSTSEC-2026-0370,
-0371 and -0372 (zvariant, zbus_names, zvariant_derive), all via glycin only.
Config only; `./check.sh` and `cargo build` pass. Merged to `main`; not pushed.
Item 3 (D36) is in progress on its feature branch.

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
(D36) stays planned separately on `main` (`494075d`); not implemented here.
Installed native build: `f50cd9f`. Keep archive/prototype branches.
