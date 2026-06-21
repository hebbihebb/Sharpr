# External Edit Workflow And Image Families

Sharpr should support image families: a root original plus generated, edited, converted, or externally returned versions. Normal browsing shows one preferred version per family. Version review opens a focused Compare workspace with all versions available in a virtual filmstrip.

The workflow remains non-destructive. Sharpr never silently overwrites or deletes the old original. Promotion may move a better derivative into the original folder position, but the previous folder original is archived safely and lineage is preserved.

## Core Decisions

- Managed version storage lives under `~/Pictures/Sharpr Edits/`.
- Sharpr-managed storage is hidden from ordinary folder, smart, quality, duplicate, tag, and collection views.
- Normal browsing shows the family's preferred version only, with a version-count badge.
- `View Versions` opens a navigable virtual version set and enters Version Compare.
- External attachments copy files into managed storage; incoming files are left untouched.
- Attached versions do not become preferred automatically.
- Tags and collections should migrate toward id-backed keys before promotion work. Path-backed compatibility may remain during transition, but the promotion design must not depend on rewriting path-keyed curation state.
- Promotion moves the promoted file into the folder-original position only after the old folder original has been archived and verified.
- Version records support unavailable history states: `available`, `trashed`, and `missing`.

## Data And Storage Model

Add family/version persistence to the library index:

- `image_families`: family name, root original version, preferred version, current folder version, archived original version, manifest path, timestamps.
- `image_versions`: family id, path, role, relationship type, parent version id, source method, tool/provider, optional note, availability state, file metadata, timestamps.

Relationship types:

- `derived`
- `sibling_variant`
- `upscale`
- `converted`
- `compressed`
- `external_edit`
- `export`
- `promoted_folder_original`

Storage layout:

```text
~/Pictures/Sharpr Edits/
  Vestrahorn - 2026-05-20/
    manifest.json
    archived-original/
    working/
    external/
    exports/
```

The manifest is a recovery format, not the live source of truth. SQLite remains authoritative while the app runs, but manifests should be understandable from a file manager and restorable later.

## Attach Workflow

Entry points:

- Right-click `Attach Edited Version...`
- Drag/drop an image onto a thumbnail or viewer image
- File picker while an image/version is selected

All entry points route through one confirmation sheet with side-by-side preview, file details, relationship choice, optional tool/provider, and optional note.

On attach:

- Copy the incoming file into `external/`.
- Create or update the image family.
- Parent the new version to the selected version by default.
- Inherit tags and collection membership.
- Update SQLite and manifest.
- Make the result available in version history and Compare.
- Do not overwrite the original or change the preferred version.

## Promotion Workflow

Action: `Promote to Folder Original...`

The confirmation dialog must explain that the selected version will move into the original folder position and the old folder original will move to Sharpr Edits for safe keeping.

Transactional behavior:

1. Ensure family storage exists.
2. Copy or move old folder original into `archived-original/`.
3. Verify archived file exists and matches expected size/hash.
4. Move promoted version to the chosen folder-original path.
5. Update id-backed tags and collection membership.
6. Update family/version SQLite rows.
7. Write manifest.
8. Refresh filmstrip, viewer, thumbnails, and selected image state.

Failure rule: prefer leaving duplicate files present over losing data. Never remove or move the visible old original until archive verification succeeds.

## Compare Modes

Sharpr should keep one underlying left/right comparison viewer and expose two workflows:

- `Version Compare`: one image family, root original plus versions.
- `Standard Compare`: independent source/result pairs from Tasks or batch review.

The version filmstrip is a `FocusedImageSet`. It must use generation tokens, replace contents atomically, and avoid stale entries from previous folders or views.

Default Version Compare slots:

- Left: root original.
- Right: preferred version.

Slot assignment:

- Single-clicking a version thumbnail sets the right pane.
- Right-click menu supports `Set as Left` and `Set as Right`.
- Drag/drop onto Compare panes also sets left or right.
- Filmstrip overlays `L` and `R` badges on assigned thumbnails.

## Compare Toolbar

Replace the current compare-page info chip with a centered floating OSD toolbar:

- `gtk4::Box` with `.osd` and `.toolbar` classes inside the existing `gtk4::Overlay`.
- Wrapped in a `gtk4::Revealer` with slide-up transition.
- Centered horizontally above the bottom edge.
- Does not span full width, resize images, or push content.
- Visible on entering Compare; toggled by `H`, `Esc`, handle click, pointer movement, or tap.
- Contents: left slot label and metadata, separator, right slot label and metadata, Version Compare actions, info popover, overflow menu.

Do not include Sync Zoom, Fit, 1:1, Back to Tasks, or Remove from Compare in the toolbar. Those belong to filmstrip context menus or window navigation.

## Tasks Integration

Tasks remains a generation queue, not the primary version manager.

Output modes:

- `Create Version` default: output goes to managed storage and becomes a non-preferred version.
- `Export Copy`: output is a loose distributable file and does not attach to lineage.

Queue rows should store a stable `source_version_id`, not only a path. If the queued source version is missing or trashed when the runner starts, the task fails clearly.

Generated outputs inherit tags and collection membership. Multi-step tasks record each material output, for example original -> upscale PNG -> converted JXL.

Task history remains an operational log. `Clear History` removes task records only; it never removes family versions, manifests, or files.

## MVP Actions

Normal thumbnail/viewer:

- `View Versions`
- `Attach Edited Version...`
- `Add to Queue`
- `Compare With Original`
- `Set as Preferred Version`
- `Promote to Folder Original...`

Version filmstrip:

- `Set as Left`
- `Set as Right`
- `Set as Preferred Version`
- `Promote to Folder Original...`
- `Add This Version to Queue`

Tombstoned versions remain in history, show as dim unavailable rows, cannot be opened or compared, and are skipped by keyboard navigation into the viewer.
