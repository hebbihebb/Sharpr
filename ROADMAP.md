# Sharpr Roadmap

This is the only roadmap. `CURRENT_TASKS.md` tracks which phase is active. When a phase finishes, summarize it in `docs/archive/history.md` and delete it from this file.

## Direction

Sharpr is a fast, local, offline Linux image organizer and wallpaper-preparation app. Its core is the viewer, filmstrip, folders, and buckets. Originals are never modified; preparation creates new copies. Enhancement and upscaling stay in external tools.

Current priorities:
1. Keep tags, buckets, and files safe.
2. Make finding images in large folders easy.
3. Decide on offline visual search from evidence.
4. Finish crop and WebP preparation.

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
- Move each area out of `window.rs` into `ui/window/` controllers when a phase changes it (Phases 1, 2, and 5).
- Do not run a separate refactor phase or feature freeze.
- Target: no new feature logic in `setup()`, and by Phase 6 no function longer than about 500 lines in the areas phases touched.

**Revisit triggers:** a Phase 1–2 fix requires replacing the thumbnail scheduler or `LibraryManager` wholesale, or GTK/Libadwaita changes that force a broad UI rewrite anyway.

## Order

```text
Phase 0 → Phase 1 → Phase 2 ─┬→ Phase 3 → (go) Phase 4 ─┐
                             └→ Phase 5 ────────────────┴→ Phase 6
```

- Phases 0–2 and 6 are mandatory.
- Phase 3 is a mandatory evaluation.
- Phase 4 happens only if Phase 3 says go.
- Phase 5 is small and can run any time after Phase 0.

---

## Phase 0: Checkpoint And Trustworthy Baseline

**Status:** next.

**Outcome:** All of the October 12 work is committed in reviewable units, the build is green, the user runs a known build, and user data is backed up.

**Scope:**

1. ~~Fold the documentation realignment into `viewer-reset`.~~ Done on October 12.
2. **Make the tree build and pass `./check.sh` with default features.** `main.rs` declares `semantic_trial`, but `src/semantic_trial.rs` does not exist yet. Because `check.sh` uses `--all-features`, either finish the module or move the trial wiring out of the ordinary commits.
3. **Commit the uncommitted work as separate units:**
   - (a) search, filters, aspect/resolution, manual-tag persistence fixes, and the GTK test
   - (b) Crop and WebP copies with the export safety fixes
   - (c) benchmark tools and `docs/research/image-search.md`; private artifacts stay in `.tmp/`
   - (d) the experimental visual-search trial, last and separately, only once it compiles with its feature

   Use the existing self-audit; do not start a new audit round. Leftover findings belong to Phases 1, 2, and 5.
4. **Back up the real data** using the commands in `docs/reference/development.md`.
5. **Separate "deployed" from "dev".** Install a release build of the accepted commit (with compiled schemas) to a fixed location outside the checkout. Point the launcher there; update `tools/install-native-launcher.py`.
6. **Record a performance baseline** on the real library with the release build: folder open, thumbnail apply latency, and rapid scroll, using bench logs. Write the numbers in `CURRENT_TASKS.md`.
7. **After the user accepts the build:**
   - Fast-forward `main` to `viewer-reset` (agreed in principle on October 12; confirm at the time).
   - With approval, delete the six fully merged old branches and the `Sharpr-Base` worktree (`viewer-base`).

**Acceptance:**

- `viewer-reset` has a clean working tree, and `cargo build`, `./check.sh`, and the GTK test pass at its head.
- The launcher runs a release build of a named commit, and a `cargo build` in the checkout does not change it.
- The user opens their library with folders, all 6 buckets, and their manual tags intact, and accepts search, filters, and crop/WebP after manual testing.
- `main` equals `viewer-reset`.

## Phase 1: Data-Safety Repairs

**Outcome:** No Sharpr action can silently lose tags, bucket membership, or image files. Every defect below is verified in the code and exists at committed `HEAD`.

**Scope:**

1. **Stop deleting tag rows for missing files.** `TagDatabase::all_tags()` and `paths_for_tag()` currently delete tags for every path that doesn't exist, and the Tags page calls them on every refresh. Files moved outside Sharpr, or an unmounted root, lose their tags permanently.
   - Filter missing paths when reading instead, and keep the rows.
   - Replace the test that enshrines pruning (`tags/db.rs`).
2. **Rename never overwrites.** The in-app rename uses `std::fs::rename` with no checks.
   - Reject path separators and `..`.
   - Refuse an existing target name (no-replace rename).
   - Report `rename_path` tag-update failures.
3. **Bucket edits remove only the bucket's own tags.**
   - Removing an image from a child bucket must not strip inherited ancestor tags, which currently also drops the image from sibling buckets.
   - Reparenting drops a tag only when no remaining lineage needs it.
   - Renaming a parent includes members that are in disabled folders.
   - Add parent/child/sibling regression tests.
4. **Thumbnail workers cannot deadlock.**
   - Workers must not `send_blocking` hash follow-ups into their own bounded queue (`thumbnails/worker.rs`).
   - The stale-drain loop must not discard hash or shutdown messages.
   - Add a stress test that exceeds the queue capacity.
5. **Add real schema versioning.**
   - Set `PRAGMA user_version` on both databases.
   - A binary that finds a newer version refuses to write and shows an error.
   - Delete the compiled-out `#[cfg(any())]` pipeline code. The legacy tables stay.
6. **Guard broad actions.**
   - Tags page "Delete tag": confirm first, naming the affected buckets, and run off the GTK thread.
   - Reject bucket names that differ only by case.
   - Delete and other single-key shortcuts must not fire while a text entry has focus.
   - Report trash failures.

**Acceptance:**

- Each item has a regression test, or a stated manual check where a test is impractical.
- A test shows that a missing library root deletes nothing.
- A copy of the live databases opens and round-trips with identical tag and bucket counts under the new build.
- `./check.sh` passes.

**Not in scope:** merging the two databases, and crash-atomic bucket edits (see Backlog).

## Phase 2: Finding Images In Large Folders

**Outcome:** Tagging, searching, and filtering stay quick in a library of 5,000–50,000 images, without stalling the filmstrip.

**Already implemented:** these are uncommitted until Phase 0.
- scoped search with a whole-library toggle
- 150 ms debounce and a single coalescing worker
- aspect sort in both directions, plus closest to 16:9
- landscape, portrait, square, and 16:9 ±2% filters
- minimum width and height
- tag edits moved off the GTK thread
- automatic tags no longer created

**Scope:**

1. **Add a measuring tool first.** Add a main-loop stall probe to `bench.rs`, such as a lateness measurement on a periodic timer, plus a small log-summary script. This makes "doesn't stall" testable.
2. **Clean up legacy automatic tags** (approved by the user on October 12). 7,565 of the 7,925 live tag rows are format and resolution tags (`jpg`, `4k`, `1080p`, `png`, …), and no bucket uses them.
   - Make it an explicit one-time action with a backup, a count preview, and confirmation.
   - It removes only the known automatic tag names that no bucket references, and never touches manual tags.
   - Delete the dead `tags/indexer.rs`.
3. **Match whole tags or tag prefixes** instead of `%substring%`, so "jp" stops matching almost everything.
4. **Allow tagging a multi-selection** from the tag popover.
5. **Make tag information truthful.**
   - The viewer's bucket chip shows real membership (every effective tag) within the active library.
   - Tags-page counts are per active library.
6. **Use one source for dimensions:** the library index's oriented width and height. Remove the extra `DimensionCache`s and the repeated header reads.
7. **Move the remaining GTK-thread work off it:**
   - Fill the tag lookup during filmstrip bind off-thread and cache it per entry.
   - Open buckets on a worker with the checked query and a chunked store load.
   - Replace the linear `entries_for_path` scans in virtual views with the path index.
   - Count sidebar buckets off-thread.
   - Compute duplicate groups once per change, not on every image navigation. It is currently O(n²) per arrow key.
   - Cache the whole-library search candidate list for each search session.
8. **Extract the code this phase changes from `window.rs`:** bucket open/edit and file actions move into `ui/window/` controllers, following the existing `search_controller` and `tag_controller`.

**Acceptance:**

- With a release build on the real library, typing a search, toggling filters, and opening buckets cause no main-loop stall over 100 ms in the probe.
- The Phase 0 baseline regresses by no more than 10%.
- With a 50,000-entry synthetic fixture, search, filter, and sort finish on the worker in under 1 s, and image navigation does no work proportional to library size.
- Tagging 20 selected images takes one popover action.
- After cleanup, autocomplete and the Tags page show only manual tags, and every bucket's member count is unchanged.

## Phase 3: Visual-Search Evaluation

**Outcome:** A recorded go/no-go decision and model/runtime choice based on evidence. No production integration happens in this phase.

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

1. **Judge relevance with the agreed rubric.** A query passes when the top 20 contain at least 10 relevant results, including at least one in the top 5. At least 24 of 30 queries must pass, and unjudged queries fail. A contact sheet or the trial UI may be used for judging.
2. **Make runs reproducible.** Every report records:
   - model and projector hashes
   - quantization
   - runtime and library versions
   - preprocessing
   - thread count

   Add a parity script that computes cosine agreement and top-20 overlap between native and reference vectors.
3. **Measure filmstrip impact while indexing** with the Phase 2 probe. The limit is a 10% slowdown.
4. **Check packaging.**
   - Can llama.cpp ship as a Flatpak module, or will this be native-only?
   - Record the disk and memory footprint.
   - SigLIP 2 also needs a shippable native runtime. ONNX Runtime was removed in July for its size. Evaluate one only if SigLIP clearly wins relevance or Gemma fails.

**Decision rule:**
- Choose the candidate that passes relevance with the simplest runtime that can ship.
- When results are comparable, prefer EmbeddingGemma 2, because its native llama.cpp path is already proven.
- If no candidate passes, stop: no integration, and Names & tags stays the only search.

**Optional experiment, approved by the user:** an in-app trial built only with `--features visual-search-trial`.
- It queries the 1,000-image sample index through a local llama.cpp child process, off by default and not in the Flatpak.
- It lives in separate commits and supports the relevance judging. It is not integration.

**Acceptance:** The decision, with its numbers, is recorded in this file, and the reports can be reproduced from their recorded provenance.

## Phase 4: Visual-Search Integration

**Status:** conditional on a Phase 3 go.

**Outcome:** "Visual" becomes a second mode of the existing search entry. Names & tags stays the default.

**Scope:**

- **Search behavior:**
  - Apply scope, disabled folders, and metadata filters before ranking.
  - Show the closest 200 results with a "load more" option.
  - Keep results stable until the query, scope, or filters change.
  - Show coverage as indexed versus eligible, and distinguish "not indexed" from "no good match".
- **Vector storage:** vectors go in their own SQLite file, separate from tags and the index, so it is safe to delete. Records are versioned by model and preprocessing and keyed by file identity (path, size, mtime). Incompatible or stale vectors are rejected.
- **Indexing:**
  - Index the whole enabled library in the background at the lowest priority.
  - It is resumable and stops cleanly on quit.
  - Errors are counted and reported.
- **Model runtime:**
  - The app manages a local child process and talks to it only over authenticated loopback.
  - The user supplies the model files at a documented path; the app downloads nothing.
- **Packaging:** native builds are supported. The Flatpak is supported only if Phase 3 proved it feasible.

**Acceptance:**

- Phase 3 relevance and latency hold inside the app.
- Browsing slows by no more than 10% during indexing.
- A missing or mismatched model shows a clear message, and Names & tags keeps working.
- Deleting the vector file loses nothing else.

**Later (optional):** Find similar, per-folder indexing, scheduling controls, and incremental updates when files change.

## Phase 5: Wallpaper Preparation

**Outcome:** The user can turn any image into a correctly oriented 16:9 wallpaper copy in one short flow.

**Already implemented:** these are uncommitted until Phase 0.
- Crop Copy: a movable 16:9 selection, or free aspect
- WebP Copy: quality 90 by default, original size or resized
- intermediate PNG copies
- exclusive output reservation and checked flushes, with original-byte tests

**Scope:**

1. If the hidden partial file collides with an existing file, move on to the next free output name instead of failing.
2. When glycin cannot preview a format, fall back to a crop preview decoded and oriented on a worker. Add a test with an EXIF-rotated source.
3. Remember the last destination, offer a Wallpapers folder, and let the user open the result beside its source in Compare.

**Acceptance:**

- Cropping EXIF-rotated JPEG and PNG sources produces correct pixels and orientation.
- Originals stay byte-identical, and a name collision never overwrites a file.
- The user goes from a source image to a 16:9 WebP wallpaper in one manual run.

## Phase 6: Release Readiness

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
- **README:** update the feature list.
- **Release checklist:**
  - `./check.sh`, the GTK test, and a full Flatpak build pass.
  - A manual workflow pass on the real library: open a folder, scroll, tag, use buckets, search, filter, Crop/WebP Copy, and trash.
  - Performance compared against the Phase 0 baseline.
- Tag the version and fast-forward `main`. Whether to push is the user's decision.

**Acceptance:** Every item on the checklist passes, and the installed build is the tagged commit.

---

## Backlog

Optional and unordered. Pick something up only when a phase touches that area, or the user asks.

- Close the two-database crash window by moving tags into the index database or adding a startup reconciliation pass. The window is currently accepted.
- Split the rest of `SharprWindow::setup` and `filmstrip::build_ui` (about 700 lines), following the architecture decision above.
- Merge the two near-identical thumbnail worker loops. Bound active-view thumbnail memory in folders of 10,000+ images. Keep thumbnails across sort and search reloads.
- Use one sort implementation; the SQL and Rust versions currently break ties differently.
- Move file move, copy, and trash operations off the GTK thread.
- Add a folder-load token so two loads of the same folder cannot both append.
- Support non-UTF-8 paths in tag keys.
- Make Flatpak `home` access writable for move and rename, if the user needs it.

## Out Of Scope

These items will not be reintroduced:
- Tasks, processing queues, pipelines, and generation history
- AI upscaling, enhancement, and generated or descriptive tags
- ComfyUI and remote inference
- model downloaders
- rotate/flip pixel editing and metadata write-back
- a full editor, import workflow, saved searches, batch rename, splash screen, and scripting
- colour management, unless the user asks for it
