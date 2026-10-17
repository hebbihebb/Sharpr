# Sharpr GTK And Libadwaita Reference

## Design Direction

Sharpr should feel like a calm, fast GNOME review desk. The workflows and the eight UX principles are in `product.md`:
- find
- curate in buckets
- prepare wallpapers
- clean up
- compare
- view fullscreen

This file covers how to build them in GTK.

## UI Rules

- Prefer standard GTK and Libadwaita widgets, actions, dialogs, popovers, overlays, and accessibility behavior over custom UI.
- Use `GAction`s for menu commands and their sensitivity.
- Compare accepts N ≥ 2 selected images and shows two at a time (D8). Left/Right arrows and Keep Left/Keep Right retain that side and dismiss the other; Undo restores the last choice. Compare never changes files (D28).
- Search, filters, and sort apply to the current folder or bucket unless the user explicitly picks the whole library. Clearing a search restores the scope, selection, scroll position, and current sort.
- Open search by typing anywhere or pressing Ctrl+F (D7). Keep Space free for viewing and Compare.
- Recognized filter words in the search box become removable chips in a wrapping bar under the header (D16). The bar wraps so it never widens the window. The combined Sort and Filter menu adds these same chips (D27). Clicking a chip's label turns it back into a quoted search word. Filter words that complete the word being typed are offered as flat buttons in the same bar, rather than in a popover, which would take focus from the entry.
- Words in the search box go to visual search when its model is installed (Phase 5). File-name matches come first, then the closest images, 200 at a time. A status line under the header always says what happened: match strength and coverage ("4,210 of 5,812 images indexed"), or that only file names were matched and why. Never show weak matches as if they were good ones (principle 5).
- Tags are retired (D1, Phase 5). Add no tag UI; buckets are how images are gathered.
- Keep the interface quiet. Only resolution with format, and bucket membership, are always visible; everything else belongs in the Info panel (D18).
- Ctrl+G or image menu → 16:9 Guide toggles a view-only guide (D36, D38): a non-targetable `GuideLayer` (`ui/guide_layer.rs`, drawn with GPU-composited snapshot nodes rather than a Cairo surface, so dragging stays smooth) in the viewer overlay outlines a 16:9 rectangle with corner squares and dims the rest of the image. Until moved it is the largest centred 16:9 inside the image as placed at Fit (`viewer::guide_rect`), so zoom and pan leave it in place; a moved guide is kept in overlay coordinates through zoom and image changes, clamped to the viewport on resize, and showing the guide again refits it. A capture-phase `GestureDrag` on the overlay claims presses on a corner (resize along the diagonal, 16:9 kept) or the outline (move) and denies the rest, so the scrolled window's pan gesture and the corner chip still get them; the cursor shows the grab. Geometry helpers are `viewer::guide_grab`, `guide_resized`, `guide_moved`, `guide_within` and `guide_frame`. While it is shown, Crop Copy of the viewed image, or Ctrl+Shift+G, passes `guide_frame` to `crop_mode::start`, which starts from `CropRect::framed`. It is off at start and changes no files.
- Phase 9 expands Info inside the same readout card after a short hover. Leaving collapses it; clicking or Ctrl+I pins it until Escape or another click. Clearing/changing the image retires delayed hovers. No second popover appears.
- Ctrl+Shift+B repeats the last successful bucket destination, scoped to the library and persisted additively. The image menu names that destination. Picker, drag/drop and repeat use the same background membership writer; successful writes refresh the sidebar, badges and current image's membership summary.
- Destructive or broad actions need confirmation that states their effect. These include deleting a tag, deleting a bucket, trashing files, and bulk tag removal. When an action affects buckets, name them.
- Every background failure surfaces as a toast or dialog. Never let a failed read look like an empty result.
- Copy dialogs keep the oriented source aspect ratio by default. Crop defaults to a movable 16:9 selection. PNG output disables the quality control.
- The operation indicator is informational and does not navigate anywhere.
- Every meaningful icon-only control needs an accessible name or tooltip.
- Keyboard-only navigation is an acceptance path. Delete and other single-key shortcuts must not fire while a text entry has focus.

## Threading

See `architecture.md`. In UI code this means:
- Never decode, scan, hash, read headers, query SQLite in bulk, or touch files in a callback.
- Never move GTK objects into worker threads.
- Apply worker results only after checking a generation counter.

## Styling And Layout

- Prefer Adwaita style classes and theme variables. Overlays must stay readable in light, dark, and high-contrast themes.
- Support the 1024 × 600 logical-pixel minimum window size.

## Approved Look (D26–D27)

- Libadwaita supplies adaptive sheets, theme/accent handling, and toasts. Sharpr's shared stylesheet is `sharpr/data/style.css`; `sharpr/src/ui/look.rs` owns its single provider and validated dynamic colour rules; the main window has `.sharpr` and `.sharpr-mono-all`.
- `sharpr/data/fonts/` contains the official JetBrains Mono v2.304 variable fonts and OFL licence, embedded in GResource. `ui/fonts.rs` registers them once with Pango (≥ 1.56), using process-owned anonymous files held open for later font reads. No desktop font installation or user-cache extraction is needed. CSS prefers `JetBrains Mono`; the platform monospace face remains a fallback. About includes the font licence.
- Preferences and Keyboard Shortcuts are `AdwPreferencesDialog` and `AdwShortcutsDialog` sheets presented in the main window. They inherit its font and use `.sharpr-sheet` for compact rows and headers. Keep the existing `win.show-preferences` and `win.show-help-overlay` actions.
- Quick pickers use popovers and dismiss on click-away. Preferences and other editing sheets use their own close controls and Escape.
- WebP export uses a compact `AdwDialog` with `.sharpr-sheet`, sharing the main window's native surface. Keep that same form mapped throughout Select/Cancel in the native folder chooser; never recreate a grabbing popup on return. D25's click-away cancellation uses the earlier window capture handler, scoped to export and removed when the sheet closes. Cancel, Escape and the close button also cancel; only Export starts work.
- One Sort and Filter menu adds the same removable chips as typing filter words. The old independent Filter popover and its separate filtering state are removed; chip matching is the single filtering path. Displayed dimension caching and aspect sorting remain on the browse worker.

## Contextual Viewer Header (D28)

- `ui/window/mode_header.rs` swaps the existing header's search content for Compare/crop controls. The app menu and window buttons stay in place. Search text, chips, scope and status widgets stay alive; pending browse results are retired when entering a mode.
- Compare's header contains the viewing modes, Linked zoom, Fit/100%, Undo and Done. Each image's filename, details and Keep button sit below the canvas. D29 replaces the dropdowns with a compact horizontal candidate row: L/R badges, an explicit Replace Left/Right target defaulting to Right, and a quiet remaining count. Choosing changes that side without dismissing either image; selecting the opposite image swaps sides. Keep removes the loser and Undo restores its original position.
- The candidate row is a horizontal `GtkListView`. Its visible rows request thumbnails through one serial worker per Compare session using the existing thumbnail cache/decoder. No decoding or filesystem reads happen during binding. Session generations reject late results; recycled rows are updated through their current model item. Candidate-row arrows and Space keep GTK's navigation/activation behaviour instead of choosing a winner; canvas/header arrows still keep that side.
- Ctrl+Shift+C opens the existing Compare action for at least two selected images; text fields, popovers, modal sheets, crop preparation, active crops and an active Compare retain their keys. The context menu and Shortcuts sheet show the binding. Ctrl+C remains Copy.
- Crop's selection draws on the same snapshot `GuideLayer` as the viewer guide (shared `guide_layer::dim`, `outline`, `handles`), and its 16:9 corner resize follows the drag along the diagonal (`CropRect::resized`), as the guide does.
- Crop zooms and pans (D39). The same `GuideLayer` also draws the image, scaled by a snapshot texture node under a clip, so there is no `Picture`; the layer covers the whole viewer, and the key hints float over it. The view is a `CropView` (`ui/crop_view.rs`: zoom times fit, and the image point at the canvas centre), so a resize keeps the centre and refits, and the selection (`CropRect`) is in image pixels and never moves with the view. It opens on the viewer's scale and centre (`ViewerPane::view_state`, read from the picture's real bounds). The side panels slide away as it opens and widen the canvas, so the scale and centre carry over rather than screen pixels, and they are held (`Crop::opening`) through that settling until the first zoom or pan; only then does the view follow the canvas as zoom-times-fit. Scroll zooms at the pointer in viewer steps (1.1×, to the viewer's 20× reach); dragging outside the box, or the middle button anywhere, pans. While a crop is open the viewer's own scroll zoom stands down (`ViewerPane::set_crop_open`). A preview (the viewer's picture, or a 2560 px worker decode) is swapped for the full decode, on a worker, once the view needs more pixels than it holds (`crop_view::needs_full_resolution`); a failure keeps the preview and says so.
- Crop's header contains aspect, live output dimensions, destination, Cancel and Save Copy; zoom/pan/drag/Enter/Escape hints float at the bottom of the canvas. The destination label ellipsizes and its tooltip names the full path.
- Mode keys are routed from the window so header focus works. Text fields, picker popups and modal sheets keep their own keys. Ctrl+F and type-to-search leave an active mode alone. Fullscreen retains the active mode controls.

## Survey (D30)

- Select two or more images, then Ctrl+Shift+S or image menu → Survey Selected Images. A virtualized adaptive GtkGridView replaces browsing, with a contextual remaining count, Undo and Done. Delete dismisses the selected tile; Ctrl+Z restores the last dismissal in original order; Escape finishes with survivors selected. Dismissal changes no files or bucket memberships. Search and chips return unchanged.
- Survey and Compare share a serial viewport-fed thumbnail lane through the existing cache/decoder, with session generations and recycled-row checks. Filmstrip pools and visible-thumbnail priority stay unchanged. File actions and search are guarded during review.

## One library root (D32)

- Main menu → Library → Open Library Root and Preferences → Library Root activate the same window action. The native folder chooser checks directory availability on a worker. Applying the choice retains the active identity and saved disabled paths, retires browse/thumbnail generations, clears the old view, refreshes the sidebar, and restarts scoped indexing/fingerprints. The folder tree has its own generation so retired scans cannot reopen an old root, and the scan itself is told to stop.
- Folder browsing is recursive regardless of the stored legacy folder mode. Disabled rows remain available for enabling, without scanning their children. Inactive library settings remain serialized, without switch/create controls. Buckets and fingerprint/vector rows are not migrated or deleted by changing the root.

## Shared style, image actions and Help (D33)

- `data/style.css` holds all static rules, with theme-derived surfaces and neutral image canvases. `palette.rs` is the single bucket palette and drawing-token source. `look.rs` appends only parsed/canonicalized user-colour rules to the same provider. No widget installs its own provider. High contrast adds a class from StyleManager, including explicit OSD edges and readable secondary text. GTK's [CSS variables](https://docs.gtk.org/gtk4/css-properties.html) and Libadwaita's [appearance support](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1-latest/styles-and-appearance.html) supply the platform behavior.
- Namespaced symbolic SVGs are embedded under the icon resource path; `data/icons/README.md` records original source and CC0 licence. Standard actions keep system icons. Thumbnail placeholders and hover/press feedback are static CSS, with no scheduler change or animation.
- The viewer's right-click menu uses the filmstrip's exact menu builder, context and SimpleActionGroup. Its popover is explicitly unparented on viewer disposal. Shift+F10/Menu falls back to the viewer when the current filmstrip row is unavailable, including fullscreen. File actions remain guarded during review/crop.
- Preferences keeps root/GPU settings. The retired overlay/cache controls leave their settings stored; the readout is contextual regardless of the legacy metadata-visible value. Index/fingerprint status lives in Library Overview. Main menu → Help offers Shortcuts, Guide, About and the two legacy tag maintenance actions, reusing existing actions and their confirmation/backup behavior.
- `interface-inventory.md` records every active/retired control group and its governing decision. The source map supplements the private original map without inventing its control IDs.
