# Current Tasks

## Active: Phase 9 interface review (October 15, 2026)

The user asked Codex to review Claude's UI work and continue. Started from a clean `viewer-reset` at `b7241f7`.

**Approved direction:** hybrid GTK4/Libadwaita (D26), then Sharpr's design language (D27): monospace everywhere, aligned 42 px top bars, compact menus and popover pickers, narrow sidebar, neutral image canvas, hover/movement readout, filter chips and one Sort and Filter menu. Implemented in `cb57749`..`52595e5`; the user liked the isolated hands-on trial. Not deployed.

**Accepted:** Preferences and Keyboard Shortcuts now use styled sheets (`8b6ac1d`), retaining settings wiring and nested editors. Verified by build, full checks, and 12 GTK tests. Accepted in the October 15 hands-on trial; not deployed.

**Accepted unit:** bundled JetBrains Mono v2.304 variable fonts, upright and italic (weights 100–800), plus OFL licence/source checksums. Embedded in GResource and loaded by Pango from anonymous process-owned files; no desktop font installation or runtime downloads. CSS prefers the bundled family; About includes its licence. Implemented and verified by `./check.sh` (211 regular tests, strict clippy, deny, machete), all 13 GTK tests including one with system fonts excluded, and light/dark screenshot review at 1024×600 and 1600×900. Cargo sources regenerated unchanged; no crate versions changed. Committed as `0183d3f`; accepted in the October 15 UI trial, not deployed.

**Accepted unit (D28):** contextual Compare/crop headers with native left/center/right groups. Linked zoom, arrow keys and matching Keep buttons; crop aspect, live dimensions, destination, Cancel and Save Copy. Browse search/chips survive; delayed search/sidebar signals and adaptive resizes leave the mode intact. Verified by `./check.sh` (211 tests), all 14 GTK tests, new-copy/original-byte checks, and light/dark screenshots at 1024×600 and 1600×900. The old picker test had intermittent timing failures; final full suite passed, with follow-up in `ROADMAP.md` Backlog. Committed as `c5699d7`. The user accepted the hands-on checks on October 15: choices/Undo, modes, linked zoom/pan, survivors and search/chip restoration. Not deployed. Candidate thumbnails remain a separate proposal.

**Accepted: WebP export sheet** (`fde5a52`, user test October 15). The compact form stays on the main window's surface during folder selection; the failed popup recreation and recovery toast are removed. Click-away still cancels. Verified by full checks (211 tests), all 14 GTK tests, Select/Cancel and same-surface checks, actual two-image export, original-byte preservation, and light/dark screenshots at both sizes. Crop also worked for the user. Not deployed. The subsequent Compare and Preferences/Shortcuts checks also passed; the current UI units are accepted. Phase 9 remains active for the planned work below.

**Next implementation:** remove the hidden old Filter popover. Then propose the remaining behavior ideas (I1, I3–I5, I10, I13) before implementing them. The full plan, outstanding observations, user reactions, and acceptance criteria are in `ROADMAP.md` Phase 9. The one-library-root change remains planned there.

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
