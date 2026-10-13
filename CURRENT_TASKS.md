# Current Tasks

## Active Milestone

Acceptance of Phases 1–2. The work is done and committed. Release build `66f3d94` was deployed on October 13 for the user's test. Next: the user's manual check, a performance comparison, and the user's decision on fast-forwarding `main`. The Phase 0–2 history is in `docs/archive/history.md`.

## State (October 13, 2026)

- **Git:**
  - `viewer-reset` is the development line, 30+ commits past `main`.
  - `main` is still at the Phase 0 close; fast-forwarding it waits for acceptance and the user's approval.
  - Nothing is pushed. Pushing is the user's call.
- **Deployed:** `66f3d94` at `~/.local/opt/sharpr/66f3d94`, with `current` pointing to it.
  - **Rollback:** `ln -sfn ~/.local/opt/sharpr/5832010 ~/.local/opt/sharpr/current`. The previous build ignores `user_version`, so rolling back is safe.
  - Redeploy only with approval, via `python3 tools/install-native-launcher.py`.
- **Backups:**
  - `~/.local/share/sharpr.backup-20261013-042010/`: before `66f3d94`, byte-verified, integrity-checked.
  - `~/.local/share/sharpr.backup-20261012-221640/`
- **Live data:** 4,544+ indexed images and 6 buckets (Model, Art › Diffusion, upgrade?, CSR, Trash?). Of the 7,718 tag rows, 7,346 are old automatic tags; the cleanup is ready but not run.
- **Archive branches, kept:** `preserve/october-12-wip` and `visual-search-trial-wip`.
- **Product direction:** `docs/reference/product.md` (D1–D21). Tags are frozen; the Tags page has been removed (D20).

## User's Acceptance Check (Build `66f3d94`)

1. Scroll fast in Wallpapers and switch between big folders. It should feel at least as fast as before.
2. Search `mountains 4k png 16:9 `: the filter words become chips. Also try `below 4k`, and type `4` to see suggestions.
3. Press Delete while typing in search; nothing is trashed.
4. Right-click a thumbnail for Info, Crop, WebP, or Resize Copy and the file actions. Shift+F10 opens the same menu, and Ctrl+I opens Info.
5. Select images and press Ctrl+B to add them to a bucket.
6. The corner chip shows size, format, "Below 4K", and buckets. All 6 buckets are intact, including Art › Diffusion.
7. Optional: main menu → Library → Remove Old Automatic Tags…, which backs up first.
8. Renaming onto an existing name is refused.

**Afterwards:** run `python3 tools/bench-summary.py` on the session log and compare with the baseline below. Acceptance criteria: no main-loop stall over 100 ms while browsing, and no more than a 10% regression.

## Performance Baseline (Phase 0, Build `5832010`, October 12)

The log is `~/.cache/sharpr/logs/run-1791843839575-1067250.jsonl`.

| Measure | Result |
| --- | --- |
| Startup to first folder ready | 498 ms |
| Folder open, 2,216 images | 116–162 ms; one outlier at 2,449 ms (fixed since) |
| Folder open, 408 images | 34 ms |
| Thumbnail worker time | p50 1 ms, p95 4 ms, max 293 ms |
| Memory (RSS) | 243 MB at start, 2,345 MB peak, 1,382 MB at end |

## Waiting On The User

- **Phase 3:** judge the search results in `.tmp/search-judge.html` (blind, 908 results), save `search-judgments.json`, then run `tools/benchmark-score.py` (command in `docs/reference/development.md`). That decides visual search (Phase 4/5) and tag retirement.
- **Thumbnail memory:** keep whole-folder preload, or cap it. Decide after measuring the deployed build (`ROADMAP.md` Phase 2, item 8).
- **Phase 4 (fingerprint-keyed buckets):** starts only after Phases 1–2 are accepted, because it migrates bucket data.
- **Independent audit (October 13):** `docs/archive/audit-2026-10-13.md`, by Codex `gpt-6.1-sol`, 12 findings. Fixed on October 13 (committed and verified, **not deployed**: `66f3d94` doesn't include them):
  - 1, 2: renaming or moving onto a vacated name kept tags; renames use the kernel's atomic no-replace
  - 3: searching while a bucket loads
  - 4: bucket membership in the Info chip
  - 5, 6: badge colours on visible rows; one store update per view
  - 8–10: judging and scoring tools. The judging sheet was rebuilt; the scorer now needs `--manifest`.
  - 11: cleanup dialog wording
  - 12: docs
  - Finding 7 (thumbnail generations) is in the Backlog.
  - A redeploy needs the user's approval, ideally after today's test.

## Guardrails

- Test in isolated directories (`docs/reference/development.md`); never touch the live data directory.
- Commit each unit. No push, merge into `main`, or deployment without approval.
