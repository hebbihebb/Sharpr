# Current Tasks

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
