# Sharpr Data Model and Migration Audit

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Report orientation: SQLite/library index growth, migration safety, and curation state reliability

## Executive Summary

Sharpr's persistent data model is a major strength, but it is support infrastructure rather than the truth. Folders are the truth; SQLite exists for speed, stability, cache, task history, and curation state. The app separates original files from curation data, has tags and quality in local databases, tracks ignored folders, and keeps pipeline state for long-running export/upscale workflows.

The main model risks are schema evolution, duplicate source-of-truth boundaries, path identity assumptions, generated-output lineage, and collection/tag inheritance. Saved searches are out of scope for now and should not drive near-term schema design.

## Owner Decisions / Product Corrections

- Folders are the truth; SQLite supports performance, cache, stability, task history, and curation state.
- Collections are central and generated outputs should probably inherit relevant tags/collections.
- Tasks are central for background work, queued work, generated outputs, and user decisions.
- Compare already behaves like a virtual folder by populating the filmstrip from compare/task results.
- No saved searches for now.
- No embedded metadata writing and no Sharpr tag export to IPTC/XMP.
- Optional PNG sidecars are user-controlled output artifacts, not a general metadata sidecar system.

## What Is Solid

- Original files are not modified by Sharpr curation state.
- Folders remain the source of file truth.
- `LibraryIndex` stores folders, images, collections, collection items, pipelines, and pipeline steps.
- Images preserve metadata/phash/quality when file size and mtime are unchanged and invalidate cached fields when they change.
- Folder reconciliation is transaction-based and removes stale rows for missing paths.
- Ignored folders are represented in the index and checked before reconciliation/upsert.
- Pipeline recovery marks interrupted work and gives the app a way to handle incomplete tasks after restart.
- Tags and sharpness live in SQLite through `TagDatabase`.
- Useful indexes exist for folder, quality, phash, collection item path, collection item order, pipeline status, and pipeline steps.
- Tests already cover some migration and reconciliation behavior.

## Risky Schema and Model Assumptions

- Paths are primary identifiers. This is simple and practical, but moves/renames/removable drives become delete-and-add events unless future reconciliation adds stronger identity.
- Path matching uses string/path prefix behavior. Symlinks, case-insensitive filesystems, Unicode normalization, and bind mounts can produce surprising duplicates or ignored-folder leaks.
- `schema_meta` records a schema version, but migrations are mostly additive helper functions rather than a clearly versioned migration list.
- Tags and sharpness are in `tags.sqlite3`, while library index data is in `library-index.sqlite`. This separation is workable, but source-of-truth boundaries must stay explicit.
- Collections have evolved toward tag-backed behavior. That is useful, but it increases migration pressure around inherited tags, collection identity, and tag renames.
- Compare/task-generated virtual views need reliable identity and stale-result handling because they populate the filmstrip like folders.
- Cache-like data and user-authored data share databases. Quality scores, phash, and metadata status are recomputable; tags, collections, ignored folders, task decisions, and generated-output links are user data.

## Top 5 Migration and Data-Safety Improvements

1. Introduce explicit versioned migrations.

   Keep `schema_meta`, but move schema changes into ordered migration steps with tests that open older schemas and migrate to current.

2. Define source-of-truth categories.

   Document which data is user-authored, derived/recomputable, cache, task history, and preferences. Backup/export should prioritize user-authored data.

3. Add path identity tests.

   Cover symlinks, case-only rename behavior where possible, Unicode filenames, very long filenames, hidden folders, removable roots, and ignored-folder descendants.

4. Add backup/export for user data.

   A minimal JSON or SQLite export for tags, collections, ignored folders, task decisions, and generated-output links would reduce data-loss fear before larger schema work.

5. Add reconciliation tests for moves and disappearing files.

   Test file move, rename, mtime change, same path/new content, missing file, and reappearing file with previous cached metadata.

## Current Virtual View Representation

Saved searches are out of scope for now. Any future `LibraryView` or `ContentSource` layer should be justified as a reliability/refactoring aid for existing folders, collections, quality views, duplicates, compare, and task-generated virtual views, not as a path toward saved searches.

Recommended behavior:

- Folders load from filesystem truth, with SQLite as cache/index.
- Collections remain explicit user grouping and should be central in the model.
- Tasks own queued work, generated outputs, and user decisions.
- Compare/task result views can act like virtual folders by populating the filmstrip.
- Generated outputs should preserve traceability to source images and likely inherit relevant tags/collections.
- Collections remain explicit user grouping.
- Tags remain reusable labels.
- Ignored folders must be respected across all current views.

## Tests to Add Before Schema Changes

- Open an old schema fixture and migrate to current without data loss.
- Collection migration preserves name, tags, color, icon, parent, and item membership.
- Ignored folders exclude descendants in folder scan, smart views, tags, duplicates, quality, compare, and task-generated views.
- Tag database and library index stay coherent when a file disappears and later reappears.
- Generated outputs inherit or intentionally skip source tags/collections according to a tested rule.
- Task/compare virtual-folder results do not show stale outputs after folder switching or task completion.
- Backup/export round-trips user-authored curation data.
- Unicode and long paths survive insert, query, and display.
- Pipeline recovery handles queued, in-progress, completed, and failed states after restart.

## Files and Modules to Watch

- `sharpr/src/library_index/mod.rs`
- `sharpr/src/tags/db.rs`
- `sharpr/src/model/library.rs`
- `sharpr/src/config/settings.rs`
- `sharpr/src/ui/window.rs`
- `sharpr/src/duplicates/phash.rs`
- `sharpr/src/quality/*`
- `sharpr/src/upscale/*`

## Practical Position

Sharpr's data model is good enough for current curation workflows, but future growth should be migration-led and folder-truth-first. The next schema work should focus on generated-output tracking, collection/tag inheritance, task history reliability, and migration tests, while keeping original image files untouched and user-authored data easy to back up.
