# gThumb vs Sharpr Deep Research Report

Date: 2026-05-09  
gThumb checkout: `/home/hebbi/Projects/gthumb`, branch `master`, commit `1b7dc65db`  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`, branch `main`, commit `2de12d9`  
Report orientation: practical lessons and roadmap for Sharpr

## Executive Summary

The current gThumb repository is not the older GTK3/C codebase many people still associate with gThumb. The cloned `master` branch is a `4.0.alpha` GTK4/libadwaita rewrite using Vala for application/UI logic plus C/C++ for image primitives, codecs, metadata, color management, and low-level helpers. This makes it a much more relevant comparison for Sharpr than expected: both projects target modern GNOME, both use GTK4-era UI concepts, both are local-first image applications, and both separate main-thread UI from background decoding work.

The largest architectural difference is product scope. gThumb is a complete photo browser/editor/organizer with a mature feature surface: file manager operations, catalogs, selections, metadata sidecars and embedded metadata, import, search, print, scripts, slideshow, editor tools, saver preferences, video/audio viewing, color management, and format-specific loaders/savers. Sharpr is narrower but more specialized: it has a persistent SQLite library index, tags, collections, duplicate detection via perceptual hashes, quality scoring, benchmark logging, a Rust-heavy thumbnail pipeline, Glycin-based viewing, and AI upscale/ComfyUI workflows.

The most important Sharpr takeaway is not “copy gThumb.” gThumb’s value is in its capability map and extension-shaped organization. After owner product decisions, Sharpr should borrow only the patterns that strengthen folder-based quality review: filmstrip reliability, collections, Tasks, generated-output handling, progress visibility, format/export preferences, and GNOME polish. Saved searches, import workflows, batch rename, embedded metadata writing, and arbitrary scripts are out of scope for now.

Sharpr is already ahead in several areas that matter for a modern curation tool: it has substantially more unit coverage, a persistent indexed data model, explicit benchmark instrumentation, modern duplicate/quality workflows, and stronger local-first non-destructive organization. gThumb is ahead in breadth, metadata maturity, color management, editing tools, file operations, save pipeline polish, UI template coverage, translations, and long-lived GNOME product completeness.

## Repository Shape

### gThumb

The cloned gThumb tree is a Meson project with Vala, C, and C++:

- Top-level project: `meson.build`, `meson.options`, `.gitlab-ci.yml`, Flatpak manifests, appdata, schemas, desktop files, translations.
- Main app code: `src/*.vala`.
- Extension-style feature groups: `src/Ext/*`.
- Dialogs: `src/Dialogs/*`.
- Low-level image library: `src/lib/*`.
- GTK templates: `data/ui/*.ui`.
- Settings schemas: `data/schemas/*.xml`.
- Tests: `src/Tests/*`, enabled only with `-Ddeveloper-mode=true`.

Source inventory from this checkout:

- Vala files: 246
- C files: 41
- C++ files: 2
- Header files: 37
- GTK UI templates: 95
- XML files counted near source/data: 4 in the simple inventory command, with additional schema/appdata/resource XML under `data`
- C/C++ line count: about 15,183 lines

That line count undercounts total gThumb behavior because Vala files are the majority of the application and were counted separately from the C/C++ total. The project is feature-rich even in this alpha branch.

The README describes gThumb as an image viewer, editor, browser, and organizer. Supported image/media areas include PNG, JPEG, WebP, SVG, JXL, HEIF/AVIF, TIFF, GIF, RAW, EXIF/IPTC/XMP, color profiles through lcms2/colord, and audio/video through GStreamer.

### Sharpr

Sharpr is a Rust project under `/home/hebbi/Projects/Sharpr/sharpr`:

- UI: `src/ui/*`, mostly code-built GTK4/libadwaita widgets.
- Model: `src/model/*`.
- Persistent index: `src/library_index/mod.rs`.
- Thumbnail workers/cache: `src/thumbnails/*`.
- Image pipeline metadata worker: `src/image_pipeline/*`.
- Tags and smart tagging: `src/tags/*`.
- Duplicate detection: `src/duplicates/*`.
- Quality scoring: `src/quality/*`.
- Export and upscale workflows: `src/export/*`, `src/upscale/*`.
- Settings/resources/packaging: `data/*`, `packaging/*`, `build.rs`.

Source inventory:

- Rust source line count: about 30,048 lines
- Rust source files: 62 under `src` from the earlier map, with additional nested files such as `src/upscale/backends/comfyui.rs`
- Tests found across many modules; `cargo test` ran 138 tests successfully

Sharpr’s current README positions it as a high-performance local-first image curation tool and viewer, with chunked loading, background thumbnail caching, Glycin decoding, SQLite-backed organization, perceptual duplicate detection, quality scoring, and AI enhancement.

## Build And Test Findings

### gThumb

Attempted command:

```bash
meson setup /home/hebbi/Projects/gthumb/builddir /home/hebbi/Projects/gthumb -Ddeveloper-mode=true
```

Result:

```text
/bin/bash: line 1: meson: command not found
```

This is an environment/tooling blocker, not evidence of source failure. The project requires Meson and the GNOME/native development dependencies listed in `README.md` and `meson.build`, including recent GLib, GTK 4.18.5+, libadwaita 1.8+, exiv2, lcms2, GStreamer 1.26+, libpng, zlib, libjpeg, and optional format libraries.

gThumb’s tests are defined in `src/meson.build` only under `if get_option('developer-mode')`. The test targets are:

- `dom`: XML DOM serialization/loading.
- `util`: filename/path/URI helper behavior.
- `strings`: string helpers including Unicode-containing names.
- `template`: token/template parser behavior, including Unicode token handling.

This is useful but narrow. It mostly covers utility logic rather than browser, viewer, loader, metadata, save, catalog, or long-running job behavior.

### Sharpr

Commands run:

```bash
cargo build
cargo test
```

Results:

- `cargo build`: passed.
- `cargo test`: passed, `138 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.18s`.

The passing tests cover substantially more behavioral surface than gThumb’s visible developer tests: duplicate grouping, export paths and resize behavior, JXL assets/preview fallback, settings normalization/migration, SQLite library index reconciliation and migrations, ignored folders, collections, pipelines, EXIF orientation parsing, library scanning, operation queue events, quality scoring, tag database behavior, thumbnail cache and worker generation logic, UI helper behaviors, ComfyUI parsing, downloader cleanup/progress, upscale runner output, and tiling.

Sharpr’s test coverage is therefore a major strength. The remaining gap is that much of the GTK UI behavior, end-to-end folder browsing, real decoder behavior through Glycin, and long-running workflow UX still depend on manual testing or indirect unit tests.

## gThumb Architecture Deep Dive

### Application-Level Registry

`src/Application.vala` is the central registration hub. At startup, `Gth.Application` initializes global services and registries:

- Settings: `settings`, `viewer_settings`.
- Tests and sorters: hash tables plus ordered arrays.
- File sources: VFS, catalogs, selections.
- Metadata providers: file, Exiv2, image, video, comment sidecar.
- Loaders and external loaders: MIME-type keyed image/video loader functions.
- Savers and saver preferences: MIME-type keyed save functions and preferences pages.
- Jobs, I/O worker factory, image loader, thumbnail loader, image saver.
- Color manager, metadata reader/writer, devices, bookmarks, migration, scripts, image editor, shortcuts, tools, selections.

This registry design makes gThumb extensible without dynamic plugins. Most features register themselves into application-owned lists or maps, and the UI can ask the application which tests, sorters, loaders, savers, sources, tools, and viewers exist.

Pros:

- Clear discoverability for available features.
- Easy to add new formats, metadata providers, file sources, and tools.
- Good fit for a broad app with many optional capabilities.
- Allows multiple feature groups to plug into the browser/viewer/editor without each page knowing everything directly.

Cons:

- A large global object accumulates responsibility.
- Ordering and priority are implicit in registration order.
- Type safety is weaker than a Rust trait/service design.
- Testing isolated feature behavior can be harder because many components reach into `app`.

For Sharpr, the lesson is to add explicit registries where the feature surface is genuinely extensible, not to create one global “god app.” Good candidates are file actions, saved views/search predicates, export encoders, metadata panels, and maybe viewer backends. Bad candidates are core library/index state, which should remain typed and explicit.

### UI Pages And State Transitions

`src/MainWindow.vala` defines three primary pages through a `Gtk.Stack`:

- Browser
- Viewer
- Editor

The `set_page` method handles page transitions, save prompts for modified viewer images, sidebar width restoration, thumbnail list start/stop, browser selection restoration, and viewer/editor handoff. This page-state logic is direct and pragmatic. It centralizes the high-level page lifecycle in one place and gives each page clear hooks like `before_close_page` and `after_open_page`.

Sharpr has an app shell centered in `src/ui/window.rs`, with `AppState`, `ViewScope`, sidebar, filmstrip, viewer, compare page, tasks page, operations indicator, preferences, and workflows. Sharpr’s state model is more data-heavy because it owns a persistent index and multiple virtual scopes. However, `window.rs` is also visibly large and dense. It contains state definitions, helper functions, dialogs, callbacks, and workflow logic.

Recommendation for Sharpr: adopt the lifecycle clarity of gThumb’s page model. Keep `ViewScope`, but make each major content mode expose a small lifecycle contract: enter, leave, refresh, selection changed, current file changed. This would reduce the burden on `window.rs` without requiring a full rewrite.

### Browser, Sources, Catalogs, And Selections

gThumb’s `Browser.vala` coordinates:

- Folder tree
- File grid
- Filter bar
- Property sidebar
- Folder status/free space
- VFS/catalog/selection sidebar modes
- History and bookmarks
- Sorting/filtering
- Thumbnailer activation

The abstraction underneath is `FileSource.vala`, an abstract source interface with methods for roots, display info, metadata reads, child enumeration, monitoring, add/remove, reorder/save order, and renamed-file notification.

Implemented sources include:

- `FileSourceVfs`: real filesystem folders.
- `FileSourceCatalogs`: saved catalogs.
- `FileSourceSelections`: temporary user selections.

This is one of gThumb’s best designs. It lets catalogs and selections behave like navigable file locations instead of special one-off screens. Search results can also be saved as catalogs, creating continuity between browsing and organization.

Sharpr has an explicit `ViewScope` enum:

- Folder
- Collection
- Duplicates
- Search
- Quality
- Compare

Sharpr’s approach is type-safe and maps well to its SQLite index. But each scope has to be wired through application logic. gThumb’s source abstraction is more uniform for user navigation.

Recommendation for Sharpr: consider a typed `ContentSource` or `LibraryView` abstraction above `ViewScope` only if it improves reliability for current folders, collections, quality views, duplicates, compare queues, and task-generated views. It should not erase folder-truth semantics or become a saved-search project.

### Image Loading And Decoding

gThumb’s image loading is centered on `ImageLoader.vala` and low-level loaders under `src/lib/io`. The loader:

- Queries file attributes and ETag.
- Reads streams asynchronously.
- Guesses content type.
- Uses registered MIME-type loader functions.
- Supports external file loaders for video/audio thumbnails.
- Can skip huge images for preloading with `LoadFlags.NO_BIG_IMAGES`.
- Loads metadata through registered metadata providers.
- Applies monitor color profiles after decode.

This design has a strong feature surface. It handles multiple formats, metadata, animation frame counts, color profile integration, video thumbnails, and requested-size decode paths.

The tradeoff is security and maintenance. gThumb directly links many native codec libraries and custom C/C++ helpers. This gives control and performance, but image decoding is a risky attack surface. Every extra native codec path increases maintenance and security exposure.

Sharpr uses Glycin for the main viewing pipeline, which is a better default security stance for a modern GNOME app because decoding is sandbox-oriented. Sharpr also has Rust/image-rs/JXL/WebP/turbojpeg paths in the thumbnail/export/upscale areas. This creates a split pipeline: secure viewer decode on one side, worker thumbnail/export decode on another.

Recommendation for Sharpr: keep Glycin as the primary viewer decode path and document exactly where non-Glycin decoders remain. Long term, reduce decoder divergence where possible. gThumb is a good feature reference for requested-size decode, metadata integration, color handling, and saver preferences, but Sharpr should preserve its sandbox-first design.

### Saving, Metadata, And Sidecars

gThumb’s `ImageSaver.vala` is notably mature. It:

- Saves through MIME-type registered saver functions.
- Looks up format-specific saver preferences.
- Handles ICC profile conversion.
- Writes embedded metadata with Exiv2 when possible.
- Falls back to comment sidecar XML if embedded metadata cannot be saved.
- Updates ETags after replace/create.

gThumb’s metadata model includes registered metadata categories and fields, Exif/IPTC/XMP property views, comment metadata, rating, tags, title, description, place, date, camera metadata, coordinates, and video metadata.

Sharpr currently has strong metadata reading for curation and orientation. Owner direction rejects embedded metadata writing and IPTC/XMP tag export, so metadata work should stay read-only and quality-review focused. Export metadata privacy can be revisited later as a one-time popup and preference toggle.

Recommendation for Sharpr: keep metadata policy deliberately non-destructive:

- Local tags/collections remain Sharpr-only curation state.
- No embedded metadata writing.
- No Sharpr tag export to IPTC/XMP.
- Optional PNG sidecars are user-controlled output artifacts only.
- Export metadata privacy is a later preference/popup.
- Originals are never modified except explicit user trash.

gThumb’s embedded-or-sidecar fallback is useful background knowledge, but it should not become a Sharpr feature direction right now.

### Thumbnailing

gThumb’s `Thumbnailer.vala` supports freedesktop-style thumbnail sizes and subdirectories:

- normal: 128
- large: 256
- x-large: 384
- xx-large: 512

It validates cached thumbnails against source metadata, generates missing thumbnails, saves failed thumbnails, overlays film holes for video, applies monitor profiles, and queues active thumbnail work against the global thumbnail loader worker pool.

Sharpr’s thumbnail pipeline has a more explicit worker design:

- Visible worker pool and preload pool.
- Bounded request channels.
- Generation counters to drop stale work on folder switches.
- Pending path deduplication.
- Thumbnail result messages returned to GTK main thread.
- Hash and sharpness work chained through the worker system.
- Disk cache paths fingerprint source path, size, and mtime.
- Benchmark events around start/finish/failure/stale results.

Sharpr’s thumbnail pipeline is stronger for responsive large-folder curation and observability. gThumb’s cache format is stronger for desktop interoperability and standard thumbnail semantics.

Recommendation for Sharpr: consider whether Sharpr thumbnails should interoperate with the freedesktop thumbnail cache. If not, document why Sharpr uses a private cache. If yes, borrow gThumb’s valid-thumbnail metadata model. Either way, keep Sharpr’s generation-based stale cancellation and visible/preload split.

### Jobs And Progress

gThumb’s `Job.vala`, `JobQueue`, `ProgressDialog.vala`, and progress rows provide a coherent user-facing operation model. Jobs have:

- IDs
- Cancellable objects
- Progress
- Title/subtitle/icon
- Running/cancelled/completed state
- Foreground/hidden flags
- Open dialog accounting
- Toast integration
- Automatic progress dialog behavior after a delay

Sharpr has `src/ops/queue.rs`, operation events, and an operations indicator. Its unit tests verify ordered events, terminal events, monotonic IDs, dropped handle behavior, and concurrent unique IDs. The model is solid, but the UI surface is not as broadly used as gThumb’s jobs across all workflows.

Recommendation for Sharpr: make Tasks/operation queue usage mandatory for every long-running workflow: indexing, thumbnail backfill, hash generation, quality scoring, smart tagging, export, upscale, downloads, compare outputs, and generated-output decisions. Import workflows, batch rename, scripts, and metadata writing are out of scope.

### Editor And Tools

gThumb has a substantial editor/tool surface:

- Brightness, colors, contrast, saturation.
- Crop, resize, rotate, sharpen, grayscale, special effects.
- Color picker, curves, aspect ratios, grid/mask selectors.
- Format conversion and saver preferences.
- JPEG/metadata-related operations.
- Print and desktop background tools.

Sharpr’s editing scope is intentionally different. It has export and upscale/comparison workflows rather than an integrated destructive editor. Owner direction makes this stricter: originals should not be modified, rotate/flip is out of scope, and crop/resize/format conversion should live only in export/task workflows.

Recommendation for Sharpr: do not become a full pixel editor immediately. Instead, add “curation-safe edits”:

- Export-only transform/orientation handling if it is ever needed.
- Export-time crop/resize/format presets.
- Format conversion through export/task workflows only.
- Compare-before-save for generated/upscaled outputs.

gThumb’s editor code is useful as a feature checklist, not necessarily as a direct architectural model.

### Search, Filters, Templates, Scripts

gThumb has a rich extension set:

- Filters and saved tests.
- Search dialogs and source editors.
- Template parsing for rename/script workflows.
- Scripts with parameter dialogs.
- Shortcuts preferences.
- Saved catalogs and selections.

The test coverage around templates and string helpers shows these small subsystems are treated as reusable primitives.

Sharpr already has tags, collections, smart tagging, quality views, duplicates, Tasks, and compare/task-result virtual-folder behavior. The missing pieces are reliability and product focus around these existing workflows, not saved searches or user automation.

Recommendation for Sharpr: prioritize collections, Tasks, generated-output tracking, keyboard navigation, and filmstrip reliability. Saved searches, batch rename, and arbitrary scripts should not be prioritized.

### UI Templates Versus Code-First UI

gThumb uses many `.ui` templates and `GtkTemplate` classes. This makes layout structure easy to inspect, supports GNOME design tooling, and keeps some layout noise out of code. The cost is split behavior across Vala and XML, and sometimes more indirection.

Sharpr is mostly code-first. This fits Rust/gtk-rs well and keeps type flow in one language, but large UI files can become hard to navigate. `src/ui/window.rs` is the most visible example: it contains app state, helper functions, dialogs, collection UI, callbacks, and workflow code.

Recommendation for Sharpr: do not switch wholesale to `.ui` templates. Instead, carve code-first UI into page/controller modules with clear ownership. gThumb’s template count is a reminder that UI structure deserves dedicated files; the Rust equivalent is smaller, focused builders/controllers rather than a monolithic window file.

## Feature Comparison

| Area | gThumb | Sharpr |
| --- | --- | --- |
| GNOME stack | GTK4, libadwaita, Vala, C/C++ | GTK4, libadwaita, Rust |
| Main identity | Viewer, browser, editor, organizer | Folder-truth image quality review, filmstrip, collections, Tasks |
| Viewer | Mature, multiple viewers, image/video/audio support | Modern viewer, Glycin-based image pipeline, compare/upscale flows |
| Browser | Folder tree, grid, filters, properties, history, bookmarks | Sidebar, filmstrip, library scopes, collections, virtual views |
| File sources | VFS, catalogs, selections via `FileSource` abstraction | `ViewScope` enum, folders as truth, SQLite-assisted loaders |
| Persistent index | Catalog/comment files and settings; no Sharpr-like global SQLite image index seen | SQLite support infrastructure for cache/curation/task state |
| Tags | Metadata/comment tags and catalog organization | SQLite tag DB, smart tagging, collections-as-tags migration |
| Search | Filters/search can become catalogs | Search exists, but saved searches are out of scope for now |
| Duplicates | Not a central feature in inspected 4.0 alpha source | Perceptual hash duplicate detection |
| Quality scoring | Not a central feature | Resolution-based quality tiers (720p/900p/1080p/1440p/4K+) |
| Metadata | Very strong EXIF/IPTC/XMP/comment categories and property views | Reading/orientation/tag storage; no embedded metadata writing direction |
| Color management | lcms2/colord integration, ICC-aware save path | Less visible color management; Glycin may handle decode path, export paths need policy |
| Thumbnailing | Standard-size cache, validation, failed thumbnail records, video film holes | Worker pools, generation cancellation, private cache, hash/sharpness integration |
| Editing | Rich integrated editor tools | Export/upscale rather than full editor |
| Import | Device/import flows | Out of scope; Sharpr works with folders and library hot swap |
| Export/save | Format savers, saver preferences, metadata write/fallback | Export module, upscale output, format conversion pieces |
| Batch actions | Rename, convert, resize, scripts, file manager actions | Format conversion belongs only in export/task workflows |
| Progress | Unified job queue/dialog/toasts | Operation queue and indicator; good tests, needs universal adoption |
| Tests | Four developer-mode utility/template tests | 138 passing Rust tests across many subsystems |
| Packaging | Meson, Flatpak, translations, appdata, schemas | Cargo, Flatpak manifest, GResource, schemas |
| Internationalization | Full GNOME translation setup with many `.po` files | No comparable translation maturity visible |

## Sharpr Strengths

Sharpr’s strongest engineering choices are:

- Rust implementation with typed data structures and safer memory defaults.
- SQLite-backed support infrastructure with reconciliation, migrations, ignored folders, collections, pipelines, quality, phash, and tests.
- Non-destructive local-first curation model.
- Glycin-based main image viewing pipeline, which is a modern security-conscious choice.
- Thumbnail worker architecture with visible/preload separation, generation cancellation, pending dedupe, and benchmark logging.
- Broad unit test coverage compared with gThumb’s visible tests.
- Curation-specific features that gThumb does not emphasize: quality scoring, duplicate groups, AI tag suggestions, upscale pipelines, comparison page.

These are not small advantages. Sharpr is less feature-complete, but the core curation engine is already more modern than a traditional file browser/editor.

## Sharpr Weaknesses And Risks

The main weaknesses are not raw capability gaps alone; they are integration and product-surface gaps:

- `src/ui/window.rs` is too central. It knows too much and will become harder to evolve.
- Scopes are present, but not yet abstracted into a uniform source/view contract like gThumb’s `FileSource`.
- Export metadata privacy policy is still pending.
- Task workflows and generated-output tracking need more polish; rename/import/scripts are not Sharpr goals.
- Color management and saver preferences need an explicit product decision.
- Thumbnail cache is private and pragmatic, but its relationship to the freedesktop thumbnail spec is not documented.
- Progress is well-modeled but should be surfaced consistently across all long tasks.
- Format support is split across Glycin, image-rs, JXL/WebP/turbojpeg, rexiv2, and upscale/export paths; this can produce inconsistent behavior if not documented and tested.
- Internationalization and GNOME polish lag gThumb’s long-lived app maturity.

## gThumb Strengths

gThumb’s strongest qualities:

- Complete user workflow coverage: browse, view, edit, organize, import, search, print, scripts, rename, convert, metadata, slideshow.
- Uniform source abstraction for real folders, catalogs, and selections.
- Mature metadata model with embedded and sidecar behavior.
- Format-specific saver preferences and save pipeline.
- Strong GNOME integration: settings schemas, templates, appdata, translations, Flatpak, libadwaita UI.
- Good central registration model for formats, viewers, sorters, tests, metadata providers, and tools.
- Coherent job/progress model.
- Color management is treated as a first-class concern.

## gThumb Weaknesses And Risks

gThumb’s current branch also has weaknesses:

- Vala plus C/C++ native codec code is more memory-risk-prone than Rust plus sandboxed decoding.
- The central `Application` registry can become a global dependency magnet.
- Developer-mode tests are narrow for such a broad app.
- Optional dependency matrix is large and can complicate builds.
- The GTK template split is clean for UI layout but can make behavior harder to trace across XML and Vala.
- The 4.0 alpha branch is modern but not necessarily as battle-tested as stable gThumb releases.

## Implementation Lessons For Sharpr

### Owner Decisions / Product Corrections

The original comparison report intentionally used gThumb as a broad feature map. Owner direction narrows that map:

- Sharpr is for sorting images for quality review.
- Folders are the truth; SQLite supports speed, stability, cache, task history, and curation state.
- No saved searches for now.
- Collections, Tasks, generated-output tracking, and the filmstrip are central.
- Compare populates the filmstrip from compare/task results, but this behavior is fragile and needs hardening before it can be relied on.
- Originals are never modified. Explicit trash is allowed; export/upscale/format conversion creates controlled outputs.
- No embedded metadata writing, no IPTC/XMP tag export, no import workflow, no batch rename, and no arbitrary scripts.
- Rotate/flip is intentionally out of scope. GNOME Image Viewer handles pixel-level adjustments; code removal from Sharpr is still pending.
- Flathub is not a real target, but GNOME polish, keyboard accessibility, thumbnail reliability, and manual QA are core.

### 1. Add A `LibraryView`/`ContentSource` Layer

Sharpr should keep `ViewScope`, but a trait-like layer may still help make current folders, collections, duplicates, quality views, compare queues, and task-generated result views feel uniform to the window shell. This is a reliability/refactoring aid, not a path toward saved searches.

Suggested behavior surface:

- Stable source ID.
- Display title/subtitle/icon.
- Load rows from index or filesystem.
- Refresh/invalidate.
- Select path.
- Supports add/remove/reorder flags.
- Source/output lineage for task-generated views where needed.

This borrows gThumb’s `FileSource` benefit without losing Rust typing or the folder-truth model.

### 2. Keep Saved Searches Out Of Scope

gThumb’s search/catalog continuity is valuable, but it does not match current owner direction. Sharpr should not implement saved searches now. Current effort should instead strengthen:

- Collections.
- Quality and duplicate views.
- Compare/task-generated virtual folder behavior.
- Filmstrip selection and thumbnail reliability.
- Generated-output tracking and tag/collection inheritance.

### 3. Define Metadata And Output Privacy Policy

Sharpr's policy should be explicit:

- Local DB tags and collections are non-destructive and remain Sharpr-only curation state.
- No embedded metadata writing.
- Do not export Sharpr tags to IPTC/XMP.
- Optional PNG sidecars are user-controlled generated-output artifacts only.
- Export metadata privacy should be handled later through a one-time preference popup and preference toggle.
- Original files are never modified unless the user explicitly trashes them.

Near-term metadata work should be read-only and quality-review focused.

### 4. Unify Long-Running Operations

Sharpr’s `OpQueue` should become as universal as gThumb’s `JobQueue`.

Every long task should emit:

- Added.
- Progress when measurable.
- Terminal success/failure/cancel.
- User-visible title/subtitle.
- Optional action target, such as “show results” or “open output.”

This should cover indexing, thumbnailing beyond visible tiles, metadata backfill, hash backfill, quality scoring, smart tagging, export, upscale, downloads, generated-output review, and compare/task-result decisions.

### 5. Add Format And Saver Preferences

gThumb’s saver preference design is a useful model. Sharpr already exports and upscales; users need durable format choices:

- JPEG quality and metadata policy.
- PNG compression level if exposed.
- WebP quality/lossless.
- JXL quality/effort.
- AVIF/HEIF if Sharpr adds first-class support.
- Default export directory/preset.
- Export metadata privacy preference when that work is prioritized.

This belongs in Sharpr settings and export UI, not scattered across workflows.

### 6. Keep Editor Scope Curation-Focused

Sharpr should not compete with gThumb’s full editor. The best near-term work is:

- Export-time resize/format presets.
- Format conversion through export/task workflows only.
- Compare output management.
- Generated-output collection/tag inheritance.

Rotate/flip is out of scope; users can open the image in GNOME Image Viewer when they need pixel-level adjustments.

### 7. Document Decoder Boundaries

Sharpr should maintain a short architecture document that says which subsystem decodes which formats and why:

- Viewer: Glycin.
- Thumbnail workers: current Rust/native paths.
- Export: current image/JXL/WebP paths.
- Metadata: rexiv2/GExiv2.
- Upscale/ComfyUI: input/output handling paths.

This matters because inconsistent orientation, color, metadata, or format support can otherwise appear between viewer thumbnails, export, and upscale.

## Priority Roadmap For Sharpr

### Near-Term

1. Split `src/ui/window.rs` responsibilities into focused controllers/builders for collection dialogs, scope loading, compare navigation, and batch actions.
2. Add or refactor toward a `LibraryView`/`ContentSource` abstraction only if it improves reliability for current folders, collections, quality, duplicates, compare, and task-generated views.
3. Make Tasks/`OpQueue` mandatory for all long-running operations and audit call sites.
4. Add a report-style architecture note documenting decoder/cache/index boundaries.
5. Add thumbnail-loading, rapid-folder-switching, compare/task virtual-folder, collections/tag inheritance, and generated-output tracking regression tests.

### Medium-Term

1. Add richer read-only metadata panel support: EXIF/camera/file/color/dimensions/tags in a structured property view.
2. Add export/saver preferences modeled after gThumb’s format-specific pages.
3. Add generated-output inheritance and output collections for upscale/format/export results.
4. Add export metadata privacy one-time popup and preference toggle.
5. Improve GNOME polish, keyboard-only navigation, and manual QA coverage.

### Longer-Term

1. Add user-configurable ComfyUI/API backends only as explicit backends, not arbitrary scripts.
2. Add optional freedesktop thumbnail cache interoperability or document why Sharpr remains private-cache only.
3. Add color management policy and tests across view/export paths.
4. Add more integration tests for UI-adjacent workflows, possibly with headless GTK where practical.
5. Consider a lite mode later if AI-heavy features need separation.

## Things Sharpr Should Avoid Copying

- Do not copy gThumb’s global `Application` registry as-is. Use typed Rust services and traits.
- Do not add every gThumb editor operation before Sharpr’s curation workflows are polished.
- Do not mutate original files for metadata, rotate, or normal curation operations.
- Do not add saved searches, import workflows, batch rename, embedded metadata writing, IPTC/XMP tag export, or arbitrary scripts now.
- Do not expand native decoder surface without a security and maintenance reason.
- Do not split UI into many templates just because gThumb does; Sharpr can get the same maintainability from focused Rust modules.

## Final Assessment

gThumb is the broader and more mature GNOME image application. It has years of product knowledge embedded in its workflows: catalogs, selections, import, metadata, editor tools, progress, templates, scripts, saver preferences, and color management. It is the right project to study for user-facing completeness.

Sharpr is the more modern quality-review curation tool. Its Rust codebase, SQLite support infrastructure, strong tests, benchmark logging, duplicate/quality features, Tasks, filmstrip, collections, and Glycin-based viewer give it a strong technical foundation. Its main risk is not that it lacks gThumb’s exact features; it is that thumbnail loading, task/compare virtual-folder behavior, generated-output tracking, collections, and keyboard navigation regress as scope grows.

The best path is selective borrowing. Use gThumb’s interaction patterns only where they strengthen Sharpr’s current identity: folder-truth browsing, filmstrip-first review, collections, Tasks, non-destructive generated outputs, secure decoding boundaries, and universal operation progress.

## GTK/libadwaita UI Implementation Lessons

This appendix focuses only on the GTK4/libadwaita implementation style. gThumb and Sharpr are both modern GNOME apps, but they organize UI work differently: gThumb leans on `GtkTemplate`, `.ui` resources, action names, and page classes; Sharpr leans on Rust widget subclasses, code-built layouts, shared `Rc<RefCell<AppState>>`, and explicit callback wiring between panes. Both approaches are valid. The useful lessons for Sharpr are about lifecycle boundaries, action consistency, and where UI state should live.

### 1. Patterns gThumb Uses Well

gThumb’s strongest GTK pattern is explicit page lifecycle ownership. The main shell is defined in `data/ui/main-window.ui` as an `AdwToastOverlay` containing a `GtkStack` with `GthBrowser`, `GthViewer`, and `GthEditor`. The behavior is centralized in `src/MainWindow.vala`, especially `Gth.MainWindow.set_page`. That method does real lifecycle work: prompts before leaving a modified viewer image, calls `editor.before_close_page`, stops or starts thumbnailers through browser/viewer hooks, preserves sidebar widths, restores browser selection from viewer state, and switches the stack child only after page-specific cleanup is handled.

The page classes then own their local GTK details. `Gth.Browser` in `src/Browser.vala` owns the folder tree, file grid, filter bar, property sidebar, history, bookmark popovers, and browser-specific thumbnail activation. `Gth.Viewer` in `src/Viewer.vala` owns `current_file`, `current_viewer`, viewer thumbnail list behavior, property sidebar updates, save prompts, and preloading. The important point is that the main window chooses the page, but the page owns what it means to become active or inactive.

gThumb’s action model is also very idiomatic GTK. `Gth.Window` in `src/Window.vala` creates a `SimpleActionGroup`, inserts it as `win`, and exposes common actions such as `job-queue`, `cancel-all-jobs`, and close handling. `Gth.MainWindow.init_actions` adds the large command surface: `win.pop-page`, `win.toggle-fullscreen`, `win.open-with`, `win.edit-metadata`, file operations, scripts, rotate, metadata clearing, and so on. The UI templates and menu resources bind to those actions by name, for example `data/ui/browser.ui`, `data/ui/viewer.ui`, and `data/ui/browser-menu.ui`. This keeps toolbar buttons, menu items, shortcuts, and code paths aligned around one command vocabulary.

The browser and viewer layouts use libadwaita containers in conventional ways. `data/ui/browser.ui` puts sidebar and content into `AdwOverlaySplitView` and wraps both sides in `AdwToolbarView` plus `AdwHeaderBar`. `data/ui/viewer.ui` uses an `AdwToolbarView` with a headerbar, then nested `AdwOverlaySplitView` instances for the properties/editor side panel and the thumbnail strip. This is heavier than Sharpr needs, but it maps well to libadwaita’s adaptive model: headerbars contain page commands, split views handle side panels, and content widgets remain focused.

Selection handling is cleanly encapsulated in `Gth.FileGrid` (`src/FileGrid.vala`). It wraps `Gtk.GridView` with either `Gtk.SingleSelection` or `Gtk.MultiSelection`, and exposes `select_file`, `select_position`, `select_files`, `get_selected_file_data`, `get_selected_files`, and `get_selected_file_data_list`. That gives `MainWindow` and `Viewer` a stable selection API instead of forcing them to manipulate GTK selection internals. `Gth.Viewer.view_file_async` then maps a viewed file back to the browser position and keeps `current_file`, property sidebar, title, sensitivity, and preloader state in sync.

The property panel is another strong pattern. `Gth.PropertySidebar` in `src/PropertySidebar.vala` owns a stack of `PropertyView` pages: file properties, EXIF, IPTC, XMP, other properties, and selection info. It registers a local `sidebar.set-view` action and builds toggle buttons that switch the stack. It also hides irrelevant pages based on `can_view`, updates search only for pages that support it, and treats selection info as a first-class panel state. This is a good model for a richer Sharpr metadata/properties area.

For long-running work, gThumb connects GTK feedback to cancellable jobs. `Gth.Job` and `Gth.JobQueue` in `src/Job.vala` track progress, title, subtitle, icon, foreground/hidden state, cancellation, open dialogs, errors, and terminal state. `Gth.ProgressDialog` in `src/ProgressDialog.vala` binds jobs to rows with a cancel button and progress bar, delays automatic presentation, and keeps the window busy state in sync. `Gth.Window.show_error` and `show_message` convert routine feedback to `Adw.Toast`, suppressing cancellation/dismissal noise. This is more user-facing than a simple busy indicator.

Preferences are also implemented in a libadwaita-native way. `Gth.PreferencesDialog` in `src/Ext/Preferences/PreferencesDialog.vala` uses `AdwNavigationSplitView`, `AdwNavigationView`, `AdwToolbarView`, and a sidebar list of preference pages defined in `data/ui/preferences-dialog.ui`. It is a useful reference for when preferences become too large for one long page.

### 2. Matching Sharpr Areas

Sharpr’s equivalent shell is `SharprWindow` and `AppState` in `src/ui/window.rs`. The current content stack is built around `gtk4::Stack` and named pages such as viewer, tags, tasks, and compare. The active content scope is represented by `ViewScope`, with cases for folder, collection, duplicates, search, quality, and compare. This is a good curation-specific model, but much of the transition and workflow logic still lives directly in `window.rs`: folder opening, disabled folder handling, duplicate detection, quality views, collection dialogs, thumbnail generation bumps, toasts, and stack switching.

Sharpr’s pane classes already follow the better side of the gThumb pattern. `SidebarPane` in `src/ui/sidebar.rs` owns the library/folder/collection sidebar UI and exposes callbacks such as `connect_folder_selected`, `connect_collection_selected`, `connect_collection_add_requested`, and `connect_drop_paths_to_collection`. `FilmstripPane` in `src/ui/filmstrip.rs` owns the `gtk4::ListView`, `gtk4::SingleSelection`, search bar, sort popover, quality filter, thumbnail scheduling, context menu actions, and activation callbacks. `ViewerPane` in `src/ui/viewer.rs` owns viewer gestures, zoom shortcuts, tag popover, metadata display, and smart-tag interactions. These are the right boundaries; the missing piece is reducing how much orchestration remains in the window.

Sharpr’s filmstrip is more modern than gThumb’s grid in some ways. It uses bounded thumbnail worker channels, visible/preload sender separation, pending-path dedupe, generation checks, scroll throttling, delayed rescheduling, and benchmark events in `FilmstripPane::schedule_visible_thumbnails`. gThumb’s `Gth.FileGrid` has a simpler GTK-facing API and uses `Gtk.GridView`/multi-selection well, but Sharpr’s thumbnail scheduling is better aligned with very large curation folders.

Sharpr’s action and shortcut model is less unified. It does use `gio::SimpleAction` in `src/app.rs` for `app.about` and in `src/ui/window.rs` for commands such as duplicate search, tags, quality scan, presentation mode, zoom, metadata visibility, manual, preferences, and conversion. It also uses `gtk4::ShortcutController` directly in `src/ui/window.rs`, `src/ui/viewer.rs`, and `src/upscale/comparison.rs`. This works, but gThumb’s action resources and shortcut registry make the command surface easier to audit. Sharpr would benefit from a small action/shortcut map before the command set grows further.

For feedback, Sharpr has `OpQueue` in `src/ops/queue.rs` and `OpsIndicator` in `src/ui/ops_indicator.rs`. The queue model is well-tested and emits added, progress, completed, failed, and dismissed events. The indicator currently uses a headerbar button with busy/idle icon state and routes users to Tasks. That is simpler than gThumb’s `ProgressDialog`, and it fits Sharpr’s viewer-first design, but it underuses the operation metadata already available. Tasks and pipeline history are richer elsewhere in `src/ui/tasks_page.rs`; the headerbar indicator could bridge to that more clearly.

Sharpr’s preferences are already libadwaita-native through `build_preferences_window` in `src/ui/preferences.rs`, using `AdwPreferencesWindow`, `AdwPreferencesPage`, `AdwPreferencesGroup`, `ActionRow`, `SwitchRow`, `EntryRow`, and `ComboRow`. This is more direct and simpler than gThumb’s navigation split view. It should stay that way until the preferences surface becomes too large.

Sharpr’s metadata UI is intentionally compact: `MetadataChip` in `src/ui/metadata_chip.rs` renders a bottom-right OSD-style summary of dimensions, format, size, and quality. That is appropriate for a viewer-first app. But it is not a replacement for gThumb’s property sidebar. Sharpr needs both: a lightweight viewer chip and an optional richer property panel.

### 3. Concrete Lessons Sharpr Should Adopt

Sharpr should add explicit lifecycle helpers for content transitions. This does not mean rewriting the app. It means giving each major page or scope a small set of methods that `SharprWindow` can call consistently: before leaving, after entering, refresh, clear selection, restore selection, and cancel stale background work. gThumb’s `set_page`, `before_close_page`, and `after_open_page` pattern is worth copying at the concept level.

Sharpr should make action names the primary command interface for headerbar buttons, menu items, and shortcuts. Today, some commands are actions, while many pane controls connect directly to closures. Direct callbacks are fine inside a pane, but app-level commands like search, tags, presentation, metadata, duplicate scan, quality views, tasks, preferences, delete/trash, add to queue, and collection operations should have a consistent action map. That would make menus, shortcuts, and future help overlays easier to keep in sync.

Sharpr should keep pane-owned selection APIs. `FilmstripPane::connect_image_selected`, `navigate_to`, `select_index`, `connect_item_activated`, and context callbacks are already the right direction. The gThumb lesson is to keep that API complete enough that window code never needs to know `SingleSelection` or `ListView` details.

Sharpr should grow a real property/details panel inspired by `Gth.PropertySidebar`, but scoped to Sharpr’s goals. It should not expose EXIF/IPTC/XMP editing all at once. A good first version would have pages for file basics, image metadata, quality/duplicate information, tags/collections, and pipeline/output history for the selected image. The existing `MetadataChip` should remain as the fast viewer OSD.

Sharpr should make operation feedback more informative. Keep the headerbar Tasks button, but let it show active operation count/title in a tooltip or popover, expose cancel for cancellable operations where possible, and route failed operations to a clear Tasks/history view. gThumb’s progress rows are too heavyweight for every Sharpr operation, but its title/progress/cancel pattern is worth adopting.

Sharpr should continue using code-built Rust UI for complex custom widgets, but should use resource-backed `gio::Menu` and action names where they reduce boilerplate. gThumb’s `.ui` templates are not inherently better; their real benefit is that menus and commands are declarative and auditable.

### 4. Things Sharpr Should Avoid Copying

Sharpr should not copy gThumb’s full command surface. gThumb is also a file manager, editor, importer, printer, script runner, video viewer, and metadata editor. Sharpr is a curation-first viewer. Commands should be added only when they support browsing, selecting, tagging, comparing, exporting, or task workflows.

Sharpr should not switch wholesale to `GtkTemplate` and `.ui` files. gThumb benefits from templates because it has many traditional dialogs and pages. Sharpr’s custom Rust widgets, worker integration, and curation-specific panes are easier to keep correct in code. The better move is to split large code-built UI into focused modules.

Sharpr should not adopt gThumb’s global application registry as a UI architecture. gThumb’s `Gth.Application` works for its extension-style Vala app, but Sharpr should prefer typed Rust traits, focused service structs, and explicit ownership.

Sharpr should not implement customizable shortcuts immediately. gThumb’s `Gth.Shortcuts` and `Gth.Shortcut` are impressive, with context-aware commands and XML persistence, but they solve a mature-app problem. Sharpr first needs a stable action map and a documented default shortcut list.

Sharpr should not turn the viewer into a full editor just because gThumb has editor tools. The UI should keep curation-safe actions prominent and push destructive or format-changing operations into explicit export/task flows.

### 5. Prioritized UI And Code-Organization Improvements

1. Extract page/scope transition helpers from `src/ui/window.rs`: enter viewer, enter tags, enter tasks, enter compare, load folder scope, load virtual scope, and clear stale work.
2. Add a small action registration helper or table in `SharprWindow` so actions, menu labels, shortcuts, and sensitivity updates are easier to audit.
3. Expand `OpsIndicator` into a useful Tasks affordance: active count, current title in tooltip/popover, visible failed state, and cancel where the underlying `OpHandle` supports it.
4. Add a Sharpr property/details panel separate from `MetadataChip`, using gThumb’s `PropertySidebar` idea but with Sharpr pages: file, metadata, tags/collections, quality/duplicates, and task history.
5. Move collection/library dialogs and popover construction out of `src/ui/window.rs` into focused modules, following the existing `preferences.rs`, `sidebar.rs`, and `filmstrip.rs` pattern.
6. Document the app-level action and shortcut map before adding customization. Use that map to align headerbar buttons, menu items, help/manual text, and `ShortcutController` bindings.
