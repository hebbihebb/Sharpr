# Sharpr Data Model and Migration Audit

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Report orientation: SQLite/library index growth, migration safety, and future saved searches

## Executive Summary

Sharpr's persistent data model is a major strength. The app separates original files from curation data, uses SQLite for library index state, has tags and quality in a local database, tracks ignored folders, and keeps pipeline state for long-running export/upscale workflows. This supports the product identity: non-destructive local curation.

The main model risks are schema evolution, duplicate source-of-truth boundaries, path identity assumptions, and future feature pressure. Saved searches can fit cleanly, but only if they are modeled as first-class query definitions instead of being encoded in UI state or overloaded collection rows.

## What Is Solid

- Original files are not the source of Sharpr curation state.
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
- Search exists as a `ViewScope`, but there is not yet a durable saved-search table.
- Cache-like data and user-authored data share databases. Quality scores, phash, and metadata status are recomputable; tags, collections, ignored folders, and saved searches are user data.

## Top 5 Migration and Data-Safety Improvements

1. Introduce explicit versioned migrations.

   Keep `schema_meta`, but move schema changes into ordered migration steps with tests that open older schemas and migrate to current.

2. Define source-of-truth categories.

   Document which data is user-authored, derived/recomputable, cache, task history, and preferences. Backup/export should prioritize user-authored data.

3. Add path identity tests.

   Cover symlinks, case-only rename behavior where possible, Unicode filenames, very long filenames, hidden folders, removable roots, and ignored-folder descendants.

4. Add backup/export/import for user data.

   A minimal JSON or SQLite export for tags, collections, ignored folders, and saved searches would reduce data-loss fear before larger schema work.

5. Add reconciliation tests for moves and disappearing files.

   Test file move, rename, mtime change, same path/new content, missing file, and reappearing file with previous cached metadata.

## Saved Searches Representation

Saved searches should be a first-class table, not overloaded as collections.

Recommended model:

- `saved_searches`
- Stable `id`
- `name`
- `query_json`
- `sort_order`
- `created_at`
- `updated_at`

The `query_json` should encode a small typed predicate model rather than raw SQL. Initial predicates can stay modest: text terms, tags, quality class, file extension, folder root, duplicate status, date/mtime ranges, and dimensions. The UI can compile this predicate into index/tag queries.

Behavior:

- Saved searches are virtual views. They do not own image membership.
- Collections remain explicit user grouping.
- Tags remain reusable labels.
- Search results should respect ignored folders.
- Query execution should use indexed columns where possible and fall back carefully for tag joins.

## Tests to Add Before Schema Changes

- Open an old schema fixture and migrate to current without data loss.
- Collection migration preserves name, tags, color, icon, parent, and item membership.
- Ignored folders exclude descendants in folder scan, smart views, tags, duplicates, quality, and saved searches.
- Tag database and library index stay coherent when a file disappears and later reappears.
- Saved search predicates serialize/deserialize and reject unknown predicate versions gracefully.
- Backup/export/import round-trips user-authored data.
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

Sharpr's data model is good enough for current curation workflows, but future growth should be migration-led. The next schema feature should add explicit migration tests first, then saved searches as virtual query definitions, while keeping original image files untouched and user-authored data easy to back up.
