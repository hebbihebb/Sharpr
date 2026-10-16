# Current Tasks

## Active: Edit buckets (D35)

The user asked for it after 0.5.0: Crop Copy, WebP export and Duplicate add the
original and the copy to "Cropped" / "Exported" / "Duplicated" under an "Edits"
parent; on by default with a Preferences switch. Scope and acceptance:
`ROADMAP.md` → Next: Edit Buckets.

**Implemented and verified, installed for the user's test:** `buckets::add_edit_copies`
finds or creates Edits and the action's bucket (reusing a same-named bucket in
the library), then uses the existing writer off the GTK thread. Hooks: crop
completion, batch export completion (the report now pairs each output with its
source), Duplicate. GSettings `edit-buckets` (default on) with a Preferences
switch. A copy saved outside the library root is left out with a notice.
Waiting on the user's hands-on check.

## Done: 0.5.0 released (October 15, 2026)

Phase 10 is complete: tag `v0.5.0`, installed natively, `main` fast-forwarded.
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

- Development: `viewer-reset`; `main` holds accepted work. Pushing is the user's
  decision. Keep all archive/prototype branches (`preserve/october-12-wip`,
  `visual-search-trial-wip`, `look-prototype`).
- Verified live backup: `~/.local/share/sharpr.backup-20261015-205204`, including
  settings. All database table counts match: 815 bucket members, 5,899 fingerprints,
  5,184 vectors and 7,718 legacy tag rows.
- Automatic-tag cleanup is implemented but has never been run.
- Tests use isolated temporary data/settings. Heavy jobs run one at a time.
- Phase 9 evidence (logs, screenshots, before/after debug measurements) stays
  local under `.tmp/phase9/`.
