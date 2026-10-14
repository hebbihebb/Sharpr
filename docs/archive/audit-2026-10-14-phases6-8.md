# Sharpr adversarial audit: Phases 6–8

Review of `3a695eb..b5cb8ca`, October 14, 2026. Archived evidence, not instructions. One static review pass; no builds, tests, app launches, private images, or live data accessed. The reported 202 unit and 8 GTK passes were not independently rerun. Locations refer to this worktree.

## 1. Summary

Do not accept cleanup or mode isolation yet: confirmed paths can trash a different file from the one reviewed, and Delete remains active during cropping without confirmation. Copy classification can admit variants, while its work and preview decoding lack practical concurrency/cancellation bounds. Ordinary export collisions are handled, but output publication still has a concurrent-replacement hole. Before the next user test, fix destructive-action identity/mode guards, copy grouping and resource limits, Compare focus, and the two incompletely closed Phase 5 findings below; make negative-filter limitations visible.

## 2. The three user reports

1. **Find Copies spins forever — best hypothesis, medium confidence.** `copies.rs:146` does all pairs: 5,200 vectors require 13,517,400 comparisons, approximately 10.38 billion scalar multiplications. Borderline pairs then repeatedly prepare/decode both images serially (`:95`, `:157`), without caching their small pixels. `ui/copies_dialog.rs:109` supplies a permanently false cancellation callback and reports nothing until completion. Large JXLs and many similar versions can make this exceptionally slow; no deadlock or literal infinite loop was established. Worker failure also leaves the spinner unchanged (`:115`). See findings 5–6.
2. **Compare only works for two — best hypothesis, medium confidence.** There is no two-image limit: `ui/window.rs:2583` passes every selected path to `Knockout`. However, `ui/window/compare_controller.rs:18` starts the page before showing it (`:22`), and `ui/compare_page.rs:51` attempts focus while it is hidden. Keys are handled only within that page (`:463`); focus elsewhere can send brackets to type-to-search instead. The × buttons or bracket keys are the only way to advance candidates; there is no Next control. Actual pointer failure is unproven. The GTK test calls `page.dismiss()` directly (`ui/window/gtk_tests.rs:1261`), so passing it does not establish keyboard usability.
3. **`tower -water` seems ineffective — no position-dependent parser defect found, high confidence in that conclusion; medium confidence in explanation.** `model/search_query.rs:325` recognizes minus words anywhere; `ui/window/search_controller.rs:584` applies them even before a trailing space. A visible chip appears only after the word is completed with whitespace (`model/search_query.rs:369`). Semantic exclusion removes a fixed top tenth of stored fingerprints, not every image depicting water (`visual_search/search.rs:61`); water below that cutoff remains. Negative embedding failures are silently ignored (`ui/window/search_controller.rs:704`). These can produce the observation; its exact cause needs a later isolated UI reproduction.

## 3. Ranked findings

### 1. P0 — Reviewed paths can trash replacement files

**Location:** `ui/copies_dialog.rs:19`, `:404`; `ui/window.rs:681` (under `sharpr/src/`). **Confidence: high. Fix now.**

The review retains paths/byte counts, not fingerprints or file identities. After results appear, move an extra elsewhere and put an unrelated image at its old path; confirm trash. The worker trashes the replacement without checking it. Removing the proposed keeper during review also permits trashing the last surviving copy without the whole-group warning. Carry reviewed identities and keeper expectations through confirmation and revalidate immediately before trash. The GTK test bypasses the confirmation UI (`ui/window/gtk_tests.rs:1175`), covering neither race.

### 2. P0 — Delete while cropping trashes the browsing selection

**Location:** `ui/crop_mode.rs:439`; `ui/window.rs:3294`, `:120`. **Confidence: high. Fix now.**

Crop consumes only Escape/Enter. Delete checks text focus and Compare, then directly trashes the filmstrip selection without confirmation. Select A, right-click B → Crop Copy, then press Delete: A can disappear while B's crop is visible. F2 and Ctrl+D also lack crop guards. Disable browsing file actions throughout crop preparation/session and require destructive confirmation.

### 3. P0 — Output reservation does not survive concurrent path replacement

**Location:** `export/mod.rs:411`, `:211`, `:322`; `export/batch.rs:134`, `:147`. **Confidence: high for the conditional race. Fix now.**

Reservation creates files exclusively, closes their handles, and subsequently writes/renames/removes by pathname. If an external process replaces the reserved output before publication, ordinary rename overwrites that new file; cancellation deletes it. Replacing the partial with a symlink before `File::create` can truncate its target. This pattern predates Phase 6 but now serves batch export too. Keep ownership of the temporary file, publish without replacement, and never clean up an unverified replacement. Existing collision tests do not inject replacement after reservation.

### 4. P1 — “Same Picture” is not a safe equivalence relation

**Location:** `copies.rs:165`, `:174`; `sharpr/data/manual.md:47`. **Confidence: high for algorithmic failure; real-library frequency unknown. Fix now.**

Cosine ≥0.99 bypasses pixels entirely: same-aspect crops, mirrors, or edited variants can pass. Union-find also allows A–B and B–C matches to merge A and C despite their failing direct comparison. For a concrete vector construction, unit vectors at angles 0°, 8°, 16° have adjacent cosine ≈0.99027 and endpoint cosine ≈0.95992; equal aspects merge all three even when endpoint pixels differ. Default trash ticks then include unverified variants. Validate every extra against the keeper with stronger spatial/colour evidence; add high-score variant and chain tests. The real-model fixture's final assertions exclude its crop but do not assert exclusion of the mirrored file (`copies.rs:504`). The manual's categorical guarantee is unsupported.

### 5. P1 — Copy search has no usable work bound or cancellation

**Location:** `copies.rs:146`, `:157`, `:246`; `ui/copies_dialog.rs:109`. **Confidence: high. Fix now.**

All qualifying pairs are materialized before serial file checks. Every borderline edge decodes both endpoints again; cancellation is absent within grouping and disabled by the caller. Closing/reopening launches more complete searches. Near-identical clusters can generate quadratic pair storage and repeated expensive JXL decoding. Cache each image's comparison pixels once, bound pair/work storage and parallelism, add cancellation/progress, and test synthetic library-scale input before another real-library cleanup test.

### 6. P1 — New decoding paths can exhaust memory and stall GTK

**Location:** `ui/copies_dialog.rs:309`, `:334`; `ui/compare_page.rs:138`, `:178`, `:232`; `export/mod.rs:365`. **Confidence: high for missing bounds; OOM/stall magnitude unmeasured. Fix now.**

Copy results eagerly build every row and spawn one OS thread per file. With cold thumbnail caches, thousands of decodes can overlap; each allows gigabytes. Compare starts a fresh full decode on every changed side; rapid knockouts/undo leave old workers running, and full textures reach GTK even at Fit. Its synchronous `summary()` reads headers/EXIF on GTK, including embedded JPEG-preview decoding through `metadata/orientation.rs:128`. Export/crop JXL decoding bypasses the budget added to visual-search preparation. Reuse bounded workers and screen previews, cap JXL decode size, and load metadata off-thread. Small existing fixtures cover none of these resource limits.

### 7. P1 — Compare can escape its page without ending its session

**Location:** `ui/window.rs:3278`, `:2810`, `:3319`, `:3433`; `ui/window/compare_controller.rs:18`. **Confidence: high for unguarded routes; focus outcome medium. Fix now.**

Ctrl+F and type-to-search show the viewer without calling Compare close or restoring scope/survivors. The filmstrip/sidebar remain available, and Shift+F10/context file actions bypass the three guarded shortcuts; Move/Trash can still act during Compare. Reproduce with N selected → Compare → Ctrl+F, or filmstrip right-click → Move to Trash. Focus the canvas after it becomes visible and route every mode exit/action through one lifecycle guard. Test actual key events and menus, not only direct page methods.

### 8. P2 — Previous finding 9 is only partially fixed

**Location:** `visual_search/index.rs:144`, `:184`, `:293`; `visual_search/store.rs:124`. **Confidence: high. Fix now.**

GPU fallback is repaired, but two CPU timeouts/live HTTP 500 responses still become `ImageFailed`, persisted permanently and skipped by subsequent passes. A healthy PNG plus two infrastructure errors suffices. The new test explicitly expects this misclassification (`visual_search/index.rs:770`). Classify infrastructure errors separately, stop/resume without blaming content, and test persistence as well as recovery decisions.

### 9. P2 — Previous finding 10 still permits a stale query backlog

**Location:** `visual_search/search.rs:179`, `:187`, `:209`, `:346`. **Confidence: high. Fix now.**

Caller cancellation releases the browse worker, but requests already sent to the unbounded search queue remain executable. While one server request stalls, edit the query repeatedly: later searches wait behind obsolete work, which carries no cancellation token. The 20-second limit is per embedding, not whole preparation; ten generic prompts and startup can extend it further. Coalesce/discard obsolete requests on the owning thread and bound complete preparation. Existing tests cover caller cancellation and individual HTTP deadlines, not this queue.

### 10. P2 — Late crop preparation opens an obsolete session

**Location:** `ui/crop_mode.rs:85`, `:103`. **Confidence: high. Fix now.**

Crop preparation checks only whether the window exists. Start a slow preview for A, navigate away or start B's crop; A's late completion opens another overlay, overwrites `crop_session`, and captures already-hidden panel state. Save can then target A unexpectedly. Add a crop request generation, invalidate it on navigation/cancellation/new requests, and reject changed source stamps. Cover reordered completions in an isolated test.

### 11. P2 — A one-image result selection is ignored

**Location:** `ui/window/search_controller.rs:498`, `:517`; `ui/filmstrip.rs:2039`. **Confidence: high. Fix now.**

Select a range, Ctrl-remove all but one, then Add to Bucket. The selection path requires more than one, so it offers all/strong results instead of the chosen singleton. Keyboard export likewise falls back to the browsing current image. Honor nonempty explicit selection, including one item, and distinguish it from navigation focus. Add a range-to-singleton action test.

### 12. P3 — Overview and checkpoint wording overstate reality

**Location:** `ui/library_overview.rs:234`, `:255`, `:340`; `CURRENT_TASKS.md:13`; `ROADMAP.md:331`. **Confidence: high. Disposition: Phase 9/docs checkpoint.**

Only visual status refreshes; fingerprint/copy/bucket/index-file figures are one snapshot. Gathering errors leave “…” indefinitely and are logged only. Unsupported formats still appear collectively as “couldn't be read” (`ui/library_overview.rs:271`), although their stored reasons improved. Current Tasks and Roadmap retain “not deployed” statements contradicted by the installed `baa007d`. Surface errors/unsupported coverage, label snapshot figures or refresh them, and reconcile deployment wording.

## 4. What checked out

Previous-audit closure, by original finding number:

- **1–3:** indexing keeps/rechecks source stamps (`visual_search/index.rs:261`); search revalidates model stamps (`search.rs:280`); both server/device discovery force offline and strip inherited runtime settings (`runtime.rs:74`, `:184`). Reviewed fixes close the reported ordinary paths.
- **4–6:** tag offers/conversion share eligibility (`buckets.rs:493`); bucket/member creation is transactional (`library_index/identity.rs:241`); folder toggles requeue both services (`ui/window.rs:1964`). Legacy tag rows remain intact.
- **7–8, 14–16:** paging uses a ranked snapshot and preserves selection/scroll; new queries reload fingerprints; coverage includes names; clearing search hides status; actual positive-query errors reach the status line (`ui/window/search_controller.rs:609`, `:749`, `:839`, `:878`). Negative-query errors remain a separate gap described above.
- **9–10:** partial closure only, findings 8–9 here. **11:** visual-search JXL header guard added (`visual_search/prepare.rs:81`); new export/Compare paths reopen the memory issue. **12:** vector-file device/inode detection and reindex request added (`visual_search/search.rs:308`). **13:** decoder support explicitly deferred to Backlog; stored unsupported reasons improved, UI distinction incomplete. **17:** deployment contradictions persist, finding 12 here.

Crop/batch normally read originals, apply orientation, create unique outputs in the selected destination, check writes, and clean up controlled failures/cancellation. Lossless/collision/orientation tests exist. No new migration or deletion of tags/bucket members was found. Compare completion checks generation plus path; its knockout model accepts N and retains survivors. Filmstrip selection extends the existing scheduler; known missing end-to-end thumbnail generations remain explicitly in Backlog. Passing small tests does not substitute for the missing destructive-race, real-keyboard, and library-scale checks identified above.
