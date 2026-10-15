# Current Tasks

## Active: Phase 10, Release 0.5.0 (started October 15, 2026)

The user's choices are D34: version 0.5.0 from `Cargo.toml`; no "Open with
Sharpr"; Vulkan `llama-server` in the Flatpak now; Flatpak may write home.

1. **Committed:** one version source (metainfo test), desktop entries without
   `%F`/`MimeType` and with `StartupWMClass` = app ID, `FocusedImageSet` and the
   unused rexiv2/gexiv2 dependency and Flatpak modules removed, deny reason
   corrected, yanked `chacha20` updated, `cargo-sources.json` regenerated.
2. **Committed and verified:** Flatpak from `git archive` of HEAD, writable
   home, Vulkan `llama-server` (with build-only SPIRV-Headers) beside the CPU
   build, 512 px icon, memory-bounded `build-flatpak.sh` (Flatpak escapes
   systemd memory caps; an uncapped build crashed the computer). Built and
   installed from `38a0758` as 0.5.0. In the sandbox the GPU build lists the
   RTX 4060 and embeds an image in 0.05 s warm (first 12.7 s, shader warm-up)
   against 0.38 s for the CPU build.
3. **Committed:** README feature list.
4. Release checklist: **passed** `./check.sh` (218 tests), all 18 GTK workflows
   and the full Flatpak build on `38a0758`. Native launcher runs `38a0758`.
   **Open:** the user's manual workflow pass on the real library (find, curate,
   prepare a wallpaper, clean up, compare, view), whose log gives the
   performance comparison with the Phase 0 baseline (498 ms startup, 116–162 ms
   for the 2,216-image folder, 2.3 GB peak) with no heavy job running; earlier
   October 15 logs overlapped builds and do not count. Then tag `v0.5.0` and
   fast-forward `main` (push is the user's call).

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
