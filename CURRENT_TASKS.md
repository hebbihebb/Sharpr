# Current Tasks

## Active: Phase 9 interface review (October 15, 2026)

The user asked Codex to review Claude's UI work and continue. Started from a clean `viewer-reset` at `b7241f7`.

**Approved direction:** hybrid GTK4/Libadwaita (D26), then Sharpr's design language (D27): monospace everywhere, aligned 42 px top bars, compact menus and popover pickers, narrow sidebar, neutral image canvas, hover/movement readout, filter chips and one Sort and Filter menu. Implemented in `cb57749`..`52595e5`; the user liked the isolated hands-on trial. Not deployed.

**Accepted UI units (October 15):** styled Preferences/Shortcuts (`8b6ac1d`), bundled JetBrains Mono (`0183d3f`), contextual Compare/crop controls (`c5699d7`), and the WebP export sheet (`fde5a52`). The user tested choices/Undo, viewing modes, linked zoom/pan, returning survivors/search/chips, output-folder selection and export, and light/dark/resized sheets: everything works. Phase 8 Compare is closed in `docs/archive/history.md`. All are committed and verified (latest full checks: 211 regular tests and 14 GTK tests); not deployed. Candidate thumbnails are the approved current unit (D29).

**Filter/sort cleanup accepted (`7399cee`):** chips are the only filter path; Ascending/Descending work; retained cached thumbnails survive sort/filter reloads. Build/full checks (212 tests), 22 targeted tests, GTK checks and native large-folder checks passed; the user retested successfully. Debug logs remain under `.tmp/filter-cleanup*` and `.tmp/sort-direction*` without release performance claims. Not deployed.

**Implemented and verified (D29), ready for user acceptance:** Compare now has the horizontal candidate row, L/R badges and explicit Replace Left/Right target defaulting to Right. Choosing preserves candidates; Keep removes and Undo restores original order. Dropdowns are removed; guarded Ctrl+Shift+C appears in the context menu and Shortcuts sheet. Visible rows reuse the thumbnail cache/decoder on one worker per session; GTK's extra prepared rows do not request decodes. Build, full checks (212 tests), all 14 GTK tests, cold-cache loading, 500-candidate virtualization/session checks, original-byte checks and light/dark screenshots at both sizes passed. Native 5,000-image scrolling/folder switching passed. Debug logs: `.tmp/compare-candidates-{before,after,native}.jsonl`; no release performance claim. Committed on `viewer-reset`; not deployed. Launch the isolated trial for the user's choice/swap/Keep/Undo/shortcut retest.

**Next:** other behaviour proposals (I1, I5, I10, I13). One-library-root remains separate. Filter cleanup is closed.

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
