# Current Tasks

## Active: 0.5.0 audit — awaiting user decisions

Branch: `codex/audit-0.5.0`, from `main` at `0d81115`. Stage 1 is complete:
`AUDIT.md` has 13 proposed findings; no application code changed. Offline
build and 220 default tests pass; temporary file/database and off-screen GTK
probes confirmed the key findings. Fixtures stay in `/tmp/sharpr-audit-050/`.
Next: review every ID with the user (agreed/rejected/deferred), commit the
decisions, then fix only agreed findings. No fixes before those decisions.
Do not merge, push, deploy, access real data or build Flatpak for this task;
Claude handles merging after acceptance. See `AUDIT.md` and the user's staged
instructions for the remaining workflow, including final report deletion.

## Done: Edit buckets (D35) accepted (October 16, 2026)

Crop Copy, WebP export and Duplicate add the original and the copy to Edits ›
Cropped / Exported / Duplicated; on by default with a Preferences switch. The
user tested `f50cd9f` ("works") and approved pushing; `main` was
fast-forwarded and `main`, `viewer-reset` and `v0.5.0` were pushed to GitHub.
Installed natively: `f50cd9f`.

Later candidates: the Backlog in `ROADMAP.md`, condensing the
accepted phase sections into `docs/archive/history.md`, and building the Flatpak
on the `gigabyte` server.

## Done: 0.5.0 released (October 15, 2026)

Phase 10 is complete: tag `v0.5.0`, `main` fast-forwarded.
Details and performance: `ROADMAP.md` Phase 10 and `docs/archive/history.md`.
The installed Flatpak is from `38a0758` (no fullscreen fix); rebuild it from the
tag with `build-flatpak.sh` when a bundle is wanted.

Possible follow-up (user, October 15): build the Flatpak on the `gigabyte`
server over SSH, on its local disk, and fetch the bundle.

## Done: Phases 4–9 accepted and merged (October 15, 2026)

The user accepted Phases 4, 5, 6, 7 and 9 in a manual pass of the installed
builds (`679bbe1`, then `2b5f070`) and approved the merge. `main` was
fast-forwarded to `viewer-reset` (`dd346e0` and this record); nothing is pushed.
Their notes and follow-ups are in each phase's status in `ROADMAP.md` and in its
Backlog; `docs/archive/history.md` has the summary.

Earlier installs (`2b5f070`, `679bbe1`, `ef91d32`) stay for rollback.

The accepted phase sections in `ROADMAP.md` can later be condensed into
`docs/archive/history.md`, as Phase 8 was.

## Git, backup and guardrails

- Branches (October 16): `main` holds accepted work only; each phase, feature or
  fix gets its own branch from `main`, merged after the user accepts it
  (`AGENTS.md` → Git). `viewer-reset` was retired into `main`. Active branch:
  `codex/audit-0.5.0`. No push for the audit. Keep all archive/prototype branches (`preserve/october-12-wip`,
  `visual-search-trial-wip`, `look-prototype`).
- Verified live backup: `~/.local/share/sharpr.backup-20261015-205204`, including
  settings. All database table counts match: 815 bucket members, 5,899 fingerprints,
  5,184 vectors and 7,718 legacy tag rows.
- Automatic-tag cleanup is implemented but has never been run.
- Tests use isolated temporary data/settings. Heavy jobs run one at a time.
- Phase 9 evidence (logs, screenshots, before/after debug measurements) stays
  local under `.tmp/phase9/`.
