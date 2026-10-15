# Current Tasks

## Active: Phase 9 interface review (October 15, 2026)

The user asked Codex to review Claude's UI work and continue. Started from a clean `viewer-reset` at `b7241f7`.

**Approved direction:** hybrid GTK4/Libadwaita (D26), then Sharpr's design language (D27): monospace everywhere, aligned 42 px top bars, compact menus and popover pickers, narrow sidebar, neutral image canvas, hover/movement readout, filter chips and one Sort and Filter menu. Implemented in `cb57749`..`52595e5`; the user liked the isolated hands-on trial. Not deployed.

**Accepted UI units (October 15):** styled Preferences/Shortcuts (`8b6ac1d`), bundled JetBrains Mono (`0183d3f`), contextual Compare/crop controls (`c5699d7`), and the WebP export sheet (`fde5a52`). The user tested choices/Undo, viewing modes, linked zoom/pan, returning survivors/search/chips, output-folder selection and export, and light/dark/resized sheets: everything works. Phase 8 Compare is closed in `docs/archive/history.md`. All are committed and verified (latest full checks: 211 regular tests and 14 GTK tests); not deployed. The intermittent older Compare picker test remains in the roadmap Backlog. Candidate thumbnails remain a separate proposal.

**Filter cleanup implemented, verified and committed on `viewer-reset`:** removed the detached controls and independent filtering state; menu and typing share chips. The user reports filtering works. Dimension caching, aspect sorting and the thumbnail scheduler remain. Original verification: build, `./check.sh` (211 tests), all 14 GTK tests, light/dark screenshots and native synthetic-library checks. Before/after 5,000-image debug logs are `.tmp/filter-cleanup-before.jsonl` and `-after.jsonl`; no release performance claim. Not deployed.

**Acceptance follow-up fixed and verified; manual retest pending:** Ascending/Descending now pass boolean targets to their action. Virtual reloads keep active cached thumbnails for retained paths, including global-cache evictions, and release excluded paths. Both regressions failed before the fixes and pass now. Verified by build, `./check.sh` (212 tests), 22 targeted model/query tests, 3 GTK tests (actual menu directions/order/selection, search/mode restoration and large-folder navigation), screenshots and native synthetic-library scroll/switch checks. Logs: `.tmp/sort-direction-{before,after,native}.jsonl` (debug sanity only). Included in the amended cleanup commit; not deployed.

**Next:** propose Compare candidate thumbnails and a modified shortcut, then the other remaining ideas (I1, I5, I10, I13). The one-library-root change remains separately planned. Close this cleanup first.

## Earlier work and acceptance

- Phases 5–7 are implemented on `viewer-reset`; Phase 8 Compare is accepted (summary in `docs/archive/history.md`); the user's `ef91d32` test passed 17/19 items. Details and outstanding acceptance checks are in `ROADMAP.md` Phases 5–7. A measured browsing check during indexing is still open; browsing felt smooth to the user.
- Fixes after that test are committed and verified: Ctrl+A (`9c6eb14`), picker click-away (`15b5d7e`, subsequently replaced by popovers), trash Undo (`475249c`). Not deployed.
- Open choices: file-name matches first in search (keep for now); whole-folder thumbnail preload versus a cap; automatic-tag cleanup (ready, never run).

## Git, deployment, and guardrails

- Development: `viewer-reset`. `main` remains at accepted Phases 1–2 (`1924346`). Archive branches `preserve/october-12-wip`, `visual-search-trial-wip`, and `look-prototype` stay.
- Installed launcher: `ef91d32` under `~/.local/opt/sharpr/current`. **Do not install now** (user, October 15). Build/test in the checkout; deployment, merge, and push require explicit approval.
- Latest recorded verified backup: `~/.local/share/sharpr.backup-20261014-150245`, with settings. Earlier backups remain. Live buckets/tags/settings are irreplaceable; never use them in tests.
- Isolated hands-on trial: `.tmp/look-trial/`, with copies of library/data/cache/settings. Pass the user's icon directory through in future trials so the icon theme remains available.
- Run heavy jobs one at a time; past parallel jobs exhausted memory. GTK tests use isolated directories and memory GSettings (`docs/reference/development.md`).
