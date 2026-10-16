# Current Tasks

## Active: 0.5.0 audit fixes

Branch: `codex/audit-0.5.0`. The user agreed to all 13 findings and requested
severity order with one commit per finding. No merges, pushes or deployment;
Claude merges after the user accepts the fixes.

Committed and verified:
- A1 `cd32145`: exclusive Duplicate/Copy outputs; source bytes/permissions,
  symlinks, concurrent copies and failure cleanup covered (223 tests).
- A2 `dcc9cbf`: moves/renames carry buckets, fingerprints and legacy tags;
  retained destinations and rollback tested, including real GTK Rename (227).
- A3 `ff4846f`: scoped re-linking and Info counts; excluded data untouched (229).
- A4 `e7c6e0a`: serial/coalesced clipboard worker; orientation, original bytes,
  newest-request handling and GTK success/error verified (231).
- A5 `9301db7`: membership backup/migration runs after window construction
  on a worker; failure preserves data and disables bucket edits (24 GTK tests).
- A6 `1215b16`: bucket updates preserve newer search/filter inputs and navigation; the
  regression failed first and now passes; offline checks pass (231 tests),
  all 25 isolated GTK workflows pass, including rapid navigation.

A7 `f73623d`: F11/double-click share presentation; Info-first Escape and
Compare/crop controls verified (231 default tests, three GTK checks, build).
A13 `effc5df`: architecture correctly identifies bucket members as user data and
describes committed Flatpak sources, Cargo versions and Overview progress.
A8 verified: editing dialogs use the shared sheet style; GTK tour and Rename
workflow pass, New Bucket screenshot inspected. Next: A9–A12 (low).
Bugs require a failing regression first. Record each fixed hash in `AUDIT.md`.

## Verification and safety

- Real user data and Pictures are off limits. Tests use memory GSettings and
  isolated XDG directories; Cargo and advisory checks remain offline.
- Run one heavy command at a time. Never build Flatpak.
- Unit fixtures/logs: `/tmp/sharpr-audit-050/`. GTK fixtures must use
  `.tmp/audit-050/gtk/{data,config,cache,fixtures}`: `/tmp` is a different device,
  so GIO's trash tests cannot use its mount-root trash. Same-filesystem isolated
  fixtures pass; never use the real trash as a workaround.
- `/tmp/sharpr-audit-050/bin/cargo` wraps deny with `--offline` and nextest
  with two test threads for `./check.sh`; `CARGO_NET_OFFLINE=true`, build jobs 2,
  `CC=/usr/bin/gcc CXX=/usr/bin/g++`. GTK uses `tools/gtk-offscreen.py`, one test
  at a time. Baseline/after logs: `a4-{before,after}.jsonl`, `a5-after.jsonl`.
- Final: all agreed findings fixed individually, deferred items to Backlog
  (none currently), delete `AUDIT.md` in a final commit, then `./check.sh` and
  `cargo build`. Report manual retest instructions; leave deployment/merge to
  the user and Claude. No user acceptance has been claimed.

## Other state

The 16:9 viewer guide (D36) is planned separately on `main` (`494075d`).
Do not implement it during audit fixes. Installed native build: `f50cd9f`;
release 0.5.0 and prior phase acceptance remain in `ROADMAP.md`/archive.
Keep all archive/prototype branches. No real data was accessed for this work.

Follow-up approved: Crop Copy keeps the source format, including JPEG
(recompression tradeoff accepted); batch WebP export stays as-is. Handle
separately after the agreed audit findings, with tests and a product decision.
