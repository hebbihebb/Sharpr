# Current Tasks

## Active: 0.5.0 audit fixes

Branch: `codex/audit-0.5.0`. All 13 findings agreed; severity order,
one commit per finding. No merge, push or deployment; Claude merges after
user acceptance. No acceptance has been claimed.

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
and build pass. A12 `463514c`: menu directions checked against UI definitions.
All 13 findings fixed; final isolated GTK suite passes all 27 workflows.
Next: delete AUDIT in final cleanup commit, then required offline check/build.
Record fixed hashes in `AUDIT.md`; after all fixes, delete it in a final commit,
run `./check.sh` and `cargo build`. No deferred findings currently.

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

Approved: Crop Copy keeps the source format, including JPEG (recompression
tradeoff accepted); batch WebP export unchanged. Handle after the audit with
regressions, docs and a product decision. The 16:9 viewer guide (D36) is planned
separately on `main` (`494075d`); do not implement it here.
Installed native build: `f50cd9f`. Keep archive/prototype branches.
