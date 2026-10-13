# Current Tasks

## Active Milestone

Phase 2 follow-ups from the acceptance log (`ROADMAP.md`, Phase 2 status):
1. **Viewer stalls on large images: fixed** in `f619d64` (committed and verified, not deployed). Very large images are shown as screen-sized copies; full resolution loads on zoom.
2. **Slow folder-open read (2.9 s once):** cause unknown. `8cb13f5` logs slow database-connection waits (`index.pool_wait`) and the cached-read time. It needs a session log from a deployed build to find the cause.

Phases 1–2 were accepted on October 13. The Phase 0–2 history is in `docs/archive/history.md`.

## State (October 13, 2026)

- **Git:**
  - `main` is at `1924346`: the accepted build `66f3d94` plus its notes.
  - `viewer-reset` continues from there with the audit fixes and later work.
  - Nothing is pushed. Pushing is the user's call.
- **Deployed:** `e9c142f` (accepted work plus the audit fixes) at `~/.local/opt/sharpr/e9c142f`, with `current` pointing to it. Not yet tested by the user.
  - **Smoke test:** run on a copy of the live data, with identical tags, buckets, and image list afterwards.
  - **Rollback:** `ln -sfn ~/.local/opt/sharpr/66f3d94 ~/.local/opt/sharpr/current`.
  - Redeploy only with approval, via `python3 tools/install-native-launcher.py`.
- **Backups**, each byte-verified and integrity-checked:
  - `~/.local/share/sharpr.backup-20261013-090919/`, taken before `e9c142f`: 7,718 tag rows, 6 buckets.
  - `~/.local/share/sharpr.backup-20261013-042010/`
  - `~/.local/share/sharpr.backup-20261012-221640/`
- **Live data:** 6 buckets (Model, Art › Diffusion, upgrade?, CSR, Trash?). Of the 7,718 tag rows, 7,346 are old automatic tags; the cleanup is ready but not run.
- **Archive branches, kept:** `preserve/october-12-wip` and `visual-search-trial-wip`.
- **Product direction:** `docs/reference/product.md` (D1–D21). Tags are frozen; the Tags page has been removed (D20).

## Next User Check (Build `e9c142f`, or `f619d64` If Deployed)

1. Scroll fast in Wallpapers and switch between big folders; it should feel as before.
2. Open a bucket and start typing a search straight away; the results come from that bucket.
3. Add a visible image to a coloured bucket (Ctrl+B); its number pill takes the colour at once.
4. Remove an image from Art; the corner chip no longer lists Art › Diffusion.
5. With `f619d64`: open a very large image (8K or bigger). It appears without a freeze and looks sharp. Zooming in to 100% sharpens it fully, possibly after a short pause.

## Performance Baseline (Phase 0, Build `5832010`, October 12)

The log is `~/.cache/sharpr/logs/run-1791843839575-1067250.jsonl`.

| Measure | Result |
| --- | --- |
| Startup to first folder ready | 498 ms |
| Folder open, 2,216 images | 116–162 ms; one outlier at 2,449 ms (recurred on October 13, cause open) |
| Folder open, 408 images | 34 ms |
| Thumbnail worker time | p50 1 ms, p95 4 ms, max 293 ms |
| Memory (RSS) | 243 MB at start, 2,345 MB peak, 1,382 MB at end |

## Waiting On The User

- **Phase 3:** judge the search results in `.tmp/search-judge.html` (blind, 908 results), save `search-judgments.json`, then run `tools/benchmark-score.py` (command in `docs/reference/development.md`). That decides visual search (Phase 4/5) and tag retirement.
- **Thumbnail memory:** keep whole-folder preload, or cap it. Decide after measuring the deployed build (`ROADMAP.md` Phase 2, item 8).
- **Phase 4 (fingerprint-keyed buckets):** starts only after Phases 1–2 are accepted, because it migrates bucket data.
- **Independent audit (October 13):** `docs/archive/audit-2026-10-13.md` (Codex `gpt-6.1-sol`, 12 findings).
  - Findings 1–6 and 8–12 are fixed and deployed in `e9c142f`. The judging sheet was rebuilt, and the scorer now needs `--manifest`.
  - Finding 7 (thumbnail generations) is in the Backlog.

## Guardrails

- Test in isolated directories (`docs/reference/development.md`); never touch the live data directory.
- Commit each unit. No push, merge into `main`, or deployment without approval.
