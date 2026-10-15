# Current Tasks

## Paused: Phase 9 interface review (October 15, 2026)

**User instruction:** stop here and commit the current work. The goal is paused.
Installation and final manual acceptance remain pending; do not continue or install
until the user resumes. The desktop launcher still runs `ef91d32`.

**Approved scope:** complete Phase 9, then install its test build, with off-screen
screenshots preferred (D30). The user explicitly confirmed Survey as a way to
narrow a selection. D26–D29 define the accepted hybrid look, monospace font,
contextual Compare/crop controls and candidate thumbnails. Earlier accepted UI,
export and filter/sort work is recorded in `ROADMAP.md`.

**Completed checkpoints on `viewer-reset`:**

1. `e91136b`: private off-screen Mutter runner; larger native fast-scroll handle;
   strict `<16:9`/`>16:9` chips; bucket terminology.
2. `27789cb`: persistent library-scoped last bucket (Ctrl+Shift+B); hover-expanded
   Info in the same readout, pin/Escape, and live membership refresh.
3. `88e5e41`: Survey grid (Ctrl+Shift+S), dismiss/Delete, Undo and Done/Escape;
   original order, survivor selection, search/chips and image bytes preserved.
4. `ffa075d`: one root chooser and recursive browsing; active identity, inactive
   configurations, exclusions, bucket members, fingerprints and vectors survive
   narrowing/widening. No library switcher/list/folder-mode or bucket transfer UI.
5. **Final checkpoint, implemented and verified:** one stylesheet/provider and
   shared palette/drawing tokens; seven original CC0 symbolic icons; CSS thumbnail
   feedback; compact Help; shared viewer/filmstrip image actions; retired overlay,
   cache and index-ring controls, preserving settings. The control dispositions
   are in `docs/reference/interface-inventory.md`. The milestone self-review is
   complete: fixed hidden-row keyboard-menu targeting and large Survey lookup cost.

**Verification:** `cargo build`, full `./check.sh` (217 tests), all 18 GTK workflows,
6 focused high-contrast workflows and 2 final OSD/menu checks passed. Screenshots
are off screen, in light/dark/high contrast at 1024×600 and 1600×900. Logs and
captures: `.tmp/phase9/`. The same debug/Cairo 5,000-image scenario opened in
345 ms before / 351 ms after; browsing's longest recorded stall was 87 / 85 ms.
These are isolated debug measurements, not release performance claims.

**On resume:** finish recording this checkpoint's commit if needed, install the
clean pinned test build using the previously authorized native launcher, and
request the user's final manual pass. Phase 9 remains open until accepted; do not
archive it or advance to Phase 10 yet. No further review loop is needed.

## Git, backup and earlier acceptance

- Development: `viewer-reset`; `main` stays at `1924346`. No merge or push approval.
  Keep all archive/prototype branches.
- Verified live backup: `~/.local/share/sharpr.backup-20261015-205204`, including
  settings. All database table counts match: 815 bucket members, 5,899 fingerprints,
  5,184 vectors and 7,718 legacy tag rows. No live Sharpr process was running.
- Phases 5–7 still have acceptance items in `ROADMAP.md`; Phase 8 is accepted and
  archived. Automatic-tag cleanup is implemented but has never been run.
- Tests use isolated temporary data/settings. Heavy jobs run one at a time.
