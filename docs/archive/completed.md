# Completed Work Archive

This archive summarizes completed implementation work that used to live in `COMPLETED.html`. It is historical context, not the active task list.

## Major Completed Tracks

- Manual QA checklist and regression tests for thumbnail scheduling and stale generation handling.
- Private thumbnail cache documented and retained for O(1) validity checks.
- Compare controller extracted from `window.rs`.
- Compare page and viewer metadata OSD work.
- Collection dialogs extracted from `window.rs`.
- Action map audit for window/app actions.
- Generated-output lineage and Tasks routing groundwork.
- Tasks dashboard, queue runner fixes, flicker reduction, and batch editing.
- Privacy wording and UI affordances for ComfyUI/API backends.
- GNOME polish: naming, empty states, shortcut/manual consistency, accessible names.
- SQLite migration/reconciliation tests for collections and task history.
- Filmstrip and collection context menus migrated to `GMenu`/`GAction` patterns.
- Preferences/dialog/OSD HIG polish and writing-style audit.
- CSS theme-awareness audit, minimum window size documentation, and Rust version pin.

## Last Archived Item

Step 14f: Tasks queue batch selection and bulk edit.

- Queue rows support multi-selection.
- Batch controls apply selected setting edits across rows.
- The master plan was updated to move this item into the archive.

## Archive Policy

- Completed implementation notes belong here only when they are useful to future agents.
- Keep commit hashes in git history; do not turn this file into a full changelog.
- Active and upcoming work belongs in `CURRENT_TASKS.md` and `ROADMAP.md`.
