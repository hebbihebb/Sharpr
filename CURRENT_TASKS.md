# Current Tasks

## Active Milestone

**Phase 5: visual search and tag retirement** (`ROADMAP.md`). Started October 13. Phase 4 is deployed in `bf12e07`, not yet checked by the user.

Plan, one commit per unit:
1. **Runtime and index (GTK-free `src/visual_search/`).** Done with unit 2 in one commit (committed, verified, not deployed).
   - Model folder `$XDG_DATA_HOME/sharpr-models/embeddinggemma-2/` (D23) with the two GGUF files and `runtime/llama-server`. Flatpak: `/app/bin/llama-server`.
   - `llama-server` children on `127.0.0.1`, a random port and key (D24), stopped when idle. Indexing runs one with the projector at nice 19 and 2 threads. Queries use a second, text-only one, so a search never waits behind an image.
   - Images are prepared like the benchmark: oriented, fit within 768×768, RGB PNG.
   - Vectors go in `$XDG_DATA_HOME/sharpr/visual-search.sqlite`, keyed by fingerprint, model hash, and preprocessing version. Failures are recorded, not retried every pass.
   - Indexing passes run after each fingerprint pass and are resumable. Preferences shows the coverage or why indexing is off.
2. **Search.** Done with unit 1. Still open: the in-app parity run over the 1,000-image sample (`app_pipeline_benchmark_report`); a 10-image pilot matched the benchmark vectors at cosine 0.997 or better. Words go to visual search when a model is installed.
   - Ranking happens after scope, disabled folders, and chips.
   - Results: the top 200, then "load more".
   - Coverage and match strength are shown; strength is calibrated on the judged sample.
   - Name search stays as the fallback.
   - Check parity with the Phase 3 results inside the app.
3. **Tag retirement.** Done (committed, verified, not deployed). The tag popover, Ctrl+T, on-image pills, tag search, and the extra-tags field are gone. Main menu → Library → "Turn Tags into Buckets…" offers each unused manual tag, none ticked; a one-time toast points to it. On the live data that is 11 tags (`nsfw` 88, `nude` 88, `mountain` 8, `nature` 8, and smaller ones). Tag rows stay.
4. **Packaging.**
   - The installer places the model and runtime.
   - A llama.cpp module goes in the Flatpak manifest, without network features.
   - Update the docs.

## State (October 13, 2026)

- **Git:**
  - `main` is at `1924346`: the accepted build `66f3d94` plus its notes.
  - `viewer-reset` continues from there with the audit fixes and later work.
  - Nothing is pushed. Pushing is the user's call.
- **Deployed:** `bf12e07` (Phases 1–4, the audit fixes, and the large-image viewer fix) at `~/.local/opt/sharpr/bf12e07`, with `current` pointing to it. Installed on October 13 at 11:11 with the user's approval; not yet tested by the user.
  - **First launch** upgrades bucket membership after its own backup (`~/.local/share/sharpr/backups/before-bucket-membership-*`). After that, older builds refuse the index.
  - **Smoke test:** run on a copy of the live data. The upgrade gave every bucket identical members, and all 7,718 tag rows were byte-identical.
  - **Rollback:**
    1. Close Sharpr.
    2. Restore `~/.local/share/sharpr.backup-20261013-111041/`.
    3. Run `ln -sfn ~/.local/opt/sharpr/e9c142f ~/.local/opt/sharpr/current`.
  - Redeploy only with approval, via `python3 tools/install-native-launcher.py`.
- **Backups**, each byte-verified and integrity-checked:
  - `~/.local/share/sharpr.backup-20261013-111041/`, taken before `bf12e07`: 7,718 tag rows, 6 buckets.
  - `~/.local/share/sharpr.backup-20261013-090919/`, taken before `e9c142f`: 7,718 tag rows, 6 buckets.
  - `~/.local/share/sharpr.backup-20261013-042010/`
  - `~/.local/share/sharpr.backup-20261012-221640/`
- **Live data:** 6 buckets (Model, Art › Diffusion, upgrade?, CSR, Trash?). Of the 7,718 tag rows, 7,346 are old automatic tags; the cleanup is ready but not run.
- **Archive branches, kept:** `preserve/october-12-wip` and `visual-search-trial-wip`.
- **Product direction:** `docs/reference/product.md` (D1–D24). Tags are frozen; the Tags page has been removed (D20). Visual search goes ahead with EmbeddingGemma 2 (D22).

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

- **Phase 3:** relevance judged on October 13. Gemma passed 30/30 and was chosen (D22); see `ROADMAP.md`. Still open: the indexing-impact check and the llama.cpp packaging check (Phase 3, items 4–5). Then come Phase 4 (fingerprints) and Phase 5 (visual search, tag retirement).
- **Thumbnail memory:** keep whole-folder preload, or cap it. Decide after measuring the deployed build (`ROADMAP.md` Phase 2, item 8).
- **Phase 4 (fingerprint-keyed buckets):** starts only after Phases 1–2 are accepted, because it migrates bucket data.
- **Independent audit (October 13):** `docs/archive/audit-2026-10-13.md` (Codex `gpt-6.1-sol`, 12 findings).
  - Findings 1–6 and 8–12 are fixed and deployed in `e9c142f`. The judging sheet was rebuilt, and the scorer now needs `--manifest`.
  - Finding 7 (thumbnail generations) is in the Backlog.

## Guardrails

- Test in isolated directories (`docs/reference/development.md`); never touch the live data directory.
- Commit each unit. No push, merge into `main`, or deployment without approval.
