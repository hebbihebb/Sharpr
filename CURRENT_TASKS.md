# Current Tasks

## Active Milestone

Phase 0 repository checkpoint is committed and verified on `viewer-reset`. Operational Phase 0 acceptance remains deferred; do not start Phase 1 yet.

## Verified Checkpoints

- `515e2c5`: patched dependencies; default runtime excludes unfinished trial wiring.
- `4e412e3`: safe copy services and displayed dimensions.
- `4af3857`: scoped browsing, filters, manual tags and bucket persistence.
- `6bb3ac5`: Crop/WebP dialogs and created-copy selection.
- `4cbefd2`: development-only benchmark tools and research notes.

Each checkpoint builds from its own committed source. Final checks: `cargo build`, `./check.sh` (135 tests, strict clippy, deny, machete), and two isolated GTK tests covering search/filters/copies plus rapid navigation/folder switching. Each GTK process now gets fresh fixture and XDG directories, preventing bucket-name collisions when rerunning the suite. Synthetic navigation passes before and after the October work; it is not a real-library release-performance measurement. Logs/artifacts: `.tmp/phase0/`.

## Preserved And Deferred

- Full original working tree: `preserve/october-12-wip` (`68858b3`). This is an archival snapshot, not a verified all-features build.
- Trial-only Cargo/lockfile/main wiring: `visual-search-trial-wip` (`48a0539`). The missing `semantic_trial.rs` is recorded; the trial remains unfinished. `viewer-reset` has no trial feature, new HTTP dependencies or runtime networking.
- Private model files, Python environment, sample manifest, vectors and original reports remain ignored under `.tmp/`; no experiment was rerun or discarded.
- Data-safety repairs, dimension-cache consolidation, truthful membership chips, crop-preview fallback and benchmark methodology remain in Phases 1, 2, 3 and 6 of the restructured roadmap. No new phase was implemented.

## Product Direction (Approved October 12)

After a product interview, the user approved `docs/reference/product.md` (decisions D1–D19), and the roadmap was restructured into Phases 0–9.

- **Manual tags are frozen:** no tagging improvements. They are retired only if visual search passes Phase 3.
- **Buckets move to fingerprint-keyed membership** in Phase 4, which survives files moved outside Sharpr.
- **Finding comes first** after the data-safety work: typed filter chips, start-typing search, the Info panel, and fast add-to-bucket.

Phase 0 and Phase 1 content is unchanged. Nothing was implemented.

## User Data And Deployment

All tests used memory GSettings and isolated data/config/cache/schema directories. Builds used `CARGO_TARGET_DIR=.tmp/phase0/target`; the existing debug launcher and its binary were not replaced. Main database-file/config/launcher/binary hashes remained unchanged. Live WAL/SHM files and GSettings changed while the user's existing Sharpr process was running, so idle live-data equivalence and a consistent closed-app backup were not verified. Never restore over that live activity.

The user's request expressly forbids deployment, merging, pushing and branch/worktree deletion. Those steps were not performed. `main` remains at `2679e44`; no existing branch or worktree was deleted. None of these new checkpoint commits is accepted, merged or deployed.

## Backup (Phase 0 Step 4, Done)

On October 12 at 22:16, with Sharpr closed, the data folder was copied to `~/.local/share/sharpr.backup-20261012-221640/`.

- **Contents:** everything in `~/.local/share/sharpr`, plus `gsettings.ini` (a dconf dump) and `config-sharpr/`.
- **Verification:**
  - every file is byte-identical to the original (SHA-256)
  - a scratch copy passes `integrity_check` on both databases
  - the copy holds 4,544 indexed images, 6 buckets, and 7,718 tag rows

**Observed incident: the Phase 1 tag-pruning bug.** Compared with the September 27 checkpoint, 81 image paths lost all their tags when Sharpr restarted at 21:29.
- 21 of them were files moved outside Sharpr (for example `Upscaled/` → `Diffusion/gpt wide/`).
- 5 paths carried manual tags (`exports`, `comics`), and those files no longer exist.
- Moved files lost only automatic tags, and no bucket lost a member, so nothing of value was lost this time.
- It confirms Phase 1 item 1 is urgent before buckets are relied on further.

## Deployment (Phase 0 Step 5, Done)

On October 12 at 22:22, with the user's approval, the launcher was switched to an installed release build.

- **Deployed commit:** `5832010`, at `~/.local/opt/sharpr/5832010/`, with `current` pointing to it.
- **The launcher** runs it with its own compiled schema. Development builds no longer change what the user launches.
- **Smoke test:** the installed build ran `--help`, then started against isolated data and kept running until it was stopped. The real database files were confirmed unchanged.
- **Cosmetic only:** a GTK warning about a `SharprFolderRow` popover not being unparented at teardown.
- **Old debug binary:** `sharpr/target/debug/sharpr` remains, unused.

## Next Authorized Work

Phase 0 steps 6–7: collect real-library release bench logs, obtain user acceptance, then separately authorize the fast-forward/cleanup. Do not infer that authorization from this checkpoint request. `ROADMAP.md` remains authoritative.
