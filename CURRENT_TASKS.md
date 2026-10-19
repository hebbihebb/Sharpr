# Current Tasks

## Accepted: AVIF and HEIC filmstrip thumbnails (Work Order item 30)

Branch `fix/avif-thumbnails`. The thumbnail worker decodes AVIF and HEIC/HEIF
through glycin's HEIF loader (`prepare::decode_with_glycin`, shared with
visual search): sandboxed, upright, size-checked against the decode budget.
Other formats are unchanged. Test: glycin-written AVIF and HEIC samples give
160 px thumbnails of the right colour (fails without the fix). The five real
AVIFs in `~/Pictures/avif test` take 60–250 ms each on a worker, then come
from the disk cache. Deployed as `637d98f`; accepted by the user October 19
and merged to `main`. Survey retirement recorded as D43 / item 31 (user).

## Accepted: drop Ctrl+Shift+S for Survey (Work Order item 19)

Branch `fix/survey-shortcut`. Sharpr no longer binds Ctrl+Shift+S: the
window shortcut and the image menu's accelerator label are gone, and the
Shortcuts help, manual, README and `ui-gtk.md` no longer list it. Survey stays
in the image menu with no shortcut. The Survey GTK test now proves no shortcut
controller or application accelerator in the window claims the key (it fails
with the old binding) and enters Survey through the menu action.
`./check.sh` passes. Deployed as `a886c27`; accepted by the user October 19
and merged to `main`.

## Accepted: AVIF and HEIC in visual search (Work Order item 22)

Deployed as `6ff4a78`; accepted by the user October 19, merged to `main` and
pushed. AVIF and HEIC/HEIF are decoded for
indexing by glycin's HEIF loader (`prepare::decode_with_glycin`), the loader
the viewer already uses: sandboxed, no new dependency, and shipped by the
GNOME 50 runtime (checked in the installed Flatpak), so the Flatpak size cost
is nil. Size-checked against the 3 GB decode budget before decoding. Without
the loader an image counts as unsupported and is retried each pass, as are
images older builds turned away. Tests: glycin-written AVIF/HEIC samples
prepare upright at 768 px; with the installed model, both index and "green"
ranks them first. `./check.sh` (317 tests) and 41 GTK workflows pass.
Bug logged as Work Order item 30 (user confirmed): AVIF/HEIC show no
filmstrip thumbnail, though the viewer opens them. Work stopped here for now.

## Accepted: audit 0.5.0-2 fixes (A01–A09)

Deployed as `50ee6bc`; accepted by the user October 19 ("seems to be in working
order"), merged to `main` and pushed.

Branch: `codex/audit-0.5.0-2`, based on `bed0278` from `main`.
The user agreed to A01–A09; review decisions are committed as `0ab1934`.

- A01–A05 `6f44cba`: cleanup retains uncertain membership; complete unchanged
  coverage and positively observed containing-directory identities are required.
  Scope changes defer without blocking GTK while membership writes commit.
  Trashed directories protect descendants; trash read errors withhold cleanup.
- A06 `f8509f1`: backed-up upgrades carry unconverted legacy members directly
  into membership, preserving tags and historical rows.
- A07 `eddd30d`: all strong matches form a fixed sorted block; Show More adds weak matches.
- A08 `6c62560`, `86b6167`: root switch rows retain keyboard focus during state refresh.
- A09 `59e1f9e`: retired hash bookkeeping removed; legacy values preserved.

Verification: `./check.sh` passed (315 regular tests, formatting, strict Clippy,
dependency policy and unused-dependency checks). Off-screen GTK: 40 workflows
passed together; the remaining Settings synchronization regression was fixed
and passed its rerun. Real-model search passed. Formatting/whitespace passed
again; `cargo build --jobs 4` passed (11.38 s). All heavy work ran serially
under the 9 GiB cap with four jobs and isolated settings/data.
Tests use isolated data/settings only. No live membership or originals touched.
One independent data/concurrency review completed; its containing-directory
case was reproduced and fixed. No schema or dependency change.

Cleanup is conservative: already-missing members after restart stay until
observed again; excluded/unreadable/changed locations retain membership.
External filesystem writers cannot be locked into an atomic library snapshot.
No new manual large-folder performance benchmark or Flatpak Undo trial.

Findings, review decisions and detailed fix evidence remain in git history of
`AUDIT.md`; the report is removed before merge, per the audit process.

## Remaining work

User decisions (October 19): item 12 dropped; item 19 becomes "remove the
Ctrl+Shift+S Survey shortcut" (the Inspector bug persists, not chased); item
22 decoders approved, Flatpak size no concern; item 21 waits for a planning
session; item 24 skipped for now. Next after the audit fixes: 19, then 22.
Prior accepted work is summarized in `docs/archive/history.md` and the roadmap.
