# Current Tasks

## Active Milestone

**Phase 5: visual search and tag retirement** (`ROADMAP.md`). Deployed in `c176934` on October 14 and in use; not yet accepted.
- **Built:** indexing and search (`58ac0ef`), tag retirement (`78a0639`), packaging (`b07a226`), and GPU indexing (`c176934`).
- **The user's test so far:**
  - Search while indexing: "working really well, I am totally impressed and happy".
  - Browsing during GPU indexing "feels smooth". This is a manual check; the measured 10% check is still open.
  - The real library is fully indexed: 5,178 vectors (identical copies share one) and 0 failures. It has no AVIF or HEIC files.
- **Next, in order:**
  1. ~~Fix the Phase 5 audit's findings~~: done October 14 in six commits (`df82623`..`cf438a9`), verified, and **not deployed**. Deploying needs the user's approval.
  2. The Library Overview.
  3. Search into buckets, with selection tools.
  4. Negative search (`tower -water`).
- **Open user decision:** keep file-name matches first in results. They stay for now; the user will judge after use.

## State (October 14, 2026)

- **Git:**
  - `main` is at `1924346`, the accepted Phases 1–2.
  - `viewer-reset` carries everything since.
  - Pushed to GitHub on October 14 at the user's request.
- **Deployed:** `c176934` at `~/.local/opt/sharpr/c176934`, with `current` pointing to it.
  - The model and both runtimes are installed in `~/.local/share/sharpr-models/embeddinggemma-2/` (`runtime/` for the processor, `runtime-gpu/` for Vulkan). In the Flatpak, the server is `/app/libexec/sharpr-llama/llama-server`.
  - The build changes no schema. Its additions are `visual-search.sqlite`, one `schema_meta` key, and the GSettings key `visual-search-gpu`.
  - **Rollback:** close Sharpr, then run `ln -sfn ~/.local/opt/sharpr/bf12e07 ~/.local/opt/sharpr/current`. No data restore is needed.
  - Redeploy only with approval, via `python3 tools/install-native-launcher.py`.
- **Live data:**
  - 9 buckets: the user added 3 on October 14.
  - 7,718 tag rows, unused. Of these, 7,346 are old automatic tags; their cleanup is ready but not run.
  - Bucket membership was migrated by `bf12e07` (151 members across the original 6 buckets).
- **Backups**, each byte-verified and integrity-checked, in `~/.local/share/`:
  - `sharpr.backup-20261014-015921`, taken before `c176934`
  - `sharpr.backup-20261013-111041`, taken before `bf12e07`
  - three older ones
- **Archive branches, kept:** `preserve/october-12-wip` and `visual-search-trial-wip`.
- **Machine notes:**
  - GNOME Shell leaked about 6.9 GB of graphics memory until blur-my-shell and the wallpaper slideshow were turned off; which one caused it is not known yet.
  - Run heavy jobs one at a time inside `systemd-run --user --scope -p MemoryMax=9G`. On October 13, parallel jobs ran out of memory and the kernel killed the terminal.

## Waiting On The User

- **Thumbnail memory:** keep whole-folder preload, or cap it (`ROADMAP.md` Phase 2, item 8).
- **Automatic-tag cleanup:** run Main menu → Library → Remove Old Automatic Tags… whenever convenient.

## Performance Baseline (Phase 0, Build `5832010`, October 12)

| Measure | Result |
| --- | --- |
| Startup to first folder ready | 498 ms |
| Folder open, 2,216 images | 116–162 ms; one outlier at 2,449 ms (cause open) |
| Thumbnail worker time | p50 1 ms, p95 4 ms, max 293 ms |
| Memory (RSS) | 243 MB at start, 2,345 MB peak |

## Guardrails

- Test in isolated directories (`docs/reference/development.md`); never touch the live data directory.
- Commit each unit. No merge into `main` and no deployment without approval; push only when the user asks.
