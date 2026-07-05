# Current Tasks

This file is the first handoff point after `AGENTS.md`. Keep it short and current.

## Current State

- `viewer-reset` removes Tasks, processing pipelines, upscale backends, local AI tagging, model downloads, and runtime networking.
- The retained product is folder/library browsing, the filmstrip and viewer, metadata, explicit trash, manual and automatic tags, collections, duplicate detection, quality views, generic two-image Compare, and transient operation status.
- Resize Copy creates oriented, metadata-free JPEG, PNG, WebP, or JPEG XL copies at explicit dimensions without overwriting files.
- Existing task tables and GSettings keys remain inert for branch-switch compatibility; fresh databases do not create task tables.

## Next Recommended Work

1. Split sidebar/filmstrip orchestration from `ui/window.rs` when the next viewer feature creates a natural boundary.

## Follow-Ups

- Break `library_index/mod.rs` into image, collection, and reconciliation services; schema initialization is already isolated.
- Remove the remaining compile-time legacy settings/index declarations after a deliberate compatibility window.
- Add a focused GTK integration test harness for Compare close/return and resize-dialog sensitivity.

## Handoff Checklist

- Keep originals immutable; only explicit trash may remove a source file.
- Protect visible-thumbnail priority and generation-based stale-result rejection.
- Before handing code work back, run `cd sharpr && cargo build`.
