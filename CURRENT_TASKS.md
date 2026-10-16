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

A5 is verified: membership backup/migration runs on a worker after window
construction, with preparing/error UI and edit guards. Its regression failed
first; full offline checks (231 default tests) and all 24 GTK workflows pass.
Next: commit A5, then A6, A7, A13 (medium), then A8–A12 (low).
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
