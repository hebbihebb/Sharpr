# Development History

A compact record of how Sharpr got here. It is historical context, not instructions; `AGENTS.md` and `ROADMAP.md` are authoritative. Commit-level detail lives in Git.

## April 2026: foundation

- GTK4/Libadwaita scaffold, folder browsing, filmstrip, viewer, metadata, and background thumbnail loading arrived quickly (about 230 commits).
- Background decode, metadata, quality scoring, persistent library index, tags, collections, and duplicate detection followed.
- Local AI tag suggestions and upscale backends (ONNX, CLI, ComfyUI) were explored.
- Lesson: when custom code became fragile, existing GTK, GNOME, system, or crate facilities usually worked better.

## May 2026: reliability and the Tasks era

- Reliability, a Tasks queue, Compare, and GNOME polish were the focus. `FocusedImageSet` became the model for non-folder filmstrip views, and context menus moved to `GMenu`/`GAction`.
- Completed tracks: thumbnail-scheduling and stale-generation regression tests, extraction of the Compare controller and collection dialogs, the Tasks dashboard (queue runner, flicker fix, batch editing), the Compare/viewer OSD, an action-map audit, collection/task-history migration tests, HIG writing-style and CSS theme audits, and a Rust version pin. The last archived plan item was step 14f (Tasks queue batch editing).
- Scope decided against: rotate/flip editing, ONNX downloader, sharpness backfill, splash screen, import workflow, saved searches, batch rename, and metadata writing.
- Planning then lived in `MASTER-PLAN.html`, `COMPLETED.html`, and `HISTORY.html`. They are now retired; the external-edit/image-family design from that period is `external-edit-workflow.md` in this folder.

## July 2026: viewer-first reset (`viewer-reset` branch)

- Commit `533c43e` removed about 12,000 lines: the Tasks dashboard and pipeline, every upscale backend and model asset, AI tag suggestions, generated-output comparison state, and runtime networking.
- Generic two-selection Compare and deterministic Resize Copy replaced them, and shared UI state plus schema initialization were isolated. Legacy task tables and settings keys were left inert rather than dropped, to protect user data.
- `main` was left at the pre-reset state (`2679e44`). `viewer-base` (`f1d937c`) marks the reset baseline.

## October 2026: wallpaper roadmap and the realignment

- October 12: Codex adopted a wallpaper-focused roadmap (`41a1a19`). It repaired the native launcher, implemented scoped search, manual-tag persistence fixes, aspect/resolution controls, and Crop/WebP copies in the working tree, and ran a semantic-search benchmark (EmbeddingGemma 2 reference and native llama.cpp, plus SigLIP 2) on a 1,000-image, 30-query sample.
- The same day produced four documentation-only commits: a self-audit, an independent audit (`audit-2026-10-12.md`), and a next-steps plan (`next-steps-2026-10-12.md`). Meanwhile about 2,400 lines of implementation stayed uncommitted. This review-and-remediation chain is why `AGENTS.md` now says "finish, then stop".
- Documentation realignment: `AGENTS.md` was rewritten to be concise and agent-neutral, `ROADMAP.md` was rebuilt from repository evidence, and `docs/reference/development.md` was added. The dated audit and plan documents moved here. `completed.md` was folded into this file. The unused `GEMINI.md` and `ANTIGRAVITY.md` stubs were removed. A clean V2 rewrite was evaluated and rejected in favor of refactoring areas as phases touch them (see `ROADMAP.md`).

## Lessons

- Protect filmstrip and thumbnail responsiveness above speculative architecture.
- Folders are the truth. SQLite remembers curation state and accelerates browsing.
- Stale-prone background flows need generation counters.
- Use platform widgets and libraries before custom code.
- Keep agent docs short enough to be read. Commit working code before writing more plans about it.


## October 2026: repository checkpoint

The documentation realignment was followed by separate verified commits for dependency fixes, displayed dimensions and safe copy services, scoped browsing/manual bucket persistence, Crop/WebP dialogs, and offline benchmark tools. Full starting work and trial-only wiring were preserved on local archive branches. All normal checkpoints build; default checks and isolated GTK workflows pass. No model runtime or trial placeholder was added to the normal branch.

Builds used an isolated target, leaving the user's existing launcher binary unchanged. Live Sharpr activity prevented claiming an idle backup or settings/WAL equivalence. Deployment, real-library release measurements, acceptance and `main` fast-forward remain Phase 0 steps requiring separate authorization. No branch/worktree was deleted and nothing was pushed.

## October 2026: product vision

The evening of October 12, a product-design interview with the user corrected the direction. Manual tagging is no longer a core feature to repair. Visual search plus typed filters should replace tags for finding images, and buckets remain the hand-curated way to keep them. The approved vision and decision log (D1–D19) are in `docs/reference/product.md`. The roadmap was restructured into Phases 0–9: new phases for file identity and buckets, cleanup, and Compare, and tags frozen until visual search passes its evaluation. No code changed.

## October 2026: Phase 0 complete

Phase 0 finished on October 12.
- **Checkpoint:** Sol committed the October 12 work in verified units. The unfinished visual-search trial went to `visual-search-trial-wip`, and the full snapshot to `preserve/october-12-wip`.
- **Backup:** Sharpr was closed and the data backed up and verified, to `~/.local/share/sharpr.backup-20261012-221640/`. The backup exposed real tag pruning; no bucket lost a member.
- **Deployment:** the launcher moved to per-commit release installs under `~/.local/opt/sharpr/` (first install `5832010`).
- **Acceptance:** the user tested the build ("everything works and feels fast"), and a release baseline was recorded. Startup took 498 ms, a 2,216-image folder opened in 116–162 ms, and memory peaked at 2.3 GB.
- **Branches:** `main` was fast-forwarded to `viewer-reset`. Six merged legacy branches, `docs-realignment`, and `viewer-base` were deleted along with their worktrees.
- **Findings carried into Phase 2:** the aspect popover's click-away, the idle gear, the overlap between the "…" and right-click menus, the 2.3 s pool wait, and thumbnail memory.

## October 12–13: Phases 1 and 2

The detailed per-item log lived in `CURRENT_TASKS.md` until October 13; the commits hold the rest.

- **Phase 1, data safety:**
  - Tags of missing or trashed files are kept and hidden, never deleted (81 paths had lost tags in real use on October 12).
  - Rename and Move can no longer overwrite a file.
  - Bucket edits keep parent and sibling membership (`src/buckets.rs`).
  - Thumbnail workers can no longer deadlock on their own queue; a stress test reproduces the old freeze.
  - Both databases carry `user_version` 1, and an older build refuses newer data without touching it.
  - 1,304 lines of compiled-out pipeline code were deleted.
  - Delete Tag confirms first; bucket names that differ only by case are rejected; single-key shortcuts are ignored while typing; trash failures are reported.
- **Phase 2, finding:**
  - A main-loop stall probe and `tools/bench-summary.py`.
  - Typed filter chips with suggestions; `below 4k` replaces the quality views.
  - Ctrl+B adds the selection to a bucket.
  - A backed-up, confirm-first cleanup of old automatic tags. A preview on the live data showed 7,346 rows to remove and 372 manual rows kept; it has not been run yet.
  - Main-thread work removed:
    - per-row tag queries
    - per-navigation duplicate grouping
    - synchronous bucket opening
    - linear virtual-view lookups
    - per-keystroke library walks
  - The 2.3 s folder-open wait is gone: hash writes are batched and folder scans run on their own thread.
  - Thumbnail memory was analysed but left as a decision for the user.
  - Item 10, one dimension source, was deferred for a correctness reason.
- **User decisions D20 and D21 (October 13):**
  - The Tags page and page switcher were removed.
  - Image actions moved to the right-click menu, which Shift+F10 also opens.
  - D18 was implemented: a quiet corner chip, plus an Info panel opened with Ctrl+I.
- **Phase 3 tooling:** blind judging sheet, scorer, parity, and provenance.
- **Deployment:** release build `66f3d94` was installed on October 13 for the user's acceptance test.
- **Independent audit:** a Codex `gpt-6.1-sol` (high effort) audit was attempted twice. The first run was stopped by low system memory; the second hit "model at capacity" after running the 157-test suite (all passed). It flagged only that `CURRENT_TASKS.md` contradicted itself about what was deployed, which was corrected.

## October 15: Phase 8 Compare accepted

Compare serves two or more selected images through Side by Side, Split and Flip, per-side thumbnail pickers, linked zoom/pan and reversible choices. Done returns with survivors selected; no files change. Find Copies uses the same view and returns dismissed candidates as checked extras, without touching the browsing selection or trashing until confirmation. Earlier focus and type-to-search faults were fixed. D28 (`c5699d7`) added contextual headers and Left/Right winner choices with matching Keep buttons and Ctrl+Z.

The `ef91d32` manual test covered the original controls and action guards. In the October 15 isolated trial of `fde5a52`, the user also accepted the new choices/Undo, modes, zoom/pan, survivor selection and search/chip restoration, closing the remaining feel check. Unit/GTK coverage includes five images to one, shared zoom geometry and original-file preservation. The final checks passed 211 regular tests and all 14 GTK tests. Committed on `viewer-reset`; the current interface is not deployed or merged into `main`. Candidate thumbnails and other Phase 9 proposals remain separate.

## October 15: Phases 4–7 and 9 accepted, merged into `main`

After testing the installed builds `679bbe1` and `2b5f070`, the user accepted:
- **Phase 4:** fingerprints and bucket membership.
- **Phase 5:** visual search and tag retirement. The manual browsing-during-indexing checks stand in for a measured comparison, and file-name matches stay first.
- **Phase 6:** WebP export and crop.
- **Phase 7:** Find Copies, which found three real copies the user had missed.
- **Phase 9:** the interface review, D26–D33.

Follow-ups went to the Backlog in `ROADMAP.md`: re-linking a moved bucket member before its new folder is opened, Find Copies tuning, and the Ctrl+Shift+S clash. Final checks: `./check.sh` (217 tests) and all 18 GTK workflows, run off screen. `main` was fast-forwarded to `viewer-reset` with the user's approval; nothing was pushed.

## October 15: 0.5.0 released

Phase 10 shipped Sharpr 0.5.0 (tag `v0.5.0`).
- **Choices (D34):** `Cargo.toml` is the one version source. "Open with Sharpr" was removed. The Flatpak bundles a Vulkan `llama-server` beside the processor build and may write the home folder.
- **Cleanup:** dead code (`FocusedImageSet`) and the unused rexiv2/gexiv2 dependency were removed.
- **Flatpak:** builds from `git archive` of HEAD. `build-flatpak.sh` bounds its memory, after an uncapped build crashed the computer: Flatpak escapes systemd memory caps.
- **Manual pass:** the user's pass found one viewer bug (fullscreen fit for images smaller than the screen), fixed with a GTK regression test.
- **Performance:** startup and memory improved on the Phase 0 baseline; big-folder opens stayed in about the same range.

Next: edit buckets (D35).
