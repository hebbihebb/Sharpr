# Current Tasks

## Active Milestone

**Phase 5: visual search and tag retirement** (`ROADMAP.md`). All units are committed and deployed in `c176934` (October 14). The user tested search while indexing ran: "working really well, I am totally impressed and happy". Browsing during GPU indexing "feels smooth" (user). Still to build, after Sol's review of `87ca43e..bd250fb`: the Library Overview, then search into buckets with selection tools (ROADMAP Phase 5). The user is still to decide whether file-name matches stay first in results; they stay for now.

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
4. **Packaging.** Implemented: `tools/install-visual-search-model.py` (tested in a scratch folder) and a `llama-cpp` Flatpak module. The Flatpak builds (36 MB for `llama-server` and its libraries), and inside the sandbox it embedded an image and a query from the model folder (verified October 14; not installed).
- **Parity (October 14):** the app's CPU pipeline passes 30/30 against the user's judgments (median cosine 0.9995 to the benchmark). The GPU (Vulkan, RTX 4060) is 17× faster with matching vectors. GPU indexing is now the default: `runtime-gpu/` in the model folder, a Preferences switch, and the processor as fallback (committed, verified with the real model on the 4060).
- **GNOME Shell's graphics-memory leak:** it held 6.9 GB of the 4060's 8 GB, and dropped to 0.55 GB when the user turned off blur-my-shell and the wallpaper slideshow. Which one caused it is not known yet.
- **Crash on October 13, 15:57:** the Flatpak build, the parity run, and rust-analyzer together ran out of memory, and the kernel killed the terminal. Nothing was lost. Run heavy jobs one at a time, inside `systemd-run --user --scope -p MemoryMax=9G`.
   - The installer places the model and runtime.
   - A llama.cpp module goes in the Flatpak manifest, without network features.
   - Update the docs.

## State (October 13, 2026)

- **Git:**
  - `main` is at `1924346`: the accepted build `66f3d94` plus its notes.
  - `viewer-reset` continues from there with the audit fixes and later work.
  - Nothing is pushed. Pushing is the user's call.
- **Deployed:** `c176934` (all of Phase 5, with GPU indexing) at `~/.local/opt/sharpr/c176934`, with `current` pointing to it. Installed on October 14 at 02:00 with the user's approval; not yet tested by the user.
  - The model and both runtimes are installed in `~/.local/share/sharpr-models/embeddinggemma-2/` (approved). The first launch indexes the library on the RTX 4060, about 20 minutes.
  - The previous build `bf12e07` had already migrated bucket membership on the live data: 151 member rows, matching all 6 buckets.
  - This build changes no schema. Its new files are `visual-search.sqlite` and the model folder; it adds one `schema_meta` key (the tag offer notice) and one GSettings key.
  - **Rollback:** close Sharpr, then run `ln -sfn ~/.local/opt/sharpr/bf12e07 ~/.local/opt/sharpr/current`. No data restore is needed.
  - Redeploy only with approval, via `python3 tools/install-native-launcher.py`.
- **Backups**, each byte-verified and integrity-checked:
  - `~/.local/share/sharpr.backup-20261014-015921/`, taken before `c176934`: 7,718 tag rows, 6 buckets, 151 members.
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
