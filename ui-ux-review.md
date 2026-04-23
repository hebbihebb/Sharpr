# Sharpr UI/UX Review

## Summary
Sharpr reads as a GNOME-native image library browser with quality filtering and lightweight management tools. The core three-pane structure is understandable: folders and smart views on the left, thumbnails in the middle, preview on the right. The app looks more like a management console than a viewer, which is acceptable for large compressed-image libraries but risky if viewing is supposed to be the primary action. The current UI is clean at a component level, but the product direction already looks crowded: folders, smart folders, quality scoring, tags, collections, search, preview metadata, and upscaling all compete for attention. AI features are visible mainly through quality scoring and an upscale action, but the workflow is not self-explanatory from the screen alone. The app should not chase Lightroom-style depth; its strength is fast triage of messy image folders. The strongest direction is a focused browser for finding, inspecting, filtering, and improving compressed images, with AI tools kept contextual and optional.

## First Impressions
The first read is "image library triage tool", not generic image viewer. That is a good niche. The folder list, smart folders, quality categories, collections, and thumbnail strip make the app feel built for sorting through many files rather than editing one image.

The purpose is mostly understandable, but the hierarchy is too busy for a first launch. The left sidebar exposes too many concepts at once: physical folders, smart folders, quality buckets, collections, search, tags, and duplicates. None of these are individually wrong, but together they make the app look larger and more complicated than the primary use case requires.

The preview area is visually dominant, which is good. The selected image is clear. The floating metadata chips are useful, but they currently feel like product-specific jargon: "IQ 55%", "Fair", and the five-bar indicator need context somewhere, especially because users will not know whether this is objective quality, compression damage, resolution quality, aesthetic rating, or AI confidence.

## UI/UX Evaluation
The three-pane layout is the right base for this product. Sidebar navigation plus thumbnail browser plus large preview matches the job: browse many images quickly, inspect one image deeply, and move through collections without opening a separate viewer.

Spacing is generous and GNOME-like, but the middle thumbnail column is too wide for a list and too narrow for a serious grid. It shows large thumbnails with filenames, but it wastes vertical capacity. For large libraries, this becomes a slow browsing surface because the user sees only a few items at once. A density control or real grid/list toggle matters more here than more features.

The left sidebar has weak information hierarchy. "Folders", "Smart Folders", "Quality", and "Collections" are clear section labels, but the screen gives them nearly equal weight. Quality categories such as Excellent, Good, Fair, Poor, and Needs Upscale look like navigation destinations, not filters. If they are filters, they should behave and look like filters. If they are saved smart views, they need counts and clearer grouping.

The top header is sparse but slightly ambiguous. "Photos" in the thumbnail pane does not explain whether this is all photos, the selected folder, or a mode. "Preview" is accurate but redundant when the right pane is obviously a preview. The menu and close buttons are visually clear, but the app's most important actions are hidden. For this product, actions like compare, reveal file, tag, delete, copy path, open externally, and upscale should be discoverable from the selected image context, not buried in a menu.

The bottom overlay chips are visually strong, but they compete with the image. The left chip "1080p · webp" plus a plus button is cryptic. A plus symbol next to format/resolution suggests adding something, but it is not clear whether it adds to collection, starts upscale, creates variant, or opens actions. The right chip is better because it groups resolution, format, size, and quality, but it is large enough to obscure image content and feels like a status widget rather than a clean metadata readout.

The light theme exposes more problems than the dark theme. The image preview sits in a large white canvas, making empty space dominate the view and reducing the sense that this is an image-focused app. Dark mode feels more natural for image inspection. Light mode needs a more deliberate neutral preview background, likely not pure white.

GNOME alignment is mostly good: sidebar, headerbar, rounded controls, restrained icons, and preference for simple surfaces. The biggest mismatch is product complexity. GNOME apps tend to hide advanced behavior until needed. Sharpr currently presents advanced library concepts up front.

## Feature Scope Analysis
Core features are folder browsing, thumbnail scanning, large preview, file metadata, search, duplicate detection, basic collections, and quality filtering. These all support the stated use case of managing large compressed-image collections.

Quality scoring can be core if it is framed as compression/technical quality, not general image quality. The label "IQ" is a problem. It sounds proprietary and vague. Use clearer language such as "Quality 55%" or "Compression quality: Fair" if that is what it means.

Upscaling should be an advanced contextual action, not a primary navigation category. "Needs Upscale" in the sidebar is useful as a smart folder, but it also makes the app feel like an AI upscaler first. The app should let users ignore AI completely without feeling like they are using half a product.

Tags and collections are useful but dangerous. If both exist, they must have sharply different purposes. Tags are flexible metadata. Collections are curated sets. If the UI does not explain that through behavior, users will treat them as duplicate organizational systems.

Comparison tools are likely justified, but only if focused on before/after and duplicate/near-duplicate decisions. Do not build a general compare workspace. The product needs quick "is this better than that?" decisions, not a full review suite.

The current scope is close to overbuilt. The visible concepts already include enough surface area for an entire photo manager. The app should avoid adding editing, ratings, albums, timeline views, map views, face/person features, color correction, batch transforms, and DAM-style metadata depth.

## Workflow Analysis
Browsing is structurally sound but too low-density. Large previews in the thumbnail column make sense for visual triage, but the app needs faster scanning modes. A user managing thousands of screenshots or web images needs to see dozens of items at once, jump by folder, and filter without losing context.

Viewing is the strongest workflow. Selecting a thumbnail and seeing a large preview is immediate. The preview pane should provide obvious image navigation, zoom behavior, fit modes, and keyboard affordances. From the screenshots, those controls are not discoverable.

Comparing is not visible enough to evaluate as a complete workflow. If comparison exists, it should be reachable from the selected image and from duplicate/quality views. The expected flow is select image, compare with original/upscaled/duplicate candidate, accept or reject. Anything heavier will slow the product down.

Upscaling is hinted at but unclear. "Needs Upscale" and the plus button suggest an upscale path, but the screen does not explain what will happen: which model, output folder, overwrite behavior, queue status, or before/after review. For a destructive or expensive operation, the workflow needs more explicit staging.

File management actions are underexposed. For this audience, delete, move, reveal in files, copy, rename, and open externally are not secondary luxuries. They are part of triage. If these are only in a menu, the app will feel like a viewer pretending to be a manager.

## Edge Cases
Large libraries will stress the current thumbnail column. Seeing five or six items at a time is not enough. The app needs virtualization, progressive loading, persistent indexes, visible counts, quick filters, and a compact grid mode.

Large images need predictable zoom and memory behavior. The UI should make it obvious whether the preview is fit-to-window, actual size, or scaled. Without this, quality review becomes unreliable because users cannot tell whether blur is in the file or in the viewer scaling.

Slow hardware makes AI and quality analysis risky. Background jobs must be explicitly queued and cancellable, with visible progress that does not block browsing. The UI should never imply that AI analysis is required before basic browsing works.

Users who do not care about AI features should be able to use the app as a fast image browser without seeing constant AI terminology. Quality and upscale views should be collapsible or disabled. AI features should not occupy prime navigation space by default unless a library has actually been analyzed.

Users focused only on AI features need a tighter path than the current screen suggests. They need an "images that would benefit from upscale" queue, before/after comparison, output settings, and batch review. They do not need all library organization features visible while doing that job.

## Product Direction
The sustainable direction is not "GNOME Lightroom with AI". That will collapse under scope, performance, and UX complexity. The sustainable direction is "fast GNOME image triage for messy folders, with optional quality analysis and upscaling."

Sharpr should simplify the default experience and make advanced views conditional. Physical folders, thumbnails, and preview should be the main product. Smart folders, quality buckets, collections, tags, duplicates, and upscaling should appear as supporting tools, not equal peers.

The app does not need to be split into separate applications yet. It does need internal product boundaries. Core browsing should work without AI. AI improvement should be a mode or workflow. Library organization should remain lightweight.

If comparison and upscaling grow much further, they should become a focused workspace inside the app rather than more sidebar entries. A separate tool is only justified if upscaling becomes batch-processing software with models, presets, queues, and export management as the main identity.

## Recommendations
Remove visible concepts that are not ready to earn their space. Do not show empty or low-value sections by default. Hide collections unless the user creates one. Hide quality buckets until analysis exists. Do not expose AI terminology in the base navigation.

Simplify the sidebar. Default it to Folders, Search, and maybe Duplicates. Move Tags, Quality, Needs Upscale, and Collections behind collapsible sections or an "Organize" / "Analyze" mode. Add counts consistently if sidebar entries represent filtered result sets.

Move upscaling to an advanced contextual workflow. The selected image should have a clear "Upscale" action with a confirmation/options sheet, queue feedback, and before/after comparison. The sidebar "Needs Upscale" view can stay, but only as an analyzed smart view.

Keep the three-pane layout, dark-theme image inspection, large preview, smart folder concept, duplicate detection, and basic metadata overlay. These are aligned with the product.

Add a compact grid mode for browsing large libraries. Add visible zoom/fit controls in the preview. Add clear selected-image actions: reveal in folder, delete, tag, add to collection, compare, upscale, and open externally. Add a plain-language explanation of quality scoring in a tooltip, popover, or first-run analysis prompt.

## Top Priorities
- Top 3 problems
- The sidebar exposes too many product concepts at once, making the app feel unfocused.
- Browsing density is too low for large image libraries.
- AI/quality/upscale features are visible but not clearly explained as workflows.

- Top 3 improvements
- Make the default UI a focused folder browser with preview, search, and essential file actions.
- Add compact grid/list browsing and explicit preview zoom/fit controls.
- Turn quality analysis and upscaling into optional advanced workflows with clear queue, output, and comparison states.
