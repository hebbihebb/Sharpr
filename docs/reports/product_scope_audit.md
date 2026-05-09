# Sharpr Product Scope Audit

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Report orientation: feature discipline for a viewer-first local image curation app

## Product Identity

Sharpr is for sorting images for quality review. Its strongest shape is: folders as the truth, a defining filmstrip, collections as central curation objects, Tasks as the home for background work and generated outputs, and compare/quality/duplicate workflows that help users decide what to keep. It should not become a full photo editor, a Lightroom clone, a file manager, or a generic automation host.

## Owner Decisions / Product Corrections

- Folders are the truth; SQLite exists for speed, stability, cache, task history, and curation state.
- Collections are central. Generated, upscaled, or exported files should probably inherit relevant tags/collections and may be auto-added to upscale, format, or output collections.
- Tasks are central for background work, queued work, generated outputs, and user decisions.
- The filmstrip is a defining feature, including compare/task-result virtual-folder behavior.
- No saved searches for now.
- Do not modify originals. Explicit trash is allowed; export/upscale/format conversion creates controlled outputs.
- No embedded metadata writing, no Sharpr tag export to IPTC/XMP, and no arbitrary scripts.
- No import workflow, no batch rename, and rotate/flip does not belong (feature is out of scope; code removal still pending).
- AI stays for now; user-configurable ComfyUI/API backends are feasible.
- Flathub is not a real target, but GNOME polish still matters.

## Core Features That Strengthen Sharpr

- Filmstrip reliability and fast thumbnail loading.
- Collections, collection inheritance, and output collections.
- Tasks as the central queue/history/decision surface.
- Generated-output tracking for exports, upscales, and format conversions.
- Better metadata display for quality review, without embedded metadata editing.
- Tag and collection workflows, including collection-inherited tags.
- Duplicate review with safe compare and curation actions.
- Quality scoring (resolution-based tiers) and quality filters.
- Export/downscale workflows with safe output paths.
- AI upscale as a task workflow, especially when local and optional.
- Compare views that populate the filmstrip from compare/task results.
- Keyboard-only navigation and accessibility.

## Useful Features That Should Wait

- Export metadata privacy preference and one-time popup.
- Freedesktop thumbnail cache support if it improves desktop integration without weakening Sharpr's cache model.
- Viewer-only or lite mode, later, if AI-heavy features need separation.
- Color management for accurate viewing if scoped to display correctness, not editing.
- Plugin-like backend boundaries for ComfyUI/API providers.
- Backup/export UX for Sharpr curation data only.

These are useful, but they should come after thumbnail reliability, keyboard accessibility, collection/task correctness, data migration discipline, and large-library stress testing.

## Deferred Or Out Of Scope

- Saved searches.
- Embedded metadata writing.
- Sidecar metadata systems, except user-controlled PNG output sidecars.
- Exporting Sharpr tags to IPTC/XMP.
- Import workflows.
- Batch rename.
- Arbitrary scripts/user-defined shell actions.
- Full image editing tools.
- Rotate/orientation editing (intentionally out of scope; code removal still pending).

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
- User-configurable API backends: feasible as explicit backends, not arbitrary scripts.
- Format conversion: task/export workflow only, with controlled outputs.
- PNG sidecars: user-controlled output option only.
- Model downloads: advanced/task flow with source, size, license, and storage location shown.

## Feature Evaluation

- Saved searches: out of scope for now.
- Metadata editing: no embedded metadata writing.
- Sidecar support: only optional user-controlled PNG sidecars for generated output.
- Batch rename: out of scope.
- Import workflow: out of scope; Sharpr works with folders and library hot swap.
- Format conversion: limited export workflow only.
- Color management: yes eventually for viewing correctness.
- Freedesktop thumbnail cache: worth exploring for desktop integration and cache hits.
- Scriptable actions: no arbitrary scripts.
- Full image editing tools: no for core app.
- ComfyUI/upscale integration: keep optional, local-first, and task-scoped.
- Viewer-only/lite mode: possible later, not now.
- Plugins/backends/advanced mode: useful later for upscale/API backends and export encoders, not scripts.

## Recommended 3-Phase Roadmap

Phase 1: Make Sharpr shippable.

- Security/privacy wording and ComfyUI consent.
- GNOME metadata polish, without treating Flathub as a release target.
- Keyboard-only accessibility pass focused on filmstrip, collections, Tasks, and compare.
- Thumbnail and rapid-folder-switching regression harness.
- Migration tests before new schema work.
- Generated-output tracking through Tasks and collections.

Phase 2: Strengthen curation.

- Collection/tag inheritance for generated outputs.
- Better metadata display for quality review.
- Duplicate review polish.
- Quality view polish and measurable thresholds.
- Backup/export for user-authored curation data.

Phase 3: Add controlled power features.

- Export metadata privacy preference and one-time popup.
- Format conversion as explicit export/task workflow.
- Color management.
- Freedesktop thumbnail cache integration.
- Advanced backends for AI/API integrations and export encoders.

## Positioning Statement

Sharpr is a fast, local-first image quality review tool for Linux. It treats folders as the truth, keeps originals untouched unless the user explicitly trashes them, and uses a GNOME-native filmstrip, collections, duplicate/quality review, compare tools, Tasks, safe generated outputs, and optional AI upscaling to help users sort large image folders without becoming a full photo editor.

## Practical Product Rule

If a feature helps users decide which local images are worth keeping, grouping, comparing, exporting, enhancing, or reviewing as generated outputs, it probably belongs. If it changes originals, writes embedded metadata, manages imports, renames batches, edits pixels directly, or runs arbitrary scripts, it should stay out.
