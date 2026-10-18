# Sharpr Roadmap

This is the only roadmap. `CURRENT_TASKS.md` tracks which phase is active. When a phase finishes, summarize it in `docs/archive/history.md` and delete it from this file.

## Direction

The product vision, firm requirements, workflows, UX principles, non-goals, and decision log (D1–D19) are in `docs/reference/product.md`. The user approved them on October 12, 2026. In brief: Sharpr is a fast, offline app for finding, curating, and preparing wallpaper-grade images.

- **Keep:** the filmstrip, viewer, folders, and buckets.
- **Find:** visual search plus typed filters, replacing manual tags.
- **Prepare:** crop and batch WebP export.
- **Hand off:** heavy image work stays in external tools.

Tags are **frozen**: no improvements to tagging. They are retired only after visual search passes Phase 3 (D1). Until then, buckets still depend on tags, so the data-safety fixes for tags still apply.

## Architecture Decision: Continue, Refactor Where Touched, No V2

**Decided:** October 2026. Revisit only if one of the triggers below fires.

**Options compared:**
- **Continue unchanged:** risky because `window.rs` keeps growing.
- **Incremental refactor:** chosen.
- **Clean minimal V2** that reuses selected components: rejected.

**Evidence from the code:**

- **Size:**
  - about 25,600 lines total
  - about 3,300 lines of tests
  - about 1,200 lines compiled out with `#[cfg(any())]`
  - about 21,000 lines of production code
- **Where the complexity is concentrated:**
  - `ui/window.rs` has about 5,000 lines (one 2,117-line `setup()`), imports 30 modules, and has 153 commits, 53 of them fixes.
  - The GTK-free services (index, tags, export, JXL, metadata, quality, duplicates, bench, and config; about 4,200 lines) are cleanly separable already.
- **The parts the user values most carry the most embedded knowledge.** Filmstrip, thumbnails, `LibraryManager`, and viewer total about 5,500 lines, with 89 fix or performance commits covering stale generations, folder-switch hitches, placeholder handoff, and duplicate-open stalls. A V2 would either port this code almost verbatim, which makes it a refactor with extra steps, or rediscover those fixes.
- **User data pins the persistence design either way.** A V2 must read the same two databases (legacy columns, tag-backed buckets, path keys) and the same GSettings schema, or ship a migration importer. That importer is new data-loss risk, not less.
- **Every real defect found is local:** tag pruning, unchecked rename, bucket tag removal, worker self-send, and GTK-thread lookups. None comes from the architecture. Sharpr's architecture (GTK main thread, worker threads, channels, generation counters, SQLite) is what a V2 would choose again.
- **Hidden costs a V2 adds:**
  - a parity gap while the user keeps using V1
  - two apps that could write the same data
  - about 130 tests to rebuild
  - packaging and identity work done again
  - an agent-driven rewrite that tends to grow in scope

**What this means in practice:**
- Delete dead code (Phase 1).
- Add schema versioning (Phase 1).
- Move each area out of `window.rs` into `ui/window/` controllers when a phase changes it.
- Retiring tags (Phase 5) removes a whole UI layer instead of rewriting it.
- Do not run a separate refactor phase or feature freeze.
- Target: no new feature logic in `setup()`, and by Phase 10 no function longer than about 500 lines in the areas phases touched.

**Revisit triggers:** a fix requires replacing the thumbnail scheduler or `LibraryManager` wholesale, or GTK/Libadwaita changes force a broad UI rewrite anyway.

## Order

```text
Phase 1 ─┬→ Phase 2 Finding ──────────────────┐
         ├→ Phase 3 Visual-search evaluation ─┤
         └→ Phase 4 File identity + buckets ──┴→ Phase 5 Visual search + tag retirement (go only)
            then Phase 6 Wallpaper output → Phase 7 Cleanup → Phase 8 Compare
            → Phase 9 Interface review → Phase 10 Release
```

- **Phases 0 and 8 are done**; see `docs/archive/history.md` for their completion summaries.
- **Mandatory:** Phases 1, 2, 4, 6, 7, 8, 9, and 10. Phase 3 is a mandatory evaluation; Phase 5 happens only if Phase 3 says go.
- **Finding comes first** after the data-safety work (D19). Phases 2 and 3 can run in parallel, because Phase 3 is mostly human relevance judging.
- **Phase 4 is shared groundwork.** Phase 5 (vector identity) and Phase 7 (exact duplicates) both need it.
- **Phases 6–8 depend only on Phase 1.** The user may reorder them. Phase 6 needs nothing from Phase 4, so it can move earlier if wallpaper output becomes urgent.
- **Phase 9 (interface review) comes after the features,** by the user's choice (October 14): it reviews the whole interface once everything it must hold exists. Until then, new controls use standard placements and may move in Phase 9.

---

## Phase 1: Data-Safety Repairs

**Status:** accepted by the user on October 13, 2026 (build `66f3d94`); `main` is at `1924346`. The audit fixes of October 13 (`e9c142f`) have been in every deployed build since.

**Outcome:** No Sharpr action can silently lose tags, bucket membership, or image files. Every defect below is verified in the code and exists at committed `HEAD`.

**Scope:**

1. ~~Stop deleting tag rows for missing files.~~ Done October 12, including trash no longer deleting tags. `TagDatabase::all_tags()` and `paths_for_tag()` currently delete tags for every path that doesn't exist, and the Tags page calls them on every refresh. Files moved outside Sharpr, or an unmounted root, lose their tags permanently.
   - Filter missing paths when reading instead, and keep the rows.
   - Replace the test that enshrines pruning (`tags/db.rs`).
2. ~~Rename never overwrites.~~ Done October 12. Rename and Move to… use a no-replace rename, and tag-update failures are reported.
   - Reject path separators and `..`.
   - Refuse an existing target name (no-replace rename).
   - Report `rename_path` tag-update failures.
3. ~~Bucket edits remove only the bucket's own tags.~~ Done October 12 in `src/buckets.rs`. Phase 4 still moves membership off tags.
   - Removing an image from a child bucket must not strip inherited ancestor tags, which currently also drops the image from sibling buckets.
   - Reparenting drops a tag only when no remaining lineage needs it.
   - Renaming a parent includes members that are in disabled folders.
   - Add parent/child/sibling regression tests.
4. ~~Thumbnail workers cannot deadlock.~~ Done October 12 (`queue_hash`, `drain_stale`).
   - Workers must not `send_blocking` hash follow-ups into their own bounded queue (`thumbnails/worker.rs`).
   - The stale-drain loop must not discard hash or shutdown messages.
   - Add a stress test that exceeds the queue capacity.
5. ~~Add real schema versioning.~~ Done October 12: `user_version` 1 on both databases, newer databases refused untouched, and 1,304 lines of compiled-out legacy code deleted.
   - Set `PRAGMA user_version` on both databases.
   - A binary that finds a newer version refuses to write and shows an error.
   - Delete the compiled-out `#[cfg(any())]` pipeline code. The legacy tables stay.
6. ~~Guard broad actions.~~ Done October 12.
   - Tags page "Delete tag": confirm first, naming the affected buckets, and run off the GTK thread.
   - Reject bucket names that differ only by case.
   - Delete and other single-key shortcuts must not fire while a text entry has focus.
   - Report trash failures.

**Acceptance:**

- Each item has a regression test, or a stated manual check where a test is impractical.
- A test shows that a missing library root deletes nothing.
- A copy of the live databases opens and round-trips with identical tag and bucket counts under the new build.
- `./check.sh` passes.

**Not in scope:** merging the two databases, and crash-atomic bucket edits. Phase 4 removes buckets' dependence on tags, which closes that window.

## Phase 2: Finding

**Status:** accepted by the user on October 13, 2026 (build `66f3d94`; no issues in the manual check). The acceptance log (`run-1791881444880-10141.jsonl`, 823 s) did not meet two measured criteria, and the user chose to accept and fix them as follow-ups:
- **Startup:** 381 ms (baseline 498 ms).
- **Folder opens:** 2,216 images in 104–169 ms (baseline 116–162 ms).
- **Main-loop stalls:** 17 over 100 ms while browsing, the longest 434 ms. All but one came 0.1–1.1 s after the viewer opened an image, with RSS jumping by about 1.2 GB on very large images. That is the viewer's full-resolution texture reaching GTK. It predates Phase 2, which the baseline couldn't show (no stall probe then).
- **One Wallpapers open took 3,043 ms.** Its `images_in_folder` read on the dedicated folder-open thread took 2.9 s, at low CPU. So the October 12 outlier was not rayon queueing; the cause is still unknown.
- **RSS peak:** 3,383 MB (baseline 2,345 MB). The session applied 14,608 thumbnails (baseline about 8,000), plus the viewer jumps above.

**Follow-ups (next):**
- ~~Show large images in the viewer at screen size first, with full resolution only when zooming in.~~ Done October 13, deployed since `bf12e07`. For a 12,000 × 8,000 JPEG, the longest main-loop step fell from 0.8–2.2 s to under 40 ms.
- Time the pool wait and the query separately in the folder-open read. **Instrumented** (`8cb13f5`, `index.pool_wait`) and deployed; still needs one real session's log to find the cause of the multi-second wait.
- Re-measure on a release build and compare with both logs.

**Outcome:** In a library of 5,000–50,000 images, the user finds images quickly with one search box, typed filters, and a quiet interface, without stalling the filmstrip. Decisions: D5–D7, D12, D16–D18.

**Existing implementation:** committed in the Phase 0 checkpoint.
- scoped search with a whole-library toggle
- 150 ms debounce and a single coalescing worker
- aspect sort in both directions, plus closest to 16:9
- landscape, portrait, square, and 16:9 ±2% filters
- minimum width and height
- tag edits moved off the GTK thread
- automatic tags no longer created

**Scope:**

1. ~~Add a measuring tool first.~~ Done October 12: `mainloop.stall` probe plus `tools/bench-summary.py`. The first debug-build run showed startup stalls up to 639 ms. Add a main-loop stall probe to `bench.rs`, such as a lateness measurement on a periodic timer, plus a small log-summary script. This makes "doesn't stall" testable.
2. ~~Typed filter chips (D16).~~ Done October 12 (`model/search_query.rs`; the chips sit in a bar under the header). Chips combine (AND) with the filter popover rather than sharing its state. Suggestions while typing were added October 13 as a row under the search box (typing `4` offers `4k` and `4:3`; clicking one completes it into a chip).
   Original scope: Recognized words in the search box become removable chips, sharing state with the existing filter controls:
   - formats: `jpg`/`jpeg`, `png`, `webp`, `jxl`, `gif`, `tiff`, `bmp`
   - resolutions, meaning *at least*: `720p`, `1080p`, `1440p`, `4k`, `5k`, `8k`
   - below a resolution, such as below 4K
   - aspects: `16:9`, `21:9`, `16:10`, `4:3`, `3:2`, `square`, `landscape`, `portrait`

   The remaining text goes to name search; after Phase 5 it goes to visual search. An ambiguous word becomes a chip that turns back into text with one click. Suggestions appear while typing.
3. ~~Search from anywhere (D6, D7).~~ Done October 12. Typing anywhere and Ctrl+F already worked, and "Whole library" is visible; Space, `[` and `]` no longer open search.
   Original scope:
   - Typing anywhere opens search with that text; Ctrl+F also opens it.
   - The scope switch (this folder or bucket, or the whole library) is visible.
   - Space stays free.
4. **Quiet interface (D18).** Header overflow at 1024×600 is fixed: the Tags page and page switcher were removed on 2026-10-13 (D20), and a screenshot confirms the close button is visible. The trimmed chip and the Info panel (D18) and the right-click image actions (D21) were done on 2026-10-13. **Item 4 is done.**
   - Always visible: resolution and format (with a below-4K hint) and the image's real bucket membership, meaning every effective tag within the active library.
   - Everything else (filename, position, size, dates, EXIF) moves to an Info panel opened from a key or the context menu.
5. ~~Fast curation from results (D5, D10).~~ Done October 12. The right-click picker (with "New Collection…") and drag-to-sidebar already existed; Ctrl+B now adds the selection.
   Original scope: add the selection to a bucket by shortcut or by dragging onto a sidebar bucket, and create a new bucket from the selection.
6. ~~Retire quality views (D12).~~ Done October 12. Removed the Quality Filter menu, the orphaned `scan-quality` action, `ViewScope::Quality`, and the unused `filter_bar.rs` (about 500 lines). The resolution chip in the viewer stays.
7. ~~Clean up legacy automatic tags (D17).~~ Implemented October 12 as main menu → Library → "Remove Old Automatic Tags…". It backs up with `VACUUM INTO`, previews counts, and asks for confirmation. A preview on the live data showed 7,346 rows to remove and 372 manual rows kept. **The user runs it after deployment.**
   Original scope: 7,565 of the 7,925 live tag rows are format and resolution tags, and no bucket uses them.
   - Make it a one-time action with a backup, a count preview, and confirmation.
   - It removes only known automatic tag names that no bucket references.
   - Delete the dead `tags/indexer.rs`.
8. **Fix the Phase 0 test findings.**
   - ~~The aspect filter popover closes when the user clicks away.~~ Done October 12. The nested drop-down was replaced with radio buttons.
   - ~~Hide the background-activity indicator when idle.~~ Done October 12. It slides in only for work lasting over 0.4 s.
   - ~~A folder-open scan must never wait behind other jobs in the shared rayon pool.~~ Done October 12. The cause was one pool job per hash write (about 7,900 per session). Hashes are now batched on a writer thread, and folder scans run on their own thread.
   - **Thumbnail memory: decide with the user after measuring the deployed build.** Analysed October 12.
     - Memory does not accumulate across folder switches. It plateaus at about 1.3–1.5 GB (peak 2.3 GB) after a 2,112-image folder fully preloads.
     - Thumbnails average about 0.5 MB, because landscape system x-large thumbnails (512×288) are kept at full size. On the user's 2× display that keeps the filmstrip sharp, so shrinking them is not a free win.
     - The machine has 14 GB of RAM, about 4.5 GB free in use. The current folders are fine; folders of 10,000+ images would not be.
     - Options: cap the background preload to a window around the viewport and release far textures, or keep whole-folder preload and accept the cost.
9. ~~Decide with the user where image actions live.~~ Decided D21 (2026-10-13) and done. Image actions moved from ≡ to the filmstrip right-click menu, acting on the clicked image; Shift+F10 and Menu open it from the keyboard.
   Original note: Today the viewer's "…" menu and the filmstrip right-click menu overlap.
   - Recommendation: build both from one action model, so they cannot drift apart. "…" lists every action for the current image or selection; right-click offers the same actions for the clicked item. Shortcuts cover the frequent actions.
   - GNOME guidance is that context menus should not be the only way to reach an action. Removing "…" therefore needs another visible path, and that choice belongs to the user.
10. **Deferred (October 12): one source for dimensions.**
    - The index stores oriented sizes only for rows written since the October 12 checkpoint (`oriented_source_dimensions`); older rows, most of the live 4,544, may be unrotated.
    - So the header-read `DimensionCache`s are a correctness guard, not waste. Each path is read once per session, on a worker.
    - Unify only together with an additive migration that marks trusted rows (or re-indexes them), ideally with Phase 4's file identity.
    Original scope: the library index's oriented width and height. Remove the extra `DimensionCache`s and the repeated header reads.
11. ~~Move the remaining GTK-thread work off it.~~ Done October 12: badge colours, duplicate grouping, bucket open, virtual-view lookups, and the whole-library search cache. Sidebar counts were already on a worker. On October 13 (audit) the last bind-time tag query was removed, and bucket and search results fill the store with one `splice` instead of a signal per row; this replaces the planned chunked load.
   Original scope:
   - Fill the tag lookup during filmstrip bind off-thread.
   - Open buckets on a worker with the checked query and a chunked store load.
   - Replace the linear `entries_for_path` scans with the path index.
   - Count sidebar buckets off-thread.
   - Compute duplicate groups once per change, not on every navigation, where it is currently O(n²).
   - Cache the whole-library search candidates for each session.
12. **Extract the code this phase changes from `window.rs`** into `ui/window/` controllers: search, bucket actions, and file actions.

**Not in scope (D1, tags frozen):** multi-select tagging, tag prefix matching, and Tags-page counts.

**Acceptance:**

- Typing `mountains 4k png 16:9` anywhere opens search with three chips and searches for "mountains" in the current scope. One action broadens to the whole library.
- Twenty selected results reach a bucket in one action.
- After cleanup, every bucket's member count is unchanged, and no manual tag rows changed.
- With a release build on the real library, searching, filtering, and opening buckets cause no main-loop stall over 100 ms. The Phase 0 baseline regresses by no more than 10%.
- With a 50,000-entry synthetic fixture, search, filter, and sort finish on the worker in under 1 s, and navigation does no work proportional to library size.

## Phase 3: Visual-Search Evaluation

**Outcome:** A recorded go/no-go decision and model/runtime choice based on evidence. A go also triggers tag retirement (D1). No production integration happens in this phase.

**Relevance decision (October 13): go, with EmbeddingGemma 2 through native llama.cpp (D22).**
- The user judged all 30 queries blind (908 pooled results; judgments saved as `.tmp/search-judgments-2026-10-13.json`). Scored with `tools/benchmark-score.py`:

| Candidate | Queries passing | Subject | Style | Mood & colour | People | Relevant in top 20 | Relevant in top 5 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| EmbeddingGemma 2, llama.cpp native | **30/30** | 8/8 | 5/5 | 7/7 | 10/10 | **88.2%** (529/600) | **95.3%** (143/150) |
| SigLIP 2 base, PyTorch reference | 30/30 | 8/8 | 5/5 | 7/7 | 10/10 | 85.3% (512/600) | 90.7% (136/150) |

- Both pass every category. Gemma is ahead overall and is the user's choice; its native runtime is already proven (decision rule).
- Weakest Gemma queries: "woman wearing a bikini" (11/20), "watercolor" and "pool" (14/20). They still pass.
- The user's own rating: about 85 out of 100 for accuracy, "very good".
- The user reported tiring after about 25 queries, but the saved file marks all 30 as judged. The verdict holds either way: with five unjudged queries counted as fails, Gemma would still pass 25/30.
- Items 4 and 5 were completed on the same day; Phase 3 is done. Phase 5 starts after Phase 4, with the two packaging choices in item 5 settled by the user.

**Evidence so far:** recorded October 12 on a CPU, using 1,000 images sampled from `~/Pictures` and 30 predefined queries.

| Candidate | Runtime | Indexing | Warm p95 query | Expected image in top 20* |
| --- | --- | --- | --- | --- |
| EmbeddingGemma 2 | PyTorch reference, BF16 | 0.21 img/s | 51 ms | 72% |
| EmbeddingGemma 2 | llama.cpp `b11457`, native | 0.30 img/s | 32 ms | 69% |
| SigLIP 2 base patch16-224 | PyTorch reference, FP32 | 2.67 img/s | 112 ms | 76% |

\* "Any one expected image appears." This is weaker than the gate, and the results are unjudged.

- Gemma native and reference agree: median image-vector cosine 0.983, mean top-20 overlap 79.5%.
- Every candidate passes the latency requirement.
- For the real library (about 5,800 images), Gemma native needs about 5.4 hours for a full index, which is acceptable overnight. SigLIP 2 needs about 40 minutes.
- Speed does not decide the choice; relevance and packaging do.

**Scope:**

1. ~~Cover the real query mix (D15).~~ Checked October 13: the 30 queries already span all four categories. The judging sheet labels each query's category, and the judge can change it.
   Original scope: Check the 30 queries against the four categories: subjects, style, mood and colour, and people or adult content. Add queries where a category is missing, choosing expectations before seeing any results. Report pass rates per category, because a model that handles adult content poorly fails the user even if its overall rate passes. Never send filter words (formats, resolutions, aspects) to the model.
2. **Judge relevance with the agreed rubric.** Tooling is ready (October 13): `tools/benchmark-judge.py` builds a blind judging sheet (908 pooled results for native Gemma and SigLIP 2), and `tools/benchmark-score.py` applies the rubric overall and per category. **This step is the user's.** A query passes when the top 20 contain at least 10 relevant results, including at least one in the top 5. At least 80% of queries must pass (24 of 30), and unjudged queries fail. A contact sheet or the trial UI may be used for judging.
3. ~~Make runs reproducible.~~ Done October 13. Both scripts record provenance (hashes verified against the download manifest), preview caches are keyed by file size and mtime, and `tools/benchmark-parity.py` exists.
   - Gemma native vs reference: cosine median 0.983, 5th percentile 0.955, minimum 0.905.
   - Top-20 overlap: mean 79.5%, minimum 50%.
   Original scope: Every report records:
   - model and projector hashes
   - quantization
   - runtime and library versions
   - preprocessing
   - thread count

   Add a parity script that computes cosine agreement and top-20 overlap between native and reference vectors.
4. ~~Measure filmstrip impact while indexing.~~ Done October 13: **passes**, with a 5–7% slowdown against the 10% limit.
   - **Method:** a temporary release-build GTK test stepped through 119 images of a 200-image folder of synthetic 2560×1440 JPEGs. Each run was measured idle, and again while `llama-server` (b11457, EmbeddingGemma Q8_0) indexed in the background with 2 threads at `nice 19`. Each run was done twice.
   - Typical next-image time: 30.0 → 32.2 ms (+7%).
   - Slowest 5%: 32.9 → 34.8 ms (+6%).
   - Folder ready: 742 → 778 ms (+5%).
   - The longest main-loop step was 85–97 ms in both conditions, so indexing did not cause it.
   - Indexing ran at about 0.41 images/s with 2 threads, about 4 hours for 5,850 images.
5. ~~Check packaging.~~ Done October 13 (assessed; the Flatpak build itself is verified in Phase 5).
   - **Footprint:**
     - Model: 296 MB (`embeddinggemma-2-Q8_0.gguf`) plus the 530 MB projector, 826 MB total.
     - Runtime: 44 MB as prebuilt (`llama-server` and its libraries, 20 MB of which are 14 CPU-specific variants).
     - Server memory while indexing: 1.3–1.4 GB RSS. Warm query p95: 32 ms.
   - **Flatpak:** llama.cpp is a CMake project with no required dependencies beyond the C/C++ runtime, so it can be a standard `cmake-ninja` module in the manifest, built CPU-only. Build it without network features (`LLAMA_CURL`/OpenSSL off; the prebuilt links OpenSSL only for model downloads), which keeps the offline invariant. Not native-only.
   - **Decided October 13 (the user deferred to Claude):**
     - **Model delivery:** the model files are installed once into a folder Sharpr checks, not bundled (D23).
     - **Runtime:** `llama-server` runs as a managed child process on loopback, started on demand and stopped when idle (D24).
   - SigLIP 2 was not evaluated further: Gemma won relevance.

**Decision rule:**
- Choose the candidate that passes relevance, in every category, with the simplest runtime that can ship.
- When results are comparable, prefer EmbeddingGemma 2, because its native llama.cpp path is already proven.
- Try another candidate only if every current candidate fails, and say which category failed.
- If none passes, stop: no integration. Name search and typed filters remain, and tags stay frozen, not retired.

**Optional experiment, approved by the user:** an in-app trial over the 1,000-image sample index through a local llama.cpp child process. The unfinished wiring is preserved on `visual-search-trial-wip`. It is off by default, not in the Flatpak, and kept in separate commits. It supports judging; it is not integration.

**Acceptance:** The decision, with its numbers per category, is recorded in this file, and the reports can be reproduced from their recorded provenance.

## Phase 4: File Identity And Buckets

**Outcome:** Bucket membership survives moves and renames made outside Sharpr (D3), keeps inheritance (D4), and stops depending on tags. Every image gets a content fingerprint, which Phase 5 and Phase 7 reuse.

**Status: accepted by the user on October 15, 2026** (build `679bbe1`; deployed and in daily use since `bf12e07`). The user's check: a bucket image moved outside Sharpr first gave an "image not found" notice; after opening the folder it was moved to, it reappeared in its bucket. "Could be improved but works" (Backlog).

Earlier status (October 14): not yet formally accepted. All acceptance criteria are met in tests. On the user's first launch, the migration ran on the live data: 151 members across the original 6 buckets, matching the copy test exactly. The user has since created 3 more buckets.
- **Unit 1, fingerprints:** a background pass at nice 19 and idle I/O; progress shows in Preferences.
- **Unit 2, membership:** the `bucket_members` table and the migration. Every reader and editor has switched over.
- **Unit 3, re-linking:** after each pass, members follow moved and edited files, and missing ones are counted.
- **Acceptance on a copy of the live databases:** all 6 buckets were identical before and after, image for image, including Art › Diffusion. The counts: Model 95, CSR 44, upgrade? 4, Art 3, Trash? 3, Diffusion 2, none missing. All 7,718 tag rows were unchanged, the backup was written first, and the index was stamped version 2.
- **Moved files:** a file moved and renamed outside Sharpr stays in its bucket after the next pass (unit and GTK tests). A deleted file counts as missing and is kept. Removing an image from Diffusion leaves it in Art when Art holds it directly.
- **Rollback note:** older builds refuse the version-2 index. Going back means restoring the `before-bucket-membership-*` backup.

**Scope:**

1. **Content fingerprint.**
   - A fast hash of each image's bytes, computed in the background at the lowest priority.
   - Cached by path, size, and mtime.
   - Disabled folders are excluded, and the progress is visible.
2. **Per-bucket membership.**
   - A membership table keyed by fingerprint that records each image's last known path.
   - Inheritance is computed at display time: a parent shows its own members plus every member of its descendants.
   - Removing an image from a child bucket never affects the parent's own membership.
   - Bucket edits stop writing tags, which removes the two-database crash window for buckets.
3. **Re-link.**
   - A library scan reattaches moved or renamed members by fingerprint.
   - Members that can't be found show as "missing" with a count, and are never dropped.
   - Identical copies of a file are all members.
4. **Non-destructive migration.** It requires Phase 1's schema versioning.
   - Back up first.
   - Build each bucket's membership from its current effective-tag membership.
   - Verify that every bucket has an identical member count.
   - Leave all tag rows untouched, so the migration is reversible.

**Acceptance:**

- On a copy of the live databases, all 6 buckets keep identical member counts after migration, including Art › Diffusion.
- Tag row counts and contents are unchanged.
- A file moved and renamed in Nautilus is still in its bucket after a rescan.
- A deleted file shows as missing; it is not silently removed.
- Removing an image from Diffusion leaves it in Art when it is also a direct member of Art.
- `./check.sh` passes.

## Phase 5: Visual Search And Tag Retirement

**Status: accepted by the user on October 15, 2026** (build `679bbe1`). The user accepts the manual browsing-during-indexing checks in place of a measured comparison, and file-name matches stay first alongside visual results (may be revisited).

Earlier status (October 14): every scope item built and committed; the core deployed in `c176934`. Phase 3 said go (D22).
- **Deployed in `c176934`** with the model installed: visual search, GPU indexing, and tag retirement.
- **Deployed in `baa007d`** (October 14): the audit fixes, the Library Overview, search into buckets, and negative search.
- **User test of `ef91d32` (October 15):** `tower -water` and the strict `--water` work, and so does Add to Bucket from the search line. Two faults, fixed in the round in `CURRENT_TASKS.md` (D25):
  - The bucket picker doesn't close on a click outside it.
  - Ctrl+A does nothing after clicking a thumbnail. Likely cause, still to confirm: the filmstrip list handles Ctrl+A itself before the window's shortcut. Its single-selection model can't select all, but the key is still used up.
- **The user's hands-on verdict on `c176934`** (October 14): Sharpr "seems to run even better than it did before this recent work started, regardless of indexing work".
- **Units:** indexing and search (`58ac0ef`), tag retirement (`78a0639`), packaging (`b07a226`), and GPU indexing (`c176934`).
- **Verified in tests:**
  - A GTK test with the real model indexes synthetic images in the background and ranks them by content, file-name matches first.
  - Without a model, search falls back to file names and says so.
  - Turning a tag into a bucket keeps every tag row unchanged.
- **Parity:** the app's own pipeline over the 1,000-image sample (release build, CPU) matched the judged benchmark vectors at median cosine 0.9995 (5th percentile 0.997, minimum 0.968), with 95% top-20 overlap (minimum 85%).
  - It passes 30/30 queries in every category. 85.2% of its top 20 and 94.7% of its top 5 are judged relevant, against 88.2% and 95.3%; these are lower bounds, because the about 5% of results outside the judged pool count as not relevant.
  - Indexing ran at 0.31 images/s with 2 threads, about 5.2 hours for the real library. Queries took 15 ms at p95. The 2 errors were files deleted since the sample was drawn.
- **GPU experiment (October 14, approved by the user):** the official Vulkan build of the same llama.cpp (`b11457`), pinned to the RTX 4060 Laptop GPU, ran the same 1,000 images at **5.24 images/s, 17× the CPU**. That is about 20 minutes for the real library instead of 5 hours, using about 1.2 GB of graphics memory.
  - Its vectors match the CPU's at median cosine 0.99989 (minimum 0.9985), with 98% top-20 overlap, so both can share one index.
  - It also passes 30/30, with 85.3% relevant in the top 20 and 95.3% in the top 5.
  - Integrated the same day, at the user's request: GPU indexing is the default when a GPU build is installed, with the processor as fallback. Phase 3 should have measured the GPU from the start.
- **Match strength** is calibrated on the judged sample: the fifth-best score after subtracting each image's closeness to generic prompts. It separates 28 of 30 real queries from 13 of 15 absent subjects. It isn't judged below 50 indexed images.
- **Decision for the user:** whether file names should still match, first, alongside visual results (implemented that way for now, so names like `seedvr2-…` stay findable).
- **Left for acceptance:**
  - Deploy the undeployed work and have the user check it.
  - Browsing during indexing: the user reports it smooth, and the app better than before (manual checks). A measured comparison against the 10% limit is still open, unless the user accepts the manual checks.
  - The real library is fully indexed: 5,178 vectors, no failures.

**Outcome:** Words typed in the search box find images by content. Tags disappear from the interface without losing any data.

**Scope:**

- **Search behavior.**
  - Apply scope, disabled folders, and chip filters before ranking.
  - Show the closest 200 results with a "load more" option.
  - Keep results stable until the query, scope, or filters change.
  - Show coverage as indexed versus eligible, and match strength, so "not indexed" and "no good match" are clearly different (principle 5).
  - Decide with the user, during this phase, whether filename matching remains as a secondary mode.
- **Vector storage.**
  - Vectors go in their own SQLite file, which is safe to delete.
  - Records are keyed by the Phase 4 fingerprint and versioned by model and preprocessing.
  - Incompatible or stale vectors are rejected.
- **Indexing.**
  - Index the whole enabled library in the background at the lowest priority.
  - It is resumable, stops cleanly on quit, and reports errors.
  - Overnight indexing is acceptable.
- **Model runtime.**
  - The app manages a local child process and talks to it only over authenticated loopback.
  - The user supplies the model files at a documented path; the app downloads nothing.
  - Native builds are supported. The Flatpak is supported only if Phase 3 proved it feasible.
- **Library Overview (added October 14 at the user's request; built October 14, committed and verified, not deployed).** Show the user how big the library is and what the indexes hold. Its GTK test checks the counts. The GTK suite's intermittent screenshot timeout predates it: the same commit without the overview failed 1 run in 3.
  - A window from Main menu → Library → Library Overview…, built from standard Libadwaita rows, updating live while open. Its groups:
    - **Library:** total images and size, a breakdown by format, the count below 4K, and disabled folders with how many images they hide.
    - **Fingerprints:** images fingerprinted, and exact-copy groups.
    - **Visual search:** indexed, waiting, and couldn't be read; the device in use, speed, and time left; the model, the vector file's size, and when the last pass finished.
    - **Buckets:** buckets, images, and members that can't be found.
  - Actions: "Rebuild Index…" (confirm, delete the vector file, reindex) and "Open Model Folder". Not included (the user's choice): opening failed or unindexed images in the filmstrip.
  - While indexing runs, the existing background-activity indicator in the header shows a progress ring; its tooltip gives the count, the device, and the time left.
  - Preferences keeps only settings. The visual-search status line moves into the overview.
  - All counting runs off the GTK thread.
- **Search into buckets (added October 14 at the user's request; built October 14, committed and verified, not deployed).** Before this, turning results into a bucket meant Ctrl-clicking each image (no select-all, no range).
  - "Strong matches" are the file-name matches plus visual results whose generic-corrected score reaches `STRONG_RESULT` (0.04, the Strong strength threshold). Only shown results count.
  - **Search into bucket (the user's idea):** one action in the search status line, "Add to Bucket…", turns the current results into a new bucket or adds them to an existing one.
    - Because results run from strong to weak (the closest 200), the dialog says how many images will be added and offers the strong matches by default. It can also use everything shown, or only the selection when there is one.
    - The bucket is a snapshot. It never updates itself; saved or smart searches stay out (D5).
    - Several searches can fill one bucket, such as `tower`, then `plane`. A single query `tower and plane` instead finds images showing both.
  - **Selection tools:** Shift-click selects a range (pick down to the last good result), Ctrl+A selects all, and Ctrl-click drops images. The status line shows "N selected".
  - All membership writes run off the GTK thread, through `buckets::add_to_bucket`.
- **Audit fixes (done October 14, committed and verified, not deployed; audit `docs/archive/audit-2026-10-14-phase5.md`).** 17 findings, with 0 critical. Claude reviewed them and agrees with all 17.
  - Fixed in `df82623` (offline), `aab73e0` (vector integrity), `ea7aa90` (failures), `b15b151` (tag conversion and scope), `6dddb45` (search stability and status line), and `cf438a9` (query deadlines, JPEG XL budget, unsupported formats).
  - Every fix has a regression test. `./check.sh` passes 190 tests; with the real model, the indexing service and all four GTK tests pass.
  - The planned order, kept for reference:
  1. **Offline guarantee (finding 3, high).** Pass `--offline` to every managed `llama-server`, and strip inherited `LLAMA_ARG_*` and other remote-model environment variables. Today an inherited `LLAMA_ARG_MODEL_URL` could make the child download, and even replace the installed model. Also correct the Flatpak docs: they disable HTTPS and UI fetches, not every network capability.
  2. **Vector integrity:**
     - **Finding 1, high.** Keep each file's stamp in the indexing queue. Recheck it before decoding and before saving a vector or failure. A file changed mid-pass waits for a fresh fingerprint and is never saved under the old one.
     - **Finding 2, high.** Search revalidates the model key from file stamps on every query. A replaced model drops the cached server, query vectors, and image vectors.
     - **Finding 12.** Detect a deleted or replaced vector file, invalidate the cache, and request indexing. The Library Overview's "Rebuild Index…" uses the same path.
  3. **Failure classification (finding 9).** Only a real image problem is recorded as a permanent failure. Server, transport, or GPU errors on a live server retry once on the processor; otherwise they stop the pass without marking the image.
  4. **Tag conversion (findings 4–6):**
     - Create the bucket and its members in one transaction.
     - Offer and conversion share one eligibility rule, which excludes automatic rows and disabled folders.
     - Disabling or enabling a folder cancels and requeues fingerprint and indexing work.
  5. **Search stability (findings 7–8).** "Show More" extends one ranked snapshot per query, scope, and filters, keeping the selection and scroll. The fingerprint cache refreshes when the search inputs change.
  6. **Responsiveness (finding 10).** Query preparation gets a deadline and honours cancellation, so a stalled server can't hold up newer searches or filters. A stalled child is killed and restarted.
  7. **Memory (finding 11).** Check a JPEG XL's dimensions against the decode budget before a full decode.
  8. **Small UI fixes (findings 14–16):**
     - Coverage counts every candidate, including file-name matches.
     - Escape and folder or bucket changes clear the search status line.
     - Query errors show their real reason.
  9. **Done on October 14 (finding 17):** `CURRENT_TASKS.md` was rewritten to match the committed and deployed state.

  **Not now (finding 13):** AVIF and HEIC can't be indexed. The user's library has none of these files (3,752 JPEG, 1,457 PNG, 462 JPEG XL, 186 WebP), so it moves to the Backlog. Until it is fixed, an unsupported format must be reported as unsupported, not as unreadable.

  **Sol's open questions, answered:**
  - Files changing mid-pass are skipped and retried, not snapshotted.
  - "Show More" keeps its first ranking until the inputs change. The Phase 5 scope already requires stable results.
  - Disabled folders are excluded from conversion; AGENTS.md already rules this.
  - The GTK screenshot timeout and the rapid-navigation flake are Claude's to investigate, in the test harness.
  - Browsing impact is still to be measured.
- **Negative search (added October 14 at the user's request; built October 14, committed and verified, not deployed).** `tower -water` finds towers, leaving out images that look like water.
  - **Syntax:** a word starting with `-` becomes a chip reading "not water". Clicking the chip turns it back into text, like the other chips (D16).
  - **The same negation works for every kind of word:**
    - `-png` and `-16:9` leave out a format or an aspect ratio, which gives the "not 16:9" the user asked for (see Phase 9).
    - A negative word also removes file-name matches whose names contain it, such as `-seedvr2`.
  - **Method: remove the images most like the negative word.** Never send it to the model as part of the query.
    - For each negative word, score every indexed image's closeness to it, corrected by the generic prompts (as match strength is).
    - Drop the images in the word's top 10% across the library; the rest keep their normal order.
    - Tune the cut-off on real searches.
  - **Evidence (October 14, judged 1,000-image sample, six cases).** Each pair below is: images judged to show the unwanted thing in the top 20, then images judged relevant.

    | Query | Plain | Removing the top 10% | Sending "… without …" |
    | --- | --- | --- | --- |
    | woman −beach | 1 / 17 | 0 / 15 | 11 / 5 |
    | woman −bikini | 1 / 17 | 0 / 17 | not given |
    | robot −gun | 8 / 17 | 1 / 9 | not given |
    | blue sky −mountains | 5 / 18 | 1 / 15 | not given |

    - **Tuning check (October 14, after the user found towers by water in `tower -water`).** Eight judged cases, totals of unwanted / relevant in the top 20:

      | Rule | Unwanted | Relevant |
      | --- | --- | --- |
      | No minus word | 22 | 132 |
      | Library's top 10% (current) | 4 | 111 |
      | Library's top 20% | 2 | 87 |
      | Library's top 30% | 2 | 69 |
      | Top 30% of the word's own top 100 results | 4 | 107 |
      | "pos neg" scores higher than "pos" (contrastive) | 16 | 129 |

      Stronger cut-offs cost far more relevant images than they remove. The image vectors mostly describe the main subject, so water that is only part of a tower picture scores low. The 10% rule stays the default; at the user's request, `--water` ("Strictly not water") leaves out 30% for when the user accepts that cost.
    - Sending "woman without beach" backfires: the model looks for beaches.
    - Subtracting the negative word's vector sat in between (woman −beach 0 / 14, robot −gun 5 / 13).
    - Both columns are lower bounds, because images outside the judged pool count as neither.
- **Retire tags (D1, D2).**
  - Remove the tag popover and editor, tag pills, and Ctrl+T. The Tags page and the page switcher were already removed on 2026-10-13 (D20).
  - Offer each manual tag that no bucket uses (`nsfw`, `nude`, `mountain`, `nature`, `comics`, …) as a new bucket.
  - Tag rows stay in the database, unused.

**Acceptance:**

- Phase 3 relevance and latency hold inside the app.
- Browsing slows by no more than 10% during indexing.
- A missing or mismatched model shows a clear message, and name search and filters keep working.
- Deleting the vector file loses nothing else.
- After retirement, bucket counts are unchanged, offered conversions create correct buckets, and tag row counts are unchanged.
- The Library Overview's counts match the index files, update while it is open, and never block the GTK thread. The header ring appears only while indexing runs.
- From one search, a new bucket with the chosen results takes at most three actions. A second search adds to the same bucket, and no image is added twice.
- `tower -water` returns towers with the most water-like images removed. (User test, October 14: a minus word typed last showed no chip after Enter, so it looked ignored, though the search applied it. Enter now finishes the last word too. Whether the 10% cut-off is strong enough beside other words is for the user to judge on real searches.) `-png` and `-16:9` exclude those formats and ratios. Negative chips can be turned back into text.

**Later (optional):** Find similar, per-folder indexing, scheduling controls, and incremental updates when files change.

## Phase 6: Wallpaper Output

**Status: accepted by the user on October 15, 2026** (build `679bbe1`): "works so far", with more testing to come in normal use.

Earlier status (October 14): built, committed, and verified. Batch WebP export and the retirement of Resize Copy and the non-WebP outputs are in `5e2ad3d`; cropping in the viewer, the preview fallback, and the rotated-JPEG test follow in the next commit. Left for acceptance: the user makes a real wallpaper end to end.
- **User test of `ef91d32` (October 15):** batch WebP export (lossless, quality, fit within) and Crop Copy at full size work, and the copy lands in Wallpapers or the chosen folder. One fault: the export window doesn't close on a click outside it (D25).

**Outcome:** The user crops in Sharpr and batch-exports WebP well enough to stop using XL Converter (D13).

**Existing implementation:** committed in the Phase 0 checkpoint.
- Crop Copy: a movable 16:9 selection, or free aspect
- WebP Copy: quality 90 by default, original size or resized
- intermediate PNG copies
- exclusive output reservation, partial-name retry, and checked flushes, with original-byte tests

**Scope:**

1. **Batch WebP export** from any selection:
   - lossless by default, with lossy quality as an option
   - an optional fit within W×H that only downscales (preset 3840×2160)
   - a chosen and remembered destination
   - unique names
   - a background job with progress and cancel, and an error summary
   - no queue or history
2. **Retire Resize Copy** and JPEG, PNG, and JPEG XL output from the UI. Viewing JPEG XL stays.
3. **Crop preview fallback.** When glycin cannot preview a format, use a preview decoded and oriented on a worker. Add a test with an EXIF-rotated source.
3a. **Crop at full size (user, October 14).** The crop preview is far too small to judge a wallpaper. Crop in the viewer itself at full size, or fullscreen, instead of in a small dialog.
4. **Make the Wallpapers folder easy to reach** as a crop and export destination.

**Acceptance:**

- Exporting 100 selected images to lossless WebP, fitted within 3840×2160, shows progress and keeps browsing responsive.
- Cancelling leaves no partial files.
- Originals stay byte-identical, and a name collision never overwrites a file.
- Cropping EXIF-rotated JPEG and PNG sources produces correct pixels.
- The user completes a real wallpaper end to end.

## Phase 7: Cleanup

**Status: accepted by the user on October 15, 2026** (build `679bbe1`): "improved but not perfect"; on the real library it found three copies the user had not found before, and they were removed. Further tuning goes to the Backlog.

Earlier status (October 14): built, committed, and verified.
- **Item 1:** the old finder's inconsistency was incomplete coverage: it only knew the perceptual hashes of images whose thumbnails had loaded this session. "Find Copies…" replaces it.
- **Fixture with the real model:** the 1080p JPEG (0.9959) and the WebP conversion (0.9970) of a 4K picture group with it and its exact copy. A same-shape 16:9 crop (0.9711) and a mirrored copy (0.9892) fail the pixel check and stay out.
- **Every vector pair also needs the same aspect ratio,** because a test showed a crop could otherwise chain in through a close copy.
- **User test, October 14 (`baa007d`): the window never got past its spinner.** Cause: the real library has 5,383 pairs between 0.95 and 0.99 (involving 1,401 pictures), and each pixel check decoded both full-size files, one pair at a time, with no reuse and no way to cancel: up to about 10,000 full decodes. Fixed: each picture's 32×32 version is made once, in parallel, from the thumbnail cache where possible. Closing the window cancels, and the window counts progress.
- **User test, October 14 (`5044c72`):** it scanned the whole library and ticked the extras. Now it looks among the images in view (the open folder, bucket, or search results), ticks nothing, and offers Select All Extras and Select None.
- **Every vector match now gets the pixel check,** not only those below 0.99: the library has 1,031 pairs above 0.99, too many to trust the score alone with files headed for the trash. The real-model fixture still groups the 1080p and WebP copies and leaves out the crop and the mirrored copy.
- **User test of `ef91d32` (October 15):** it checks only the open folder, shows progress, stops when closed, ticks nothing, and Select All Extras and Select None work. The groups the user saw were real copies, and Compare from a group returns with the dismissed file ticked. The "no dialog" report was the filmstrip's Delete and right-click trash, which have never asked; Find Copies still confirms. D25 gives the filmstrip an Undo instead.
- **Left for acceptance:** the user reviews groups on the real library, and the thresholds are tuned on those real examples.

**Outcome:** The user reliably finds exact copies and resized or converted copies, then trashes the extras safely (D14). Images below 4K are found with the Phase 2 filter.

**Scope:**

1. **Replace the current, inconsistent perceptual-hash finder** if the vector method in item 3 passes this phase's acceptance. Only if it doesn't, find the cause of the inconsistency (suspects: incomplete hash coverage, the threshold, orientation) and fix that instead.
2. **Exact copies:** group by the Phase 4 fingerprint.
3. **Resized and converted copies: use the Phase 5 visual-search vectors.** They already exist for the whole enabled library, and comparing every pair is a few seconds on a worker.
   - **Evidence (October 14, the judged 1,000-image sample, from file metadata only):**
     - 60 pairs scored a cosine of at least 0.99. 41 of them were identical files, which fingerprints already catch. The other 19 were copies the fingerprint misses: 15 converted to another format, 3 resized, and 1 re-encoded.
     - There is a clear gap below that: 13 pairs between 0.95 and 0.99, then 372 pairs at 0.90 (merely similar pictures).
   - **Rule:**
     - At 0.99 or higher, group as copies.
     - Between 0.95 and 0.99, group only if the aspect ratio matches (which excludes crops) and a pixel check at thumbnail size agrees.
     - Tune both thresholds on the user's real examples. Crops and near-identical variants are not grouped (D14).
4. **Review groups.** Each group shows resolution, format, and size, and suggests keeping the highest resolution. Trash only after confirmation, and report trash failures.

**Acceptance:**

- In a fixture, exact copies, a 1080p JPEG with its 4K PNG version, and a WebP conversion are grouped; a crop is not.
- Nothing is trashed without confirmation.
- Grouping runs off the GTK thread.

## Phase 9: Interface Review

**Status: accepted by the user on October 15, 2026** (builds `679bbe1` and `2b5f070`) and merged into `main`. The user's pass: Compare (including the candidate row), Survey from the right-click menu, add to last bucket, the viewer right-click menu and hover-expanded Info work; the rest is as expected. Fixed during the pass: the readout's "Image info" tooltip covered the expanding card and was removed, and the fast-scroll column shrank from 46 to 14 px, which also aligned the Sort button with the cards. Ctrl+Shift+S opened the GTK Inspector instead of Survey on the user's desktop ("not a biggy"; Backlog).

**Checkpoint (October 15): implementation and verification complete; paused at the user's request before installation.** Fast scrolling, aspect comparisons, last bucket, hover-expanded Info, Survey, one library root and the final style/icon/Help/shared-menu work are implemented. `CURRENT_TASKS.md` records commits, checks and the verified live-data backup; `docs/reference/interface-inventory.md` records control dispositions. Latest checks: 217 regular tests, 18 GTK workflows and focused high-contrast checks at minimum/wide sizes. The installed build is still `ef91d32`. Final installation and the user's manual acceptance remain pending, so this phase stays open.


**Status (October 15): steps 0–2b done; the look is committed (`cb57749`..`52595e5`), not deployed.**
- **Step 0, the map:** 83 controls marked by the user. **Step 2a, research:** 18 ideas, 15 wanted. **Step 2b, the look:** the hybrid (D26), then the Sharpr design language (D27), built in two prototype passes and tried hands-on by the user on a copy of the library ("all in all i like it").
- **Built so far:** I6, I7, I8 (Preferences and Shortcuts), I12, I14, I15, I17 (in part: the library switcher and Open Folder are hidden), the combined Sort and Filter with chips (option A), the readout face, restyled chips and pill, and key hints.
- **Finishing touches** (user's trial, October 15, plus code):
  - **Accepted: Preferences and Keyboard Shortcuts as styled sheets (I8).** They now use `AdwPreferencesDialog` and `AdwShortcutsDialog` in the main window, inheriting the approved monospace face and compact styling. The old separate windows missed this styling. Nested library editors return to Preferences; the settings controls and existing menu actions are retained, with Ctrl+? for shortcuts. The unused click-outside helper is removed. Verified by `cargo build`, `./check.sh` (211 regular tests), all 12 GTK tests, and light/dark screenshots at 1024×600 and 1600×900; accepted in the October 15 hands-on UI checks, not deployed.
  - **Accepted: bundled JetBrains Mono (D27).** Official v2.304 upright and italic variable fonts (weights 100–800), with OFL licence and source checksums, are embedded in GResource. Pango loads them for this process only through anonymous files; native and Flatpak binaries carry the same assets, with no font installation or downloads. CSS prefers the bundled family, and About includes its licence. Verified by build, `./check.sh`, and all 13 GTK tests, including actual font loading with system fonts excluded; screenshot layouts reviewed. Accepted in the October 15 UI trial. Not deployed.
  - **Accepted (D28): contextual Compare/crop controls.** Compare's viewing modes, Linked zoom, Fit/100%, Undo and Done use the existing top bar; per-side pickers and Keep Left/Keep Right buttons sit below the canvas. Arrows keep that side and dismiss the other, with Undo and no file changes. A simple remaining count replaces “N of N left”. Crop's top bar holds aspect, live dimensions, destination, Cancel and Save Copy. Browsing search/chips return intact; late search/sidebar events cannot replace an active mode. Side panels stay hidden at wide sizes and restore with the current adaptive layout. Candidate thumbnails are a separate proposal. Verified by build, `./check.sh` (211 tests), all 14 GTK tests, original-byte/cropped-copy checks, and light/dark screenshots at 1024×600 and 1600×900. Committed as `c5699d7`. The user accepted the October 15 hands-on checks: choices/Undo, viewing modes, linked zoom/pan, survivors and restored search/chips. Not deployed.
  - Size filters from the old Filter popover (minimum width and height) are gone; size chips such as `4k` replace them.
  - **Accepted: WebP export as a compact in-window sheet.** Recreating the popup after a native folder choice and a Continue Export workaround both failed the real desktop trial, despite in-process chooser tests passing. Live inspection found a visible but unmapped Wayland popup and an idle GTK loop. The settings form now uses the established `.sharpr-sheet` style on the main window's native surface and stays mapped throughout the folder chooser. No popup recreation or recovery toast. The selected folder and edited options survive Select/Cancel; D25's click-away cancellation is retained with the earlier scoped capture handler. Verified by full checks (211 tests), all 14 GTK tests and build: same sheet/surface, both chooser responses, actual two-image export, fitted lossless pixels, unchanged originals, and light/dark layouts at 1024×600 and 1600×900. The user tested and accepted `fde5a52` on October 15. Not deployed.
  - **Accepted: retired the old independent Filter path (`7399cee`).** Removed the detached popover, controls, callback/reset wiring and unused `DimensionFilters`/`AspectFilter` state. The combined menu and typing share chip matching; dimension caching and aspect sorting remain on the browse worker. GTK coverage activates the real menu row and visible removal button, compares typed/menu results and restores folder selection. The former legacy-filter tests now check the chip engine's dimension/orientation and 16:9 tolerance rules. Verified by build, `./check.sh` (211 tests), all 14 GTK tests, light/dark menu screenshots at both sizes, before/after 5,000-image scroll/switch checks, and a native synthetic-library filter/scroll/folder-switch check. The user reports filtering works. Acceptance follow-up: fixed the inherited disabled Ascending/Descending targets (strings instead of booleans) and active-view thumbnail loss during virtual reloads after global-cache eviction. Both regressions failed before the fixes; latest verification passed build, full checks (212 tests), 22 targeted model/query tests, 3 relevant GTK tests and native menu/scroll/folder-switch checks. Debug logs are retained locally without a release performance claim. The user retested the sort and thumbnail fixes and accepted the unit. Not deployed.
  - Hands-on trials pass `~/.local/share/icons` through: isolating the data folder hid the user's "dreams" icon theme, which is why icons drew broken and the menu showed ≡ instead of "…".
- **Next:** resume installation and the final manual acceptance pass when the user requests it. D30 authorizes I1, I5, I10 and I13; their implementation is complete.

**Implemented and verified (D29): Compare candidates and shortcut (I3/I4).** Replace dropdowns with a compact horizontal thumbnail row in initial filmstrip order, with L/R badges and explicit Replace Left/Right target defaulting to Right. Choosing preserves candidates; choosing the opposite image swaps sides. Keep removes the loser and Undo restores its position. Keep per-side filenames/details/buttons and a quiet remaining count. Add guarded Ctrl+Shift+C and context-menu/Shortcuts hints. Use virtualized GTK ListView and existing thumbnail decode/cache code off-thread; preserve the filmstrip scheduler. Verified by build, full checks (212 tests), all 14 GTK tests, choice/swap/Keep/Undo/survivors and original-byte checks, keyboard guards, cold-cache loading, 500-candidate virtualization/session checks, light/dark screenshots at minimum/wide sizes, and native 5,000-image scroll/folder-switch checks. GTK prepares extra rows, so only viewport-intersecting rows request thumbnails. Debug before/after logs remain local without a release performance claim. User acceptance is pending; not deployed. Previous cleanup plan: `4a97bce`.

**Evidence and user choices, retained from the handoff:**

- **Step 0, the map: published October 15** at https://claude.ai/artifact/1uJDcNEQbrAcTRZ6GXp6jQ (private). It has 83 controls in 10 areas, 30 screenshots, two diagrams, 9 counted tasks, and 18 observations (N1–N18). The user's marks and notes save in the page's database (`marks/<control id>`, `notes/general`); read them back with ArtifactData before the review.
- **Screenshots** come from the `interface_tour_screenshots` GTK test (`8d6111b`), which writes `tour/` beside the fixtures. Rerun it after interface changes for before-and-after pictures.
- **The user's marks (October 15), 82 of 83:**
  - **Daily:** Filter, Sort, the folder tree, buckets, type to search, chips, minus words, right-click Add to Collection and Compare, scroll zoom and pan, the Compare split divider.
  - **Never:** the Library and Photos titles, Open Folder, the library switcher, the indexing ring, every Show Overlay switch, Fit and 1:1 in ≡, Presentation (F11), right-click Info, Rename, Whole library, Show More, the Manual, Shortcuts, About, the two tag tools, and Preferences' libraries, collections, and cache.
  - **Confusing:** Compare's `[` `]` dismiss keys ("bad shortcut key, not easy to find on keyboard") and its "4 of 4 left" with Undo.
  - **Notes:** "dialogs and right click menus are chunky"; the overlay "would be better to be contextual or appear on hover"; zoom by scroll wheel alone "works well"; double-click for full screen; the window is usually maximized, and closing it quits.
  - **How the filmstrip is used:** to scroll fast through many images with the scroll wheel or by dragging the small scrollbar handle, finding and reviewing by eye. Arrow keys would be far too slow; they suit left-and-right stepping where that applies.
  - **Icons (N16):** the sidebar toggle, the menu button, and the Appearance tab render in the real app, so the broken icons are a test-window artifact. The real menu button shows "…", not ≡ (the test window uses a different icon theme).
- **Step 2a, research: published October 15** at https://claude.ai/artifact/1eX3MVj6LwfafrLeTYyaKd (private). It covers Lightroom, darktable, FastRawViewer, Eagle, Photo Mechanic, Google Photos and Immich, Loupe, Strata and Zed, with sources. It has 18 ideas (I1–I18), with Want / Maybe / No reactions saved in `reactions/<idea id>`. Its reading of the evidence: the hybrid look.
- **The user's reactions (October 15), all 18:**
  - **Want:** I1 fast-scroll handle; I2 arrow keys pick the Compare winner; I3 a thumbnail row instead of "4 of 4 left"; I4 a key for Compare; I5 Survey view; I6 compact menus; I7 pickers as popups; I8 one dialog style; I10 add to the last bucket; I12 the chip on pointer movement; I13 hover grows the chip into Info; I14 a neutral dark canvas; I15 one top row; I17 remove unused controls; I18 one Help item and a slimmer main menu.
  - **Maybe:** I9 a type-to-filter bucket picker; I16 a sort button that names its sort.
  - **No:** I11 number keys for buckets.
- **Look comparisons:** https://claude.ai/artifact/NKxHfeUMzB6YCrVo2EZvWs (private). D26–D27 record the chosen hybrid, monospace everywhere, aligned columns, and filter chips. The trial data lives in `.tmp/look-trial/`.
- **Behavior ideas approved and implemented under D30:** I1 fast-scroll handle; I5 Survey culling grid; I10 add to the last bucket; I13 expand the readout into Info on hover.

**Added October 14 at the user's request.** Placed near the end, "but no less important".

**Outcome:** Every control has one clear home, a name or recognisable icon, and a stable position. Nothing is duplicated without reason. Controls that matter only in a context appear in that context. Each change is a decision the user approved.

**The user's direction (October 15):**
- **The core is the filmstrip plus the big viewer.** The user calls it their one good design idea; every other control is judged by whether it serves or crowds it.
- **No reference app exists**, which is why Sharpr was made. The user likes Strata and the Zed editor, and finds Libadwaita and GNOME Files limiting, with "strange conventions". Pure Rust on GNOME stays. Strata shows a pure Rust GTK4 app can have its own look without Libadwaita (Zed uses its own toolkit, so it is a reference for the look only).
- **Plan approved by the user:** map, then the user's marks, then research, then the look decision, then proposals, then build.

**Method:**
0. **Map and marks.** A private page holds an overview diagram of the window and how its parts connect, one diagram per area, screenshots, and a table of every control. It also walks the user's real tasks (find, bucket, crop and export, clean up copies, compare, cull) and counts the steps each takes. The user marks each control "daily", "sometimes", "never", or "confusing", and notes GNOME or Libadwaita conventions that bother them. The inventory in step 1 feeds it.
1. **Inventory.** GTK tests screenshot every surface and state: folder, bucket, search, chips, Compare, fullscreen, the empty library, dialogs, and menus. They do this at 1024×600 and at a wide size, in light and dark. Every control is listed with its location, label or icon, shortcut, and any duplicate.
2. **Review** each control against these questions:
   - Is it duplicated or redundant?
   - Is it unlabelled, or does it move?
   - Should it always be visible, or appear only in context (principle 6, quiet interface)?
   - Can the keyboard reach it, with an accessible name?
   - Does it follow the GNOME HIG?
2a. **Research,** aimed at the problems the marks find. Look: Strata, Zed, and one or two apps known for clean interfaces. Workflow: Lightroom's grid and compare views, Photo Mechanic's culling, and Eagle's collections. What to borrow, not what to copy.
2b. **The look decision comes early** (the Libadwaita question below), because the proposals are mocked up in the chosen style. Options: keep Libadwaita and restyle; a hybrid that keeps its invisible parts (system light, dark, and accent, toasts, Preferences) and drops the conventions the user dislikes (separate header bars, alert dialogs for everything, the ≡ button); or plain GTK4 like Strata.
3. **Proposals** go on a page with before-and-after screenshots, one yes/no decision each. Approved changes are recorded in the decision log, as D18 and D21 were.
4. **Build** in small commits, with screenshot tests that hold the agreed layout.

**Starting list (the user's observations, October 14, plus code findings):**
- **Aspect ratio exists in three places:**
  1. The filmstrip "Filter" popover: all, landscape, portrait, square, and 16:9 within 2%, plus a minimum size. It keeps its own filter state.
  2. The typed chips: `landscape`, `portrait`, `square`/`1:1`, and `16:9` (D16).
  3. The sort menu: aspect, and "wide screen" (distance from 16:9).

  None can say "not 16:9", "narrower than 16:9" (boxier), or "wider than 16:9" (ultrawide). The sort by aspect gives a useful range, but not quite the right one. Phase 5's negative search adds `-16:9`; the comparisons `<16:9` and `>16:9` remain for this phase.
  - Idea to weigh: the typed chips become the one filter language, with `<16:9`, `>16:9`, and `not 16:9`. The popover becomes a clickable way to add the same chips, and "closest to 16:9" stays a sort.
- **The sort button** is only a down arrow, with no name.
- **The "Filter" label** sits beside a page title.
- **The "Photos" and "Library" titles** may be redundant; the layout explains itself.
- **The menu button moves** to the left of the search box when search opens. It needs a fixed position.
- **Search placement:** the search box could appear centred at the top of the viewer. GNOME Files puts search in the header's centre.
- **Image actions in the viewer:** many image actions (copies, crop, Compare, file actions) could also be reached from the viewer itself:
  - right-click, or a modifier-click, on the image
  - controls that appear on hover
  - context-dependent controls

  D21 put them in the filmstrip's right-click menu with keyboard access. This extends the question to the viewer and keeps one action model, so menus can't drift apart.
- **More notes from the user (October 14):**
  - **"Add to Collection…" doesn't close on click-away.** It is an `AdwAlertDialog`, and Libadwaita dialogs ignore clicks outside them by design (only Escape or Done close them). A quick pick fits better as a popover anchored to the clicked image, or the drop target, which closes on click-away. Ctrl+B uses the same picker. **Decided October 15 (D25):** it stays a centred window and closes on a click outside, like Cancel; the WebP export gets the same. Fixed in the round after `ef91d32`, ahead of this phase.
  - **"Collection" or "bucket".** The interface still says "Collection" (menus, dialogs, tooltips) where the product, the docs, and the user say "bucket". Pick one word everywhere.
  - **The Info panel** opens as a second, bigger pill above the small corner chip. A better design is for the chip itself to expand on hover (the context idea above). This revisits D18, which put details in an Info panel opened by a key or the context menu, so it needs a decision.
  - **Fullscreen:** the corner chip should hide, and reappear on mouse movement, like a video player's controls.
- **One library root (user, October 14).** Carry out D11 ("One library"). Today Sharpr keeps a list of libraries, each with its own root and a "top level" or "drill down" mode, plus a switcher. The live settings hold three: Pictures (active, with all 9 buckets), Downloads, and a "test" library on a network share.
  - **Target:** one action, "Open Library Root…". Every subfolder of the root that contains images belongs to the library and its index. Remove the library list, the switcher, and the folder-mode option. Disabled folders stay the way to leave parts of the tree out.
  - **Changing the root keeps what is known.** Nothing is re-read just because the root moved:
    - Fingerprints are cached by path, vectors by fingerprint, and bucket members by path and fingerprint, so none of them depend on the root.
    - Moving the root up a level (say `~/Pictures` → `~`) keeps every existing fingerprint, vector, and bucket. Only the newly included folders are fingerprinted and indexed.
    - Moving it down hides what falls outside without deleting anything. Bucket members outside the root are hidden, not counted as missing; this is checked in `buckets::resolve`. Moving back shows them again.
    - Keep the same library identity when the root changes. Buckets carry a `library_id`, so changing the root must edit the one library, never create a new one.
  - **Migration:** the active library becomes the single library. The others' settings stay stored, unused, under the "user data survives" rule; nothing is deleted. If the user wants Downloads included, the choice is a common parent root with disabled folders. That is the user's decision at migration time, since a wider root means more indexing.
  - **Acceptance:**
    - Changing the root to its parent keeps every bucket's members and every fingerprint and vector, with no re-indexing of known files.
    - Narrowing the root hides, and widening restores.
    - No library data is deleted.
    - The library list and folder-mode controls are gone from the interface.
- **Phase 5 and later additions:** the search status line, "Show More", and "Add to Bucket…"; the Library Overview and its header ring; the Phase 6 export and Phase 7 cleanup controls. All are reviewed here, wherever they first landed.

**Visual polish from the Strata research** (`docs/research/strata-design-and-agent-setup.md`, Tier 1). The finding: Strata's clean look comes from discipline, not from dropping Libadwaita. Sharpr's "default GNOME" look comes from these gaps:
- **One stylesheet.** Move all CSS into `sharpr/data/style.css`, which `AdwApplication` loads itself. Today 13 CSS providers in 7 files load it.
- **Name every colour once.** Build Sharpr tokens on Libadwaita's CSS variables. The bucket palette becomes one Rust constant (it is copied into 5 files), and no hex or `rgba` literals remain in `sharpr/src/ui` outside the token definitions.
- **Quieter headers.** Use flat toolbars and drop the self-evident "Library" and "Photos" titles. Pane separators become 1 px hairlines.
- **Numbers that look like numbers.** Use `.numeric` (tabular figures) for resolution, counts, and zoom, and consider `.monospace` for the resolution and format chip.
- **Small dim section labels** (`.caption-heading` with `.dimmed`).
- **Sharpr's own symbolic icons,** namespaced `sharpr-*-symbolic`, for its concepts: bucket, add to bucket, Compare, crop, WebP export, below 4K, and copies. Record their source and licence; system icons stay for standard actions.
- **Skeleton tiles** while thumbnails load, and restrained hover and press feedback. These are CSS only; the thumbnail scheduler stays untouched.
- **The viewer and Compare canvas stay neutral** whatever the theme, so image colours aren't skewed.

**Decision to make in this phase: is Libadwaita an asset or an anchor?** Decide it by prototype, not on paper.
1. Do the polish above.
2. On a separate branch, prototype a single unified top row (sidebar toggle, search, and actions across the window), still with Libadwaita, as the Strata report's Tier 2 describes.
3. Compare screenshots side by side with Strata's and the current Sharpr, at 1024×600 and at a wide size.

If the result reaches the look the user wants, Libadwaita stays and the question is closed with that evidence. If not, dropping it becomes its own decision, with a decision-log entry and probably its own phase.
- **What dropping it costs:** Sharpr uses Libadwaita in 188 places across 13 files. Light, dark, and high-contrast themes, the accent colour, the adaptive split views and breakpoints, dialogs, toasts, Preferences, reduce-motion, and the user's window-button layout would all need rebuilding. Strata's stylesheet alone is 7,091 lines.
- Claude's view going in (October 14): it is an asset that Sharpr hasn't styled yet.

**Acceptance:**
- Every control in the inventory is either kept as it is, or changed or removed by a recorded decision.
- The Libadwaita question is decided by the user from the prototype screenshots and recorded in the decision log.
- The polish holds: one stylesheet, no stray colour literals, and light, dark, and high-contrast all readable at 1024×600. Filmstrip scroll and folder-switch bench logs are unchanged.
- No action has two unrelated entry points that can disagree.
- Screenshot tests at 1024×600 and wide sizes pass, and nothing overflows or moves when search opens.
- The user approves the result in a manual pass.

## Phase 10: Release

**Status: released as 0.5.0 on October 15, 2026** (tag `v0.5.0`, installed natively). The user's choices are D34. Checks: `./check.sh` (218 tests), 19 GTK workflows, the full Flatpak build (from `38a0758`) with GPU indexing verified in the sandbox, and the user's manual pass on the real library, which found one viewer bug (fullscreen fit, fixed in `d25bf91`). Performance against Phase 0: startup 470 ms (498), the 2,213-image folder 139–231 ms, median 155 (116–162), RSS peak 1,522 MB (2,345). The Flatpak bundle is rebuilt from the tag on request.

**Outcome:** A versioned build the user can install and rely on.

**Scope:**

- **One version:** a single version source for `build.rs`, metainfo, and the bundle name.
- **Flatpak:**
  - Build it from clean source pinned to a commit (a Git source, or a sync using `--delete-excluded`).
  - GPU indexing: also bundle a Vulkan build of llama.cpp. Today the Flatpak indexes on the processor only; the native build uses the GPU (Phase 5).
  - Document or fix the `home:ro` limits on move and rename.
- **Desktop integration:**
  - Make "Open with Sharpr" work: handle opened files, or drop `%F` and `MimeType`.
  - Set `StartupWMClass` to match the app ID.
- **Dead code and stale config:**
  - Remove `FocusedImageSet` and the stale `deny.toml` advisory ignore.
  - Remove the rexiv2/gexiv2 dependency and its Flatpak modules if they are confirmed unused.
  - Removed-feature GSettings keys may stay. They are inert.
- **README:** update the feature list to match `docs/reference/product.md`.
- **Release checklist:**
  - `./check.sh`, the GTK tests, and a full Flatpak build pass.
  - A manual workflow pass on the real library: find, curate, prepare a wallpaper, clean up, compare, and view, as listed in `product.md`.
  - Performance compared against the Phase 0 baseline.
- Tag the version and fast-forward `main`. Whether to push is the user's decision.

**Acceptance:** Every item on the checklist passes, and the installed build is the tagged commit.

---

## Next: Edit Buckets (D35)

**Status: accepted by the user on October 16, 2026** ("works"; build `f50cd9f`) and merged into `main`. `./check.sh` (220 tests) and all 20 GTK workflows pass, including a new end-to-end test of crop, batch export, Duplicate, the switch and unchanged originals.

**Added October 15 at the user's request,** after 0.5.0. Editing and Compare work together: when Crop Copy, WebP export or Duplicate creates a copy, the original and the copy both join a bucket for that action ("Cropped", "Exported", "Duplicated", under one "Edits" parent), created when first needed. Comparing a copy with its original is then: open the bucket, select the pair, Compare, wherever the copy was saved.
- On by default; a Preferences switch turns it off.
- Uses the existing bucket writer off the GTK thread; files are never moved or changed. A copy saved outside the library root is hidden like any outside member, so say so when that happens.
- A batch export adds every original and copy; the user removes entries or the bucket when done.
- **Acceptance:** each action puts both images in the right bucket, once, with no duplicate entries; turning the switch off stops it; originals stay byte-identical; Compare from the bucket shows the pair; tests cover crop, batch export and Duplicate.

## Crop Copy source format (D37)

Requested October 16, 2026 for cropping before external upscaling. Crop Copy
keeps the source format, including JPEG; batch Export as WebP stays unchanged.
JPEG crops are re-encoded at maximum quality (tradeoff accepted by the user).
PNG/TIFF sample depth and transparency stay intact; WebP/JPEG XL crops are
lossless. Unsupported encoders report an error instead of silently converting.
Originals remain byte-identical and output names are exclusive and unique.

**Status:** accepted by the user on October 17, 2026 (deployed as `cabda19`) and merged to `main`.
`./check.sh` passes (236 tests), all 27 isolated GTK workflows pass, and
regressions cover source formats, 16-bit/float samples, alpha, EXIF orientation,
unique names, original bytes and unchanged batch WebP export.
**Acceptance:** crop PNG/JPEG and other supported sources into the same format,
open the copy in an external tool, and verify batch export still writes WebP.

## Planned: 16:9 Viewer Guide (D36)

**Status: accepted with notes and merged** on October 17, 2026 (deployed as `c724bab`). Ctrl+G or image menu → 16:9 Guide; inside the image, outside dimmed.

**Outcome:** Judge at a glance whether an image is wider or taller than a
16:9 landscape wallpaper, using a toggleable border/ruler in the viewer.

**Scope:**

- Draw a clear 16:9 reference outline over the displayed image, with a simple
  on/off control. Keep the aid available while browsing images and in fullscreen.
- Make the comparison clear: a wider image extends beyond the guide at its
  sides; a taller image extends above and below; an exact 16:9 image aligns.
  Settle the outline's placement and interaction when implementing the feature.
- Keep the guide aligned with the displayed image through viewer resizing,
  zoom and pan. It is a visual aid only; it changes no pixels or files and
  starts no crop or export operation.
- Reuse the existing viewer overlay/drawing patterns; toggling and navigation
  stay responsive, with no new image decoding or thumbnail-scheduler changes.

**Acceptance:**

- Toggle the guide on and off without entering Crop Copy.
- Wide, taller landscape, square, portrait and exact 16:9 examples are easy to
  distinguish in the normal viewer and fullscreen.
- The guide remains geometrically correct across image changes, window sizes,
  zoom and pan; originals stay byte-identical.
- Add focused geometry/GTK checks and document the control in the guide,
  shortcuts (if a key is assigned) and interface inventory when implemented.

## Work Order And Claude Model

Rated October 17, 2026 at the user's request. The user launches a fresh Claude session per task with the listed model and effort. Asked "what's next?", an agent names the first open row. Scale: Sonnet medium for bounded work with a clear spec; Sonnet high for threading, persistence or thumbnail-scheduler work; Opus where an open design question meets a data-safety or concurrency risk; Haiku for docs only. One model per task. Update this table when a task finishes or the order changes.

| # | Task | Model, effort |
| --- | --- | --- |
| 1 | ~~Crop Copy source format (D37): user acceptance, then merge `codex/audit-0.5.0`~~ **Done October 17, 2026** | Sonnet medium |
| 2 | ~~`cargo deny` fails on new "unmaintained" advisories for zbus, zvariant, zvariant_derive and zbus_names (via glycin): ignore them in `deny.toml` with a reason, as for `paste`, so `./check.sh` passes again~~ **Done October 17, 2026** (RUSTSEC-2026-0370/0371/0372 ignored; `zbus` itself had no advisory) | Sonnet low |
| 3 | ~~16:9 viewer guide (D36)~~ **Done October 17, 2026**, accepted with notes (see Backlog) | Sonnet medium |
| 4 | ~~16:9 guide follow-ups (D38): corners resize, outline moves, stays put on zoom, Crop Copy starts on the guide.~~ **Done October 17, 2026**, accepted by the user ("it works", build `a795ae1`) | Sonnet medium (done by Opus at the user's request) |
| 5 | ~~Crop Copy zoom (user, October 17, after D38): open at the viewer's zoom and position so the selection is where the guide was, and zoom/pan in crop with the selection attached to the image.~~ **Done October 17, 2026** (D39), accepted by the user ("it is fixed", build `e04e361`) | Sonnet high |
| 6 | ~~Move file move, copy, and trash operations off the GTK thread~~ **Done October 17, 2026**, accepted by the user (build `5f8b8d0`; follow-up "Show file-action results in the open folder" in the Backlog) | Opus medium |
| 7 | ~~Keep thumbnail generations through to completion, together with the folder-load token~~ **Done October 17, 2026**, accepted by the user (build `790e2a4`) | Sonnet high |
| 8 | ~~Compare: bounded decodes and a screen-sized preview~~ **Done October 17, 2026**, accepted by the user ("all tested working", build `6bf37cd`) | Sonnet high |
| 9 | ~~Show file-action results in the open folder: Duplicate, Copy to Folder, Rename, and moves into the open folder appear in the filmstrip straight away, keeping the selection and the thumbnail scheduler's state (user, October 17; after item 7's folder-load token)~~ **Done October 17, 2026**, accepted by the user ("everything works", build `790e2a4`) | Sonnet high |
| 10 | ~~A broad library root such as Home: the folder walks (sidebar tree and library-wide paths) stay on the root's own drive, skip hidden folders, and the sidebar fills in as it scans (user, October 17: Home as root showed nothing, because the walk crawled the NAS mounts under `~/Network`)~~ **Done October 17, 2026**, accepted by the user ("changing root works", test build `7bb4aa6`) | Sonnet high |
| 11 | A folder created in the Move to Folder (or Copy to Folder) chooser is not indexed: the files land there but the new folder does not appear in the library index or sidebar (user, October 18). The same happens to folders and images the user adds outside Sharpr while it runs (user, October 18), so this is really "notice changes under the library root": nothing watches the file system today. Prefer file-system events (`gio::FileMonitor`, one per watched folder, local drives only) plus a rescan when the window regains focus over a timer poll. After `move_to_folder` / `copy_to_folder` (`ui/window/file_jobs_controller.rs`), index the destination directly | Sonnet high |
| 12 | Tune Find Copies (ask the user for missed and false examples first) | Sonnet high |
| 13 | Re-link a bucket member moved outside Sharpr | Opus medium |
| 14 | Guard or retire the startup legacy-bucket migration | Sonnet high |
| 15 | Stop computing perceptual hashes in the thumbnail workers | Sonnet medium |
| 16 | Use one sort implementation | Sonnet medium |
| 17 | Merge the two thumbnail worker loops | Sonnet high |
| 18 | Library Overview refresh, errors, and unsupported count | Sonnet medium |
| 19 | Ctrl+Shift+S opens the GTK Inspector | Sonnet low |
| 20 | Make the flaky GTK navigation test deterministic | Sonnet high |
| 21 | Split `SharprWindow::setup` and `filmstrip::build_ui`, piecemeal | Sonnet high |
| 22 | AVIF and HEIC/HEIF indexing (the user approves the decoders first) | Sonnet high |
| 23 | Trash Undo in the Flatpak (needs a Flatpak build to verify) | Sonnet high |
| 24 | Non-UTF-8 paths (low value; paths are stored as SQLite text) | Opus high |
| 25 | More than one library root (user, October 18): keep Pictures as the root and add Downloads as a second top-level root in the folder list, below the first. Today one library is active and `set_single_library_root` replaces its root; `config/settings.rs` already holds several libraries with an overlap check, so decide first whether a second root is a second library or a second root of one library (buckets, tags, search, Find Copies and the index must span it). Adding and removing a root must never touch the files. Maybe: each root gets its own colour, so a root's folders and images read at a glance (user idea, October 18; decide with the design) | Opus medium |

## Backlog

Optional. Pick something up only when a phase touches that area, or the user asks. The suggested order and model are in the table above.

- More than one library root (Work Order item 25, user, October 18): the sidebar lists each root as a top-level entry; "Add Folder…" adds one, and removing one only forgets it. Library-wide paths (search, Find Copies, Survey, the indexer and the Library Overview) must cover every root, and the existing rule that roots may not overlap stays. Keep the single-root case looking exactly as it does now. Optional: a colour per root (user idea) shown in the sidebar and perhaps as a thin mark on cards; it must work in light and dark and not rely on colour alone.

- A folder created while choosing a destination is not indexed (Work Order item 11, user, October 18): in "Move to Folder…", the user made a new folder in the chooser and moved images into it, and the new folder was not indexed. `move_to_folder` and `copy_to_folder` (`ui/window/file_jobs_controller.rs`) only add the moved rows to the view with `show_new_files`; nothing asks the indexer to pass over the destination. Check whether a library-root folder that appears mid-session (also the sidebar tree) is ever picked up, and whether Copy to Folder behaves the same. Wrap or extend the existing index and thumbnail scheduling; don't rewrite it.

- A broad library root (Work Order item 10, user, October 17): choosing Home as the root left the sidebar empty. `build_folder_tree` (`ui/sidebar.rs`) and `collect_library_image_paths_cancellable` (`model/library.rs`) walk every folder under the root, including hidden ones (`.cache`, `.steam`, `.cargo`) and the NFS shares mounted under `~/Network/Gigabyte`, and the sidebar shows nothing until the whole walk ends. Stay on the root's filesystem (a mount inside it is added on purpose by choosing it, or as its own root), skip hidden folders, publish the tree progressively, and consider a note when the root is a home folder. The indexer must not crawl the NAS either.

- One guide-and-crop tool (user idea, October 17; D38): whether the 16:9 guide and Crop Copy become a single tool, perhaps with other ratios such as 21:9 or 4:3. Decide after the user has used D38's Crop Copy from the guide for a while.
- Split the rest of `SharprWindow::setup` and `filmstrip::build_ui` (about 700 lines), following the architecture decision above.
- Merge the two near-identical thumbnail worker loops.
- Use one sort implementation; the SQL and Rust versions currently break ties differently.
- Support non-UTF-8 paths.
- Guard the startup legacy-bucket migration (`migrate_legacy_collections_to_tags`) with a backup and count check, or retire it, before old databases are treated as a supported upgrade (audit finding 12).
- Ctrl+Shift+S opened the GTK Inspector instead of Survey on the user's desktop (October 15). Find what claims the key, or choose another.
- Re-link a bucket member moved outside Sharpr before the user opens its new folder. Today the bucket shows "image not found" until a pass reaches that folder (user, October 15).
- Tune Find Copies on the real library: the user found it "improved but not perfect" (October 15).
- Stop computing perceptual hashes in the thumbnail workers, now that "Find Copies…" (Phase 7) uses fingerprints and vectors; remove `duplicates/phash.rs` and `LibraryIndex::duplicate_groups` with it. Wrap or extend the thumbnail scheduler; don't rewrite it.
- Index AVIF and HEIC/HEIF for visual search, with a bounded decoder on the worker (Phase 5 audit, finding 13). The live library has none; weigh the Flatpak cost before adding decoders.
- Library Overview: refresh the fingerprint, copy, bucket, and index-file figures while it is open, or label them as a snapshot; show gathering errors instead of "…"; count unsupported formats apart from unreadable files (Phases 6–8 audit, finding 12).
- Trash Undo in the Flatpak (D25): the sandbox trashes through the portal into the host's trash, which Undo, reading `$XDG_DATA_HOME/Trash`, may not find. It then reports the file as not restorable, and it stays in the trash. Check this in the Phase 10 Flatpak build.
- GTK test harness: `rapid_navigation_and_folder_switch_preserve_latest_view` once saw 0 rows (Phase 5 audit, test runs); make it deterministic. The dropdown selection test was replaced by direct candidate-row activation in D29. The screenshot timeout is solved (October 14): a window the compositor isn't drawing (covered, for example by a fullscreen video while the user works) gives no frame, so screenshots are now best-effort and never fail a test.

### Considered, Not Planned

Ideas weighed and set aside, with the reason, so they aren't proposed again without new facts.

- **Running the embedding model on another machine** (a LAN server or an internet API, October 14). It would break the offline invariant and would need a decision-log entry. Images would leave the machine, and a remote model can't prove which files it runs, so mismatched vectors could mix into the index. GPU indexing on the laptop covered the speed need. If it ever comes back: LAN only, opt-in, and checked against a reference image.
- **Shorter vectors** (256 of the 768 numbers, Matryoshka truncation). The whole index is about 18 MB, too small for the saving to be worth the lost accuracy.
- **Indexing file and folder names together with each image** (interleaved input). It would change the vectors the user judged in Phase 3, and most file names say nothing about the picture. Separate file-name matching covers it.

## Out Of Scope

See the non-goals in `docs/reference/product.md` and the removed features in `AGENTS.md`. Neither list is reintroduced without the user's approval and a decision-log entry.
