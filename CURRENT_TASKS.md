# Current Tasks

## Active: Phase 10, Release 0.5.0 (started October 15, 2026)

The user's choices are D34: version 0.5.0 from `Cargo.toml`; no "Open with
Sharpr"; Vulkan `llama-server` in the Flatpak now; Flatpak may write home.

1. **Committed:** one version source (metainfo test), desktop entries without
   `%F`/`MimeType` and with `StartupWMClass` = app ID, `FocusedImageSet` and the
   unused rexiv2/gexiv2 dependency and Flatpak modules removed, deny reason
   corrected, yanked `chacha20` updated, `cargo-sources.json` regenerated.
2. Flatpak: clean source pinned to a commit, writable home, Vulkan
   `llama-server` with CPU fallback; full build and a GPU-indexing check.
3. README feature list from `product.md`.
4. Release checklist: `./check.sh`, GTK tests, full Flatpak build, release-build
   performance against the Phase 0 baseline, the user's manual workflow pass on
   the real library; then tag `v0.5.0` and fast-forward `main` (push is the
   user's call).

## Done: Phases 4–9 accepted and merged (October 15, 2026)

The user accepted Phases 4, 5, 6, 7 and 9 in a manual pass of the installed
builds (`679bbe1`, then `2b5f070`) and approved the merge. `main` was
fast-forwarded to `viewer-reset` (`dd346e0` and this record); nothing is pushed.
Their notes and follow-ups are in each phase's status in `ROADMAP.md` and in its
Backlog; `docs/archive/history.md` has the summary.

**Deployed:** the desktop launcher runs `2b5f070` (later commits are notes only).
`679bbe1` and `ef91d32` stay installed for rollback.

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
