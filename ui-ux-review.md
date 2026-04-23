# Sharpr UI/UX Review

## Summary
Sharpr is not just a viewer; the code shows a folder-backed image triage and curation tool with persistent indexing, disabled folders, thumbnail workers, perceptual duplicate detection, tags, collections, quality scoring, basic transforms, and three AI upscale backends. The three-pane UI is a strong foundation, but the product is already carrying too many concepts for its first screen. The main problem is not missing functionality; it is that important functionality is either overexposed in the sidebar or buried in menus, context menus, and keyboard shortcuts. The screenshots make upscaling and quality scoring look cryptic, while the code shows a much more complete but heavy workflow behind them. The app is drifting from "fast GNOME image library viewer" toward "photo manager plus AI lab plus lightweight editor." That direction is risky because the target use case is browsing and managing large compressed-image collections, not full digital asset management. The product should keep its powerful internals, but the default UX needs to become calmer, denser, and more explicit about workflows.

## First Impressions
The first screen says "library manager", not "image viewer". That is acceptable for the stated use case, but the app immediately presents folders, smart folders, quality buckets, collections, tags, search, duplicates, preview metadata, and upscaling hints. A new user can understand the broad shape, but not the priority.

The actual implementation is more capable than the screenshot suggests. There are context-menu actions for opening externally, revealing in the file manager, copying, trashing, and collection management. There are keyboard shortcuts for navigation, fullscreen, metadata, trash, tags, and zoom. There is a real before/after comparison view for upscaling. None of that is obvious from the default screen.

The UI currently makes the wrong things visible. Quality classes and collections get permanent sidebar space, while core selected-image actions are hidden behind right-click or the main menu. For image triage, delete, reveal, compare, tag, open externally, and upscale are primary actions.

## UI/UX Evaluation
The three-pane structure is the right base: sidebar for sources and smart views, filmstrip for candidates, viewer for inspection. The split view and adaptive breakpoints fit GNOME patterns, and the app already handles narrow windows by collapsing sidebars. That is a solid structural decision.

The filmstrip is too low-density for large libraries. It uses a virtualized `GtkListView` and background thumbnail scheduling, so the implementation is not naive. The UX still shows only a handful of images at once because each row uses a large 160px thumbnail plus filename. That is good for slow visual review, bad for scanning thousands of screenshots, wallpapers, or web images.

The sidebar hierarchy is overloaded. "Folders", "Smart Folders", "Quality", and "Collections" are all permanent sections. Quality buckets are implemented as virtual views, not just labels, but they behave like navigation items without counts. Collections have counts; quality and smart folders mostly do not. This inconsistency makes the sidebar feel unfinished even though the backend is real.

The preview pane has usable zoom behavior in code: fit, 1:1, Ctrl+scroll, Ctrl+0, panning, and a zoom OSD. The screenshot does not reveal this. The main menu contains fit/1:1, and there is a zoom button path in the viewer, but the visible header does not communicate enough. A viewer must make zoom state and fit mode obvious because quality judgment depends on it.

The metadata overlay is useful but too jargon-heavy. The code names it `IQ`, computes it from long edge, megapixels, file size per pixel, and format, and gives a tooltip reason. That is not general "image quality"; it is technical suitability and compression/resolution scoring. Calling it "IQ" makes it feel like opaque AI scoring when it is actually a deterministic heuristic.

The app has basic editing: rotate, flip, save edit, discard edit. This is product-risky. It is useful, but it pushes Sharpr toward editor territory. These controls are currently tucked into the menu, which is good, but the app must keep them framed as lossless/simple file operations, not editing.

GNOME alignment is mixed. The widgets and layout are GNOME-native, but the product exposure is not. GNOME-style apps usually avoid showing every internal capability in the primary navigation. Sharpr should keep the capabilities but reveal them contextually.

## Feature Scope Analysis
Core features should be folder browsing, fast thumbnail loading, large preview, zoom/pan, metadata, search, duplicate detection, trash/reveal/open, and lightweight tagging. These directly support the use case of managing messy compressed-image libraries.

Persistent indexing is justified. The code uses SQLite for image metadata, ignored folders, quality class, perceptual hashes, and collections. This is the right infrastructure for large libraries, but it should remain invisible except through speed, counts, and reliable smart views.

Quality scoring is useful, but its current product framing is wrong. The scorer rewards resolution, megapixels, bytes-per-pixel, and format. That is a "technical quality / upscale candidate" signal, not an aesthetic or objective quality score. The UI should stop using "IQ" and label it as "Technical quality" or "Resolution quality".

AI tagging is better scoped than it first appears. The star button in the tag editor suggests tags locally and requires acceptance. That is the right pattern: optional, local, and subordinate to manual tags. It should not be elevated into main navigation.

AI upscaling is feature-complete enough to be a real workflow: CLI RealESRGAN, ONNX Swin2SR with model downloads, optional ComfyUI, scale selection, progress, output path handling, and before/after comparison. The risk is that this is large enough to become its own product inside Sharpr. Keep it as "improve selected image", not "batch AI studio".

Collections are useful but secondary. The implementation supports create, rename, delete, drag/drop, add/remove, counts, and collection virtual views. That is solid, but exposing collections permanently before a user has created or used them makes the app feel heavier than it needs to.

Tags and collections overlap as organization systems. The distinction is technically clear: tags are searchable metadata, collections are curated sets. The UI needs to make that distinction behavioral and visible, or users will not understand why both exist.

## Workflow Analysis
Browsing has good architecture and weak density. Background thumbnail workers, persistent cache, preloading, and virtual list rendering are the right technical choices. The missing UX piece is a compact grid or density selector. The current filmstrip works for preview-driven browsing, not for high-volume curation.

Viewing is strong but under-signposted. The code supports panning, fit, 1:1, zoom OSD, orientation handling, metadata loading on a background thread, prefetching adjacent images, and keyboard navigation. The UI should surface these affordances with a small viewer toolbar or clearer menu state.

Search is tag-centered, not general file/library search. The placeholder says "Search tags…", and the code searches tag paths. That is coherent if Sharpr's search means tag search, but the sidebar label "Search" is too broad. Rename it to "Tag Search" or add filename search.

Duplicate detection is useful but depends on hashes being available. The app warns users to browse the library first because hashes are computed as thumbnails load. That is a workflow smell. If duplicate detection is a sidebar feature, it should either schedule missing hashes itself or clearly show "indexing required" progress.

Upscaling is implemented as a serious modal workflow with backend/model/scale choices and save/discard comparison. This is powerful but too much detail at action time. Most users need a default "Upscale" path first, with advanced backend choices in preferences. The dialog should not make casual users think they need to understand CLI, ONNX, ComfyUI, models, and scale before improving one image.

File management actions exist but are hidden. Right-click on thumbnails exposes open, reveal, copy, add/remove collection, and trash. Delete also works from the keyboard. This is functional but undiscoverable. A selected-image action bar or popover would make the app feel like a manager instead of a viewer with secret management features.

## Edge Cases
Large libraries are partially handled well. SQLite indexing, thumbnail workers, visible/preload queues, operation indicators, disabled folder filtering, and cached metadata are all appropriate. The UI still needs result counts, indexing state, and denser browsing to make those internals visible as confidence rather than mystery.

Large images are technically considered. The viewer decodes off-thread, stores RGBA, supports fit/1:1/zoom/pan, and prefetches adjacent images. The UX risk is memory and feedback: users need to know when the image is still loading and when they are seeing scaled versus actual pixels.

Slow hardware will expose the AI scope problem. ONNX, ComfyUI, RealESRGAN, smart tagging, hashing, metadata indexing, and thumbnail generation all compete for resources. The shared operation indicator helps, but AI features should never block basic browsing or make the default app feel computationally heavy.

Users who do not care about AI should be able to ignore it completely. Today they still see "Needs Upscale", "IQ", an "AI Upscale" menu section, and smart tag affordances if the model exists. These should be hidden or collapsed unless analysis/upscale is intentionally used.

Users focused on AI upscaling have the opposite problem: the workflow is powerful but scattered across preferences, the main menu, a modal dialog, an operation indicator, and a comparison view. They need a clean queue/review path if batch upscaling becomes a real goal. Until then, keep it single-image and contextual.

## Product Direction
Sharpr should not become Lightroom. It already has tags, collections, quality classes, duplicates, transforms, smart tagging, multiple upscale backends, and persistent indexing. Adding ratings, albums, timelines, EXIF editing, face/person recognition, map views, or color editing would make the app incoherent.

The best direction is "fast GNOME image triage for messy folders, with optional technical-quality analysis and image improvement." That is specific and defensible. It matches the codebase better than a plain viewer and avoids the trap of becoming a full photo manager.

The app should not be split yet. The code can support modular workflows inside one app. The product should be split into modes or progressive surfaces: Browse, Organize, Analyze, Improve. Do not put all four on screen at equal weight.

AI should be contextual, not architectural identity. Smart tags and upscaling are useful because they support curation. If AI becomes the app's main identity, Sharpr will need far more model management, batch processing, error handling, previews, and export controls than the current UI can absorb.

## Recommendations
Remove the term "IQ" from the UI. Replace it with "Quality", "Technical quality", or "Upscale score". The tooltip can explain the formula in plain language: resolution, file size per pixel, and format.

Remove permanent visibility for empty or inactive advanced sections. Hide Collections until one exists or the user creates one. Collapse Quality by default until metadata analysis has produced usable results. Keep Smart Folders, but do not make every smart capability look equally important.

Simplify the sidebar. Start with Folders, Search, Duplicates, and optional collapsed sections for Quality and Collections. Add counts to every virtual view that can compute them. If a view requires indexing or hashes, show that state directly in the row or the resulting empty view.

Move core selected-image actions out of right-click-only access. Add a compact contextual action surface for reveal, open externally, trash, tag, add to collection, compare, and upscale. Keep destructive actions confirmed or visually distinct.

Simplify the upscale dialog. Use the saved/default backend and model for the primary flow. Put backend/model details behind an expander or "Advanced" section. The main decision should be scale and output behavior, not backend architecture.

Keep the three-pane layout, persistent index, disabled folder support, background thumbnail workers, metadata overlay, duplicate detection, tag search, collection internals, and before/after comparison. These are the product's real strengths.

Add a compact grid mode or density control. This is the highest-value UI addition for large libraries. It does more for the stated use case than another AI backend or another metadata category.

Add explicit empty/loading states for smart views. Duplicates should say whether hashes are missing, scanning, or no duplicates found. Quality views should say whether metadata is indexed or being scanned. Search should say that it searches tags unless filename search is added.

## Top Priorities
- Top 3 problems
- The default UI exposes advanced product concepts before core triage actions.
- Browsing density is too low for the large-library use case despite good virtualization internals.
- AI/upscale/quality workflows are technically substantial but presented with unclear labels and too much hidden state.

- Top 3 improvements
- Reframe the default app around fast browse, preview, search, and file actions; move advanced organization and analysis behind progressive surfaces.
- Add compact grid/density controls and consistent counts/status for smart views.
- Rename and explain quality scoring, then simplify upscaling into a default contextual action with advanced backend details hidden.
