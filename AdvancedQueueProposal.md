# Advanced Queue & Generation History Proposal

## Overview

The viewer panel becomes a **multi-page workspace** navigated by chevron controls embedded in the header bar title area. The current `View` label is replaced by a page navigator (`‹ View ›`) that cycles through four named pages. This consolidates features that would otherwise require separate popup windows, gives each view full panel space, and solves the existing lack of a return mechanism from the Tags view.

The background-operations button in the header bar (left of the hamburger menu) remains as a compact status indicator; it no longer needs to host a full queue UI since that lives in the Tasks page.

---

## Page Navigator

The center of the `AdwHeaderBar` currently holds a static `View` label button. This becomes a three-part navigator:

```
  ‹   View   ›
```

- **‹** and **›** are chevrons (`pan-start-symbolic` / `pan-end-symbolic`), matching the style GNOME uses for calendar month stepping and similar sequential navigation — thinner and more native-feeling than ← →
- Left chevron steps to the previous page (wraps), right chevron steps to the next
- **The center label is itself a button** — clicking it opens a small dropdown menu listing all four pages for direct navigation:
  ```
  View
  Tags
  Tasks
  Compare
  ```
- The viewer panel beneath is a `GtkStack` — pages switch with a slide transition in the direction of travel
- This pattern gives every page a self-evident way in and out, solving the current Tags view problem where there is no obvious return to View

---

## The Four Pages

### Page 1 — View
The existing image viewer. Default landing page when an image is selected. No change to current behaviour.

### Page 2 — Tags
The existing tag browser. Navigation arrows replace the need for any explicit back button. No change to current behaviour.

### Page 3 — Tasks
The advanced queue and history workspace. Replaces the previously proposed separate Advanced Queue window — full panel width gives it room to breathe.

**Layout (two-column):**
- Left column: queue list and history list (stacked, with a divider)
- Right column: per-job settings panel (operation type, model, format, quality, scale etc.) that updates when a queue row is selected

**Queue section:**
- Images added via filmstrip context menu ("Add to Queue")
- Each row: small thumbnail, filename, operation badge (Upscale / Export)
- Drag handle to reorder, × to remove
- Active job shows inline progress bar
- Start / Pause / Stop controls at the top

**History section (below queue):**
- Persistent log of completed jobs
- Each entry: thumbnail pair (source + output), operation type, model/settings, timestamp, status badge
- File-move resilience: stores content hash at write time; degrades gracefully to "source moved" or "output missing" badge if paths change
- **Compare** button on each history entry → navigates to the Compare page loaded with that entry
- Auto-clear at a configurable cap (default 500 entries); manual "Clear history" button

### Page 4 — Compare
A persistent before/after comparison view with its own independent filmstrip. **Replaces** the current in-viewer compare stack (the `BeforeAfterViewer` that lives in the viewer's internal `GtkStack` is retired).

**Key improvement over the current compare view:** the user can navigate away to View or Tasks and come back — the comparison session is preserved. Previously the compare view was lost the moment the upscale session ended.

**The compare filmstrip:**
- A virtual filmstrip, independent of the main library filmstrip, running along the bottom of the Compare page
- Each slot represents one source→output pair drawn from history
- Clicking a slot loads that pair into the `BeforeAfterViewer` above
- Populated **exclusively by explicit user action**: clicking Compare on a history row in the Tasks page appends that entry to the filmstrip and navigates the panel to the Compare page
- No automatic population — the user curates exactly what they want to review
- Items can be removed from the compare filmstrip individually without affecting history
- Session-only — no persistence needed. The filmstrip is a virtual folder populated on demand from history each session; it does not survive app restart and requires no database storage

**What this enables:**
- Review a batch of 20 upscaled images one by one without touching the main library view
- Load the same source image upscaled with three different models as three separate filmstrip slots — flip between them to evaluate quality differences
- The filmstrip gives at-a-glance context: all pending comparisons visible at once, current one highlighted

**Controls and options on this page (expandable over time):**
- Draggable split-line slider (left = original, right = output) — existing `BeforeAfterViewer` behaviour
- Model/settings info shown per slot (what model, what scale, what settings were used)
- Zoom controls synced between both sides
- Future: a grid mode showing multiple slots simultaneously (2×1, 2×2) for direct side-by-side evaluation

---

## Header Bar Button (Ops Indicator)

Remains as a compact icon button (gear icon, no pill). Its role is now purely a **live status glance** — the full queue UI lives in the Tasks page.

Behaviour:
- **No jobs, no history**: button hidden
- **Jobs queued or running**: button visible, spinner active, count badge
- **Jobs done, history exists**: button visible, gear icon (no spinner)
- **History cleared**: button hides

Clicking the button shows a minimal popover: active job name + progress, count of queued items, and a **"Go to Tasks"** action that navigates the panel to the Tasks page.

---

## What This Replaces / Consolidates

| Current feature | Fate |
|---|---|
| Static `View` label button | Replaced by ‹ Page › navigator |
| In-viewer `BeforeAfterViewer` compare stack | Retired — Compare is now a first-class page |
| Upscale dialog (viewer toolbar) | Settings moved into Tasks page per-job settings panel |
| Export dialog (viewer toolbar) | Settings moved into Tasks page per-job settings panel |
| Proposed separate Advanced Queue window | Not needed — Tasks page has full panel space |
| Proposed separate Compare window | Not needed — Compare page is persistent and navigable |
| Ops indicator pill (sidebar overlay) | Retired — replaced by header bar icon button |
| Tags view (no return mechanism) | Solved by page navigation arrows |

---

## Persistence & Job Lifecycle

Both the queue and history are persisted in the existing SQLite database (`library_index`), in a new `jobs` table. A job's `status` column drives which section of the Tasks page it appears in.

**Job lifecycle:**

```
Queued → In Progress → Completed ─→ History
                     └→ Failed    ─→ History (with error info, can be edited & retried)
```

| Status | Editable | Deletable | Location in UI |
|---|---|---|---|
| Queued | Yes | Yes | Queue section |
| In Progress | No | No | Queue section (locked row) |
| Completed | — | Yes (clears from history) | History section |
| Failed | Yes (edit & retry) | Yes | History section |

**Queue persistence:** queued jobs are just task descriptions (source path + operation + settings). They survive app restart and are loaded back into the queue on next launch in their original order. Only the `In Progress` job at the time of a crash is reset to `Queued` on restart, since its output may be incomplete.

**History persistence:**
- Completed and failed jobs stored permanently until manually cleared or auto-pruned at the cap
- Failed entries retain the full settings that were used, so the user can open them, adjust settings, and re-queue with one action
- Content hash stored at write time for graceful degradation if files move
- Cross-reference with existing pHash index to suggest relocated files within tracked libraries

**Schema:**

Two tables replace the earlier single `jobs` table concept:

`pipelines` — one row per image being processed:
- `id`, `source_path`, `source_hash`, `status`, `queue_order`, `created_at`

`pipeline_steps` — one row per step within a pipeline, ordered:
- `id`, `pipeline_id` (FK), `step_order`, `step_type`, `status`, `input_path`, `output_path`, `settings_json`, `created_at`, `started_at`, `finished_at`, `error_msg`

---

## Pipeline Model

A **pipeline** is one image moving through an ordered sequence of steps. Each step's output becomes the next step's input. Steps run sequentially within a pipeline; pipelines themselves also run sequentially in queue order.

**Building a pipeline:**
- Steps can be defined upfront before the pipeline starts running
- Steps can also be appended to a completed pipeline at any time ("Add follow-up") — the pipeline simply continues from where the last output left off
- This means a pipeline is never truly closed until the user stops adding to it

**Example pipeline:**
```
Source PNG
  └─ Step 1: Upscale (ComfyUI, SeedVR2, ×4)     → upscaled.png
  └─ Step 2: Export (JXL, quality 90, no resize)  → upscaled.jxl
```

**In the Tasks page UI:**
- The queue list shows one row per pipeline, with the source image thumbnail and filename
- Expanding a pipeline row reveals its steps as sub-rows, each with its own status and settings
- Each step has an icon badge for its type (upscale, export, API, etc.)
- Completed pipelines show in the history section with the same expandable structure
- A completed pipeline has an **"Add follow-up step"** button that appends a new step and re-queues the pipeline runner from that point
- **Progress display:** the collapsed pipeline row shows composite progress only — "Step 2 of 4: Exporting…" with a single progress bar. Per-step detail is visible only when the row is expanded. This keeps the queue list readable during long batch runs

**Step types (current and future):**

| Step type | Status | Description |
|---|---|---|
| Upscale | Current | Local upscale via CLI, ComfyUI, or ONNX backend |
| Export | Current | Format conversion — JXL, WebP, PNG, JPEG |
| External API | Future | Send image to a remote service, receive output back |

The `step_type` field in `pipeline_steps` is a string, keeping the schema open-ended. New step types can be added without a migration. A future external API step could, for example, send an image to Gemini image generation with a prompt, receive the result, and pass it to the next step — the pipeline runner treats it identically to any other step as long as it produces an output file.

**Step type enum in Rust:**
```rust
enum PipelineStep {
    Upscale { model: UpscaleModel, settings: UpscaleSettings },
    Export  { settings: ExportSettings },
    // Future:
    // ExternalApi { endpoint: String, prompt: String, settings: ApiSettings },
}
```

---

## File Visibility & Output Philosophy

The pipeline's job is not to automate decisions — it is to give the user **full visibility** over where every file is and what happened to it, so they can make their own decisions from one place.

**Intermediate files** (outputs of steps that feed into a subsequent step) are stored in a dedicated working directory: `~/.local/share/sharpr/pipeline_work/<pipeline_id>/step_N_filename.ext`. They are:
- Visible in the expanded pipeline row so the user always knows they exist and where
- Marked as temporary — not imported into the library automatically
- Cleaned up automatically when the pipeline completes successfully
- Kept on disk if a step fails, so the user can inspect what went wrong

**Completed pipeline display** in the Tasks page shows a full ledger for each pipeline:

```
📷 original.png           /home/user/Photos/trip/
   └─ ✓ Upscale (SeedVR2 ×4)
         output: original_4x.png    /home/user/Photos/trip/
   └─ ✓ Export (JXL 90%)
         output: original_4x.jxl   /home/user/Output/
```

Every file — original, intermediates, final output — is named and its path shown. The user sees the full picture at a glance.

**Output actions on a completed pipeline row — all explicit, nothing automatic:**

| Action | Description |
|---|---|
| Reveal in library | Jumps the main library view to that output file |
| Move to… | Relocates the output and updates the stored path |
| Replace original | Destructive — guarded with a confirmation dialog |
| Add to collection | User-initiated; user chooses which collection |
| Add tags | User-initiated; inherits from original or adds new |
| Compare | Sends the source→output pair to the Compare filmstrip |
| Add follow-up step | Appends a new step to continue processing the output |

Tags, collections, and folder placement are all offered as actions from the completed pipeline row — never done automatically in the background. The pipeline is the source of truth for what was done; the user decides what happens next.

---

## Open Questions

~~1. **Queue persistence across restarts**: should queued-but-not-started jobs survive a crash/restart? Session-only is simpler; SQLite persistence is more robust for large batches~~
**Resolved:** SQLite persistence implemented — queued jobs survive restarts and crashes.

~~2. **History cap default**: 500 seems right; expose in Preferences under a new "Queue & History" group~~
**Resolved:** Default 500, exposed in Preferences "Queue & History" group via `pipeline-history-cap` GSettings key.

~~3. **Crash recovery UX**: when the app restarts after a crash mid-job, the in-progress job is reset to Queued. Should the user be notified with a banner, or silently reset?~~
**Resolved:** `AdwBanner` at the top of the Tasks page — "N job(s) were interrupted and re-queued". Dismissable.

---

## Implementation Progress

*Branch: `conductor`. Last updated: 2026-05-05.*

| Phase | Status | Key commits |
|---|---|---|
| Phase 1 — Page Navigator | ✅ Done | `e666d6e` |
| Phase 2 — Ops Indicator Header Button | ✅ Done | `dd50837` + `1fab219` (pre-branch) |
| Phase 3 — SQLite Jobs Table | ✅ Done | `faf7d0e` |
| Phase 4 — Tasks Page: Single-Step Queue | ✅ Done | `3a68ce1` |
| Phase 5 — Tasks Page: History | ✅ Done | `23aa594` |
| Phase 6 — Compare Page | ✅ Done | `bd9040b` |
| Phase 7 — Settings Panel Completion & Viewer Dialog Retirement | ✅ Done | `ad77984`–`2aa2b5c` |
| Phase 4.5 — Multi-Step Pipelines & ComfyUI | ✅ Done | `08fb842`–`ad47270` (conductor branch) |

### Implementation notes

**Phase 1 (done):** `content_stack` in `ui/window.rs` is a `GtkStack` with four named pages: `"viewer"`, `"tags"`, `"tasks"`, `"compare"`. The header bar title area holds a `nav_box` (horizontal box) with `page_prev_btn` (pan-start-symbolic), `page_label_btn` (MenuButton), and `page_next_btn` (pan-end-symbolic). Page order is defined as `SharprWindow::PAGE_ORDER`. Chevrons set `SlideRight`/`SlideLeft` transition before switching. A `gio::SimpleAction` per page (`win.go-to-{name}`) powers the dropdown. The `connect_visible_child_notify` handler keeps the label in sync with any programmatic page changes. The dropdown arrow on the `MenuButton` was noted as something to suppress later (`set_always_show_arrow(false)`).

**Phase 2 (done):** `OpsIndicator` lives in the header bar (`header.pack_end`). It is always visible (permanently). The "Go to Tasks" link in the popover was wired in Phase 4. The conditional show/hide behaviour described in the proposal (hide when no jobs/no history) is deferred — the always-visible state is acceptable until Phase 5 provides history data to drive it.

**Phase 3 (done):** `pipelines` and `pipeline_steps` tables added to `library_index` SQLite DB via `initialize_schema`. All CRUD helpers implemented on `LibraryIndex`. Crash recovery (`recover_interrupted_pipelines`) runs at `open()` time. Three unit tests cover create/fetch, status transitions, and crash recovery.

**Phase 4 (done):** New file `ui/tasks_page.rs` (~732 lines). Two-column layout: queue list (left) + settings panel (right). `UpscaleStepSettings` and `ExportStepSettings` structs serialise to `settings_json` in DB. Runner uses a `glib::timeout_add_local` polling timer (2s interval) — jobs start within 2 seconds of being queued. CLI and ONNX backends supported; ComfyUI stubbed with an error. `make_upscale_backend` was extracted from `viewer.rs` into `upscale/backend.rs` as a shared public function. Output filename collision (same image queued twice) is a known minor issue deferred to Phase 7. Drag-to-reorder deferred to Phase 7. Pause deferred to Phase 7.

**Phase 5 (done):** History section added below queue in `tasks_page.rs` (~379 lines added, file now ~1,100 lines). History rows show dual thumbnails (source + output), filename, operation summary, relative timestamp, and status badge. Clear button wipes all history. Re-queue button appears on failed rows. Auto-prune runs after every job completion based on the configured cap. Four new `LibraryIndex` methods: `pipeline_history_count`, `prune_pipeline_history`, `clear_pipeline_history`, `requeue_pipeline`. `pipeline-history-cap` GSettings key added (default 500) with matching `AppSettings.pipeline_history_cap` field and Preferences "Queue & History" group. Compare button on history rows is a labelled but insensitive placeholder, activated in Phase 6. Known issue: jobs run with ComfyUI backend (the stub) fail immediately and land in history with "Failed: ComfyUI backend not yet supported in queue" — expected, deferred to Phase 7.

**Post-Phase-6 bug fixes (commits `addd8da`–`0bec036`):** Backend dropdown added to Tasks page upscale panel (was missing, causing all jobs to use CLI). CLI and ONNX both had "Zero width" PNG encode failure when scale=0 (Auto) — `requested_scale * dimensions = 0`; fixed in both `onnx.rs` and `runner.rs`. Output file extension now reflects actual saved format (PNG when uncompressed, format extension when compressed) so Compare page can decode the file. Failed/completed rows now always call `refresh()` before `run_next_pipeline()`, fixing stale InProgress rows staying in the queue. Export step type now wired: `current_step_config()` reads the op_dropdown; `StepType::Upscale` was hardcoded in window.rs. Adding a job to the queue no longer auto-starts the runner — user clicks Start Queue explicitly. CLI binary auto-detected via `UpscaleDetector::find_realesrgan` when not configured in Preferences.

**Phase 6 (done):** New file `ui/compare_page.rs` — `ComparePage` widget with `BeforeAfterViewer` (top, expands) and a horizontal virtual filmstrip (bottom, session-only, no DB persistence). `push_pair(source, output, label)` appends a slot to the filmstrip and auto-loads the pair. Slot × button removes the slot. Empty state shows a placeholder label. The "Compare" button on Tasks page history rows is wired via `set_compare_requested_cb` callback, which calls `compare_page.push_pair` then navigates `content_stack` to `"compare"`. The in-viewer `BeforeAfterViewer` (viewer's internal stack + Commit/Discard flow) is NOT touched — it remains for the existing upscale-from-viewer workflow. Retiring the in-viewer compare is deferred to Phase 7. Two bug fixes applied alongside Phase 6: ONNX backend crashed with "Zero width" when scale=0 (Auto mode) because `requested_scale * input_size = 0`; CLI backend in the queue failed because `upscaler_binary_path` was not auto-detected (viewer detected it lazily; queue now falls back to `UpscaleDetector::find_realesrgan`). Backend selector dropdown added to Tasks page settings panel — was missing, causing all queue jobs to be stored with the default "cli" backend regardless of preferences.

**Phase 7 (done, commits `ad77984`–`2aa2b5c` on `advanced-queue`, merged to `main`):** The Tasks page settings panel was rebuilt using Libadwaita components throughout — `AdwPreferencesGroup`, `AdwComboRow`, `AdwSwitchRow`, and linked `gtk4::ToggleButton` groups for the operation and backend switchers. Settings reorganised into three named groups: *Input/Output* (destination, upscale folder), *Upscale* (backend, model, scale, smart scale), and *Advanced* (compress, output format, quality, keep PNG). Labels renamed to match GNOME HIG conventions ("Output format", "Output quality", "Compress final image", "Smart scale", "Upscale backend"). ONNX model row wired — hidden unless ONNX backend active. Destination dropdown added for both upscale and export. The viewer's upscale/compare stack was retired (`ebb4cd9`): `BeforeAfterViewer` removed from `viewer.rs` (−1335 lines), viewer's internal `GtkStack` removed, "Convert" button now navigates to the Tasks page via `win.convert` action. Advanced preferences group ("Show AI Upscale" switch) removed from `preferences.rs`. Crash recovery `AdwBanner` implemented — appears in Tasks page on launch if any interrupted pipelines were found, auto-hides when Start is clicked or banner is dismissed. Queue row × delete button added (hidden for in-progress rows). Row selection now drives the right-hand settings panel: selecting a queued row loads its settings; selecting a history row shows the summary. Empty-state label added to queue section.

**Phase 4.5 — Multi-Step Pipelines & ComfyUI (done, `conductor` branch, commits `08fb842`–`ad47270`):**

*Step chaining in the runner* (`08fb842`): `set_step_input_path()` helper added to `LibraryIndex`. `run_next_pipeline` resolves `effective_source` — step 1 uses `pipeline.source_path`, subsequent steps use the previous step's `output_path`. Intermediate files are written to `~/.local/share/sharpr/pipeline_work/<pipeline_id>/`. Work dir is cleaned up on pipeline success, kept on failure for inspection. `run_upscale_step` and `run_export_step` both accept `effective_source: PathBuf` instead of always reading `pipeline.source_path`. Two new DB-level tests: `step_input_path_roundtrip`, `multi_step_chaining_db`.

*Expandable queue rows* (`850eaec`): `build_queue_row` replaced by `build_queue_expander_row` returning `libadwaita::ExpanderRow`. The expander shows a thumbnail prefix, filename title, composite status subtitle, and one `AdwActionRow` child per step with a coloured badge (●/○) and step status. `format_pipeline_composite()` formats the subtitle: "Queued · 2 steps", "Step 1 of 2: Upscale · Standard · Smart scale", etc. `expander.set_selectable(true)` and `set_widget_name(&pipeline.id.to_string())` preserve the existing selection-by-ID tracking logic.

*"Add step" button* (`be91b6a`): `suggested-action` button placed between `settings_stack` and `summary_group` in the right column. Hidden by default; shown via `load_settings_for_pipeline` when a queued (non-history) row is selected; hidden again in `clear_summary` and the history-row selection handler. On click, reads `current_step_config()` and calls `idx.append_pipeline_step()` then `refresh()`. A `RefCell` double-borrow panic in `refresh()` was found and fixed (`55f2131`): the `state` borrow in `refresh()` was scoped to a block so it is released before `select_row()` fires `row_selected` synchronously; `selected_pipeline_id` was also copied out of its `RefCell` before the block that calls `select_row()`, since `load_settings_for_pipeline` calls `borrow_mut()` on that same cell.

*ComfyUI activation* (`46b3039`): `comfyui_workflow: Option<String>` (serde default = None) added to `UpscaleStepSettings`. A "Workflow" `AdwComboRow` (ESRGAN / SeedVR2) added to the upscale group, visible only when ComfyUI backend is active. The 4-line early-return guard ("ComfyUI backend not yet supported in queue") removed from `run_upscale_step`. Per-step workflow resolution: `settings.comfyui_workflow.as_deref().unwrap_or(&comfyui_workflow_global)`.

*"Follow-up step" button* (`ad47270`): `enqueue_followup(source, step_type, settings_json)` added to `LibraryIndex` (composes `create_pipeline` + `append_pipeline_step`). A "Follow-up step" flat button added to history rows; it resolves the highest `step_order` step with an output path that exists on disk as the follow-up source, and is greyed out if none exists. Available on both Completed and Failed history rows. New test: `enqueue_followup_creates_queued_pipeline`.

---

## Suggested Implementation Phases

Each phase is independently shippable and testable. Later phases depend on earlier ones but earlier phases deliver real value on their own.

### Phase 1 — Page Navigator
*Foundational. Everything else depends on this.*

- Replace the static `View` label button in the header bar with a three-part navigator: left chevron (`pan-start-symbolic`), center label button, right chevron (`pan-end-symbolic`)
- Convert the viewer panel's internal `GtkStack` into the page stack, adding named pages: `view`, `tags`, `tasks`, `compare`
- Center label is a `gtk4::MenuButton` with a dropdown model listing all four pages for direct navigation
- Wire the existing image viewer to `view` and the existing tag browser to `tags` — both work immediately with no other changes
- The `tasks` and `compare` pages can be placeholder widgets at this stage
- Result: navigation between View and Tags works; the return-from-Tags problem is solved

### Phase 2 — Ops Indicator Header Button
*Independent of Phase 1. Can be done in parallel or before.*

- Move the `OpsIndicator` from the sidebar overlay into the header bar as a compact icon button (gear icon, no pill), to the left of the hamburger menu
- Change popover position from Right to Bottom
- Minimal popover: active job name + progress, queued count, "Go to Tasks" link (Tasks page is a placeholder at this stage — the link can be wired in Phase 4)
- Retire the sidebar overlay pill and its CSS

### Phase 3 — SQLite Jobs Table
*Required before Phase 4. Low UI surface, high value.*

- Add `pipelines` and `pipeline_steps` tables to `library_index` SQLite schema (see Pipeline Model section for columns)
- Implement CRUD helpers in `library_index`: create pipeline, append step, update step status, reorder pipelines, delete, fetch by status
- On startup: reset any `in_progress` steps back to `queued`, reset their parent pipeline status accordingly (crash recovery)
- No UI yet — just the data layer

### Phase 4 — Tasks Page: Single-Step Queue
*Depends on Phases 1 and 3.*

One-step pipelines only (source → upscale, or source → export). The data model is already pipeline-shaped from Phase 3, but the UI does not expose step chaining yet.

- Build the Tasks page UI: two-column layout with queue list on the left, settings panel on the right
- Filmstrip context menu: right-click one or more selected images → "Add to Queue" → row appears in queue list with thumbnail, filename, default operation (Upscale)
- Settings panel: operation selector (Upscale / Export) and all relevant settings for the chosen operation, drawn from the existing upscale and export dialogs
- Each pipeline row in the queue shows a single step inline — no expand/collapse needed yet
- Drag-to-reorder rows, × to delete (queued pipelines only)
- Sequential runner: pulls next `queued` pipeline, runs its single step, marks complete or failed
- Start / Pause / Stop controls
- Wire the "Go to Tasks" link in the ops indicator popover
- Result: a functional batch queue for single operations, replacing the one-at-a-time upscale/export flow

### Phase 4.5 — Multi-Step Pipelines ✅ Done (`conductor` branch)
*Depends on Phase 4. Implemented after Phase 7.*

- Pipeline rows are expandable `AdwExpanderRow`s: collapsed shows composite progress ("Step 2 of 3: Exporting…"), expanded shows each step as a sub-row with coloured status badge
- "Add step" button in the settings panel appends a new step to the selected queued pipeline using the current panel settings
- "Follow-up step" button on history rows creates a new pipeline using the previous job's output as source
- ComfyUI backend enabled in the queue runner (was blocked by an early-return guard); per-step `comfyui_workflow` stored in `UpscaleStepSettings`
- Intermediate file management: working files written to `~/.local/share/sharpr/pipeline_work/<pipeline_id>/`, cleaned up on success, kept on failure

### Phase 5 — Tasks Page: History
*Depends on Phase 4.*

- Add the History section below the queue list in the Tasks page
- Completed and failed jobs displayed as rows: thumbnail pair, operation type, model/settings, timestamp, status badge
- File-move graceful degradation: check paths on display, show "source moved" / "output missing" badges where appropriate
- Failed rows: editable settings, re-queue action
- Auto-clear at configurable cap (default 500); manual "Clear history" button
- Preferences: add "Queue & History" group with history cap spin row
- Compare button on each history row (navigates to Compare page — placeholder until Phase 6)

### Phase 6 — Compare Page
*Depends on Phases 1 and 5.*

- Move `BeforeAfterViewer` out of the viewer's internal stack and into the `compare` page of the main page navigator
- Add the virtual filmstrip at the bottom of the Compare page: a session-only `GListModel` of source→output pairs, no persistence needed
- Clicking Compare on a history row appends the entry to the compare filmstrip and navigates to the Compare page
- Clicking a filmstrip slot loads that pair into the `BeforeAfterViewer`
- Model/settings info shown per slot
- Remove the now-retired in-viewer compare stack and its toolbar controls

### Phase 7 — Settings Panel Completion & Viewer Dialog Retirement ✅ Done
*Depends on all prior phases. Branch: `advanced-queue`, merged to `main`.*

The Tasks page settings panel was rebuilt using Libadwaita components and completed so the viewer's upscale/compare dialogs could be retired.

---

#### Part A — Complete the Upscale settings panel ✅ Done

- ONNX Model `AdwComboRow` — visible only when ONNX backend selected; hidden for CLI/ComfyUI
- Keep raw PNG sidecar `AdwSwitchRow` — maps to `UpscaleStepSettings.keep_png`
- Destination `AdwComboRow` — "Upscaled folder" / "Same as source"
- Settings grouped into three `AdwPreferencesGroup`s: *Input/Output*, *Upscale*, *Advanced*
- Labels renamed to match HIG conventions ("Output format", "Output quality", etc.)
- "Show AI Upscale" switch removed from Preferences

#### Part B — Complete the Export settings panel ✅ Done

- Destination `AdwComboRow` — "Export folder" / "Same as source"
- Quality spin row, format row, max-edge row

#### Part C — Crash recovery banner ✅ Done

`AdwBanner` at the top of the Tasks page. `recover_interrupted_pipelines` returns `usize`; count propagates through `LibraryIndex::open` → `window.rs` → `tasks_page.set_interrupted_count(n)`. Banner dismissed on Start or ×.

#### Part D — Queue row × button ✅ Done

× button on each queue row (`window-close-symbolic`), hidden for InProgress rows. Calls `idx.delete_pipeline(pid)` then `refresh()`.

#### Part E — Retire the viewer's upscale/compare dialogs ✅ Done (`ebb4cd9`)

`BeforeAfterViewer`, `ConvertKind`, `ConvertDestinationMode`, all comparison toolbar logic removed from `viewer.rs` (−1335 lines). Viewer's internal `GtkStack` removed. "Convert" button navigates to Tasks page via `win.convert` action. "Show AI Upscale" preferences group removed from `preferences.rs` (−215 lines).

#### Part F — Minor polish ✅ Done

- Dropdown arrow suppressed on page navigator `MenuButton` (`set_always_show_arrow(false)`) — `66ed19e`
- Empty-state label added to queue section
- Output filename uniquification already handled by existing `unique_output_path` helper
- Row selection drives settings panel — selecting a queued row loads its settings; deselecting clears the panel
