# Sharpr Product Scope Audit

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Report orientation: feature discipline for a viewer-first local image curation app

## Product Identity

Sharpr should stay a viewer-first, local-first, non-destructive image curation tool. Its strongest shape is: browse large local image libraries, compare images, tag and collect images, find duplicates, judge quality, export variants, and optionally run upscale workflows. It should not become a full photo editor, a Lightroom clone, a file manager, or a generic AI workflow frontend.

## Core Features That Strengthen Sharpr

- Saved searches as durable virtual views.
- Better metadata display and metadata-based filtering.
- Tag and collection workflows, including collection-inherited tags.
- Duplicate review with safe compare and curation actions.
- Quality scoring, sharpness backfill, and quality filters.
- Export/downscale workflows with safe output paths.
- AI upscale as a task workflow, especially when local and optional.
- Freedesktop thumbnail cache support if it improves desktop integration without weakening Sharpr's cache model.
- Viewer-only mode if it makes browsing faster and calmer without splitting the app identity.
- Color management for accurate viewing if scoped to display correctness, not editing.

## Useful Features That Should Wait

- Metadata editing.
- Sidecar support.
- Batch rename.
- Import workflow.
- Format conversion beyond export/task workflows.
- Scriptable actions.
- Advanced saved-search predicate builder.
- Plugin/backends system.
- Multi-library sync/export/import UX beyond basic backup.

These are useful, but they should come after release-readiness, keyboard accessibility, data migration discipline, and large-library stress testing.

## Features That Risk Bloating the App

- Full image editing tools: crop, brush, retouch, layers, filters, curves, masks.
- Lightroom-style raw development and catalog editing.
- General-purpose file manager operations.
- Print, slideshow, media playback, and broad multimedia support.
- Complex import-from-camera workflows.
- Full ComfyUI graph editor.
- Arbitrary shell script actions in the main UI.
- Cloud albums, sync accounts, sharing services, or social publishing.

These features create support burden and dilute the curation identity. If any are added, they should be explicit extensions or external handoffs, not core workflows.

## Advanced or Task-Scoped Features

- ComfyUI backend: keep behind advanced settings and make network/upload behavior clear.
- AI upscale: keep as a task with progress, output preview, and explicit save/discard.
- Scriptable actions: advanced-only, disabled by default, with clear trust warnings.
- Metadata writing/sidecars: advanced curation feature, not a default viewer operation.
- Batch rename and format conversion: task workflow with preview and undo-safe output, not inline file-manager behavior.
- Model downloads: advanced/task flow with source, size, license, and storage location shown.

## Feature Evaluation

- Saved searches: yes, near-term. They fit SQLite and curation.
- Metadata editing: later. Useful, but raises source-file and sidecar safety questions.
- Sidecar support: later with metadata editing. Do not rush.
- Batch rename: later, task workflow only.
- Import workflow: wait. Sharpr can open existing folders first.
- Format conversion: limited export workflow only.
- Color management: yes eventually for viewing correctness.
- Freedesktop thumbnail cache: worth exploring for desktop integration and cache hits.
- Scriptable actions: advanced-only, likely plugin/backend shaped.
- Full image editing tools: no for core app.
- ComfyUI/upscale integration: keep optional, local-first, and task-scoped.
- Viewer-only mode: yes if it improves focus and performance without hiding curation permanently.
- Plugins/backends/advanced mode: useful later for upscale, export encoders, scripts, and metadata providers.

## Recommended 3-Phase Roadmap

Phase 1: Make Sharpr shippable.

- Security/privacy wording and ComfyUI consent.
- Release/Flathub metadata cleanup.
- Keyboard-only accessibility pass.
- Large-library benchmark harness.
- Migration tests before new schema work.

Phase 2: Strengthen curation.

- Saved searches as first-class virtual views.
- Better metadata filtering.
- Duplicate review polish.
- Quality view polish and measurable thresholds.
- Backup/export/import for user-authored curation data.

Phase 3: Add controlled power features.

- Sidecar-aware metadata editing.
- Batch rename and conversion as explicit task workflows.
- Color management.
- Freedesktop thumbnail cache integration.
- Advanced backends/plugins for AI, scripts, and export encoders.

## Positioning Statement

Sharpr is a fast, local-first image library viewer for Linux that helps you curate large folders without modifying your originals. It combines a GNOME-native browsing experience with tags, collections, duplicate detection, quality scoring, compare tools, safe export workflows, and optional local AI upscaling for users who want deeper review without turning their image viewer into a full photo editor.

## Practical Product Rule

If a feature helps users decide which local images are worth keeping, grouping, comparing, exporting, or enhancing, it probably belongs. If it changes originals, manages the whole filesystem, edits pixels directly, or requires cloud-style workflows, it should wait, move behind advanced/task UI, or stay out.
