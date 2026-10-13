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
- Target: no new feature logic in `setup()`, and by Phase 9 no function longer than about 500 lines in the areas phases touched.

**Revisit triggers:** a fix requires replacing the thumbnail scheduler or `LibraryManager` wholesale, or GTK/Libadwaita changes force a broad UI rewrite anyway.

## Order

```text
Phase 1 ─┬→ Phase 2 Finding ──────────────────┐
         ├→ Phase 3 Visual-search evaluation ─┤
         └→ Phase 4 File identity + buckets ──┴→ Phase 5 Visual search + tag retirement (go only)
            then Phase 6 Wallpaper output → Phase 7 Cleanup → Phase 8 Compare → Phase 9 Release
```

- **Phase 0 is done**; see `docs/archive/history.md`.
- **Mandatory:** Phases 1, 2, 4, 6, 7, 8, and 9. Phase 3 is a mandatory evaluation; Phase 5 happens only if Phase 3 says go.
- **Finding comes first** after the data-safety work (D19). Phases 2 and 3 can run in parallel, because Phase 3 is mostly human relevance judging.
- **Phase 4 is shared groundwork.** Phase 5 (vector identity) and Phase 7 (exact duplicates) both need it.
- **Phases 6–8 depend only on Phase 1.** The user may reorder them. Phase 6 needs nothing from Phase 4, so it can move earlier if wallpaper output becomes urgent.

---

## Phase 1: Data-Safety Repairs

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
11. ~~Move the remaining GTK-thread work off it.~~ Done October 12: badge colours, duplicate grouping, bucket open, virtual-view lookups, and the whole-library search cache. Sidebar counts were already on a worker.
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
4. **Measure filmstrip impact while indexing** with the Phase 2 probe. The limit is a 10% slowdown.
5. **Check packaging.**
   - Can llama.cpp ship as a Flatpak module, or will this be native-only?
   - Record the disk and memory footprint.
   - SigLIP 2 also needs a shippable native runtime. ONNX Runtime was removed in July for its size. Evaluate one only if SigLIP clearly wins relevance or Gemma fails.

**Decision rule:**
- Choose the candidate that passes relevance, in every category, with the simplest runtime that can ship.
- When results are comparable, prefer EmbeddingGemma 2, because its native llama.cpp path is already proven.
- Try another candidate only if every current candidate fails, and say which category failed.
- If none passes, stop: no integration. Name search and typed filters remain, and tags stay frozen, not retired.

**Optional experiment, approved by the user:** an in-app trial over the 1,000-image sample index through a local llama.cpp child process. The unfinished wiring is preserved on `visual-search-trial-wip`. It is off by default, not in the Flatpak, and kept in separate commits. It supports judging; it is not integration.

**Acceptance:** The decision, with its numbers per category, is recorded in this file, and the reports can be reproduced from their recorded provenance.

## Phase 4: File Identity And Buckets

**Outcome:** Bucket membership survives moves and renames made outside Sharpr (D3), keeps inheritance (D4), and stops depending on tags. Every image gets a content fingerprint, which Phase 5 and Phase 7 reuse.

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

**Status:** conditional on a Phase 3 go.

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

**Later (optional):** Find similar, per-folder indexing, scheduling controls, and incremental updates when files change.

## Phase 6: Wallpaper Output

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
4. **Make the Wallpapers folder easy to reach** as a crop and export destination.

**Acceptance:**

- Exporting 100 selected images to lossless WebP, fitted within 3840×2160, shows progress and keeps browsing responsive.
- Cancelling leaves no partial files.
- Originals stay byte-identical, and a name collision never overwrites a file.
- Cropping EXIF-rotated JPEG and PNG sources produces correct pixels.
- The user completes a real wallpaper end to end.

## Phase 7: Cleanup

**Outcome:** The user reliably finds exact copies and resized or converted copies, then trashes the extras safely (D14). Images below 4K are found with the Phase 2 filter.

**Scope:**

1. **Find out why the current duplicate finder is inconsistent.** Suspects are incomplete perceptual-hash coverage, the threshold, and orientation. Confirm the cause before fixing it.
2. **Exact copies:** group by the Phase 4 fingerprint.
3. **Resized and converted copies:** a perceptual hash covering the whole enabled library, with the threshold tuned on the user's real examples. Crops and near-identical variants are not grouped.
4. **Review groups.** Each group shows resolution, format, and size, and suggests keeping the highest resolution. Trash only after confirmation, and report trash failures.

**Acceptance:**

- In a fixture, exact copies, a 1080p JPEG with its 4K PNG version, and a WebP conversion are grouped; a crop is not.
- Nothing is trashed without confirmation.
- Grouping runs off the GTK thread.

## Phase 8: Compare

**Outcome:** One on-demand Compare tool serves both version inspection and choosing the best of several (D8).

**Scope:**

- Select N ≥ 2 images; Compare shows two at a time.
- Modes: A/B flip (Space is a natural key for it), a draggable split, and side by side.
- Dismissing the current loser brings in the next candidate.
- A zoom-lock toggle keeps zoom and pan in sync: on for versions, off for different images.
- Closing returns to browsing with the survivors selected.
- Compare never changes files.
- Images from several folders arrive through a bucket (D10).

**Acceptance:**

- Five images narrow down to one through knockouts.
- With zoom lock on, flipping keeps the same region at 100%.
- The split slider works at any zoom.
- Leaving Compare restores browsing with the survivors selected.

## Phase 9: Release

**Outcome:** A versioned build the user can install and rely on.

**Scope:**

- **One version:** a single version source for `build.rs`, metainfo, and the bundle name.
- **Flatpak:**
  - Build it from clean source pinned to a commit (a Git source, or a sync using `--delete-excluded`).
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

## Backlog

Optional and unordered. Pick something up only when a phase touches that area, or the user asks.

- Split the rest of `SharprWindow::setup` and `filmstrip::build_ui` (about 700 lines), following the architecture decision above.
- Merge the two near-identical thumbnail worker loops. Keep thumbnails across sort and search reloads.
- Use one sort implementation; the SQL and Rust versions currently break ties differently.
- Move file move, copy, and trash operations off the GTK thread.
- Add a folder-load token so two loads of the same folder cannot both append.
- Support non-UTF-8 paths.
- Make Flatpak `home` access writable for move and rename, if the user needs it.

## Out Of Scope

See the non-goals in `docs/reference/product.md` and the removed features in `AGENTS.md`. Neither list is reintroduced without the user's approval and a decision-log entry.
