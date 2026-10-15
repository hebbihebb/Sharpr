# Current Tasks

## Done: Phases 4–9 accepted and merged (October 15, 2026)

The user accepted Phases 4, 5, 6, 7 and 9 in a manual pass of the installed
builds (`679bbe1`, then `2b5f070`) and approved the merge. `main` was
fast-forwarded to `viewer-reset` (`dd346e0` and this record); nothing is pushed.
Their notes and follow-ups are in each phase's status in `ROADMAP.md` and in its
Backlog; `docs/archive/history.md` has the summary.

**Deployed:** the desktop launcher runs `2b5f070` (later commits are notes only).
`679bbe1` and `ef91d32` stay installed for rollback.

**Next:** Phase 10, Release (`ROADMAP.md`), when the user starts it. Before that,
the accepted phase sections in `ROADMAP.md` can be condensed into
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
