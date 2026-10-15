# Interface inventory and control dispositions

Phase 9's durable control map. The private Claude map originally counted 83
controls in ten areas; its user marks are preserved in `ROADMAP.md`. This map
uses the tracked marks, the pre-review source at `b7241f7`, and the current GTK
surfaces. The rows below group controls that share one action or disposition;
they are not the private artifact's control IDs. Newly added controls are
included. `interface_tour_screenshots` and the focused GTK workflows supply
minimum/wide screenshots; screenshots remain local under `.tmp/phase9/`.

Decisions D1–D33 are in `product.md`. Kept means the behavior is retained;
changed means the final entry point below replaces the old presentation;
removed means the control is absent, with stored user data retained.

| Area | Control or group | Disposition and final home | Decision |
| --- | --- | --- | --- |
| Window | Library and Photos titles | Removed; column layout, Folders and Buckets sections explain the panes | D27 |
| Window | Sidebar toggle / F9 | Kept above the filmstrip | D27 |
| Window | Main menu | Changed: fixed position at the viewer's right edge; Library, Preferences, Help | D27, D33 |
| Window | Search button / Ctrl+F | Changed to the always-placed search entry; typing focuses it | D6, D27 |
| Window | Close/maximize/minimize | Kept, following the desktop's button layout | D26 |
| Window | Background indexing ring | Removed; progress remains in Library Overview | D30, D33 |
| Window | Transient operation indicator | Kept for operations that last; does not navigate | D27 |
| Window | Presentation menu item | Removed; double-click and F11 retain fullscreen | D11, D27 |
| Filmstrip | Thumbnail list, labels and index pills | Kept, virtualized; placeholders and hover/press feedback use CSS | D27, D30 |
| Filmstrip | Small scrollbar handle | Changed to a wider, always-available native handle | D30 |
| Filmstrip | Single selection / Ctrl-click / Shift-click / Ctrl+A | Kept; explicit action selection is visible, including one survivor | D25 |
| Filmstrip | Scroll, row activation and arrow navigation | Kept; visible rows retain thumbnail priority and generations | D27, D30 |
| Filmstrip | Separate Sort and Filter controls | Changed to one Sort and Filter menu above the strip | D27 |
| Filmstrip | Name/date/type/aspect/closest-to-16:9 sorts | Kept in that menu; closest-to-16:9 remains a sort | D27 |
| Filmstrip | Ascending/Descending | Kept; boolean action targets, with matching state | D27 |
| Filmstrip | Independent old Filter popover | Removed; menu entries add the same chips as typing | D27 |
| Filmstrip | Minimum width/height spin controls | Removed; resolution chips provide the approved size filters | D12, D16, D27 |
| Filmstrip | Landscape/portrait/square/16:9 filters | Changed to visible removable chips | D16, D27 |
| Filmstrip | Negative, narrower and wider aspects | Kept/added as -16:9, <16:9 and >16:9 chips; menu shares parser | D30 |
| Search | Typed words | Kept: filename matches and optional local visual ranking | D6, D22 |
| Search | Format and resolution chips | Kept; filters combine before ranking | D16 |
| Search | Minus words / strict minus words | Kept; visible chips explain exclusions | D16, D22 |
| Search | Chip edit/removal controls | Kept; click converts to text, × removes | D16 |
| Search | Suggestions | Kept below the same entry | D16 |
| Search | Whole library | Kept contextual: explicit broadening is a product requirement | D6 |
| Search | Coverage/strength/unavailable status | Kept with results, never mistaken for an empty read | D22 |
| Search | Show More | Kept only when another result page exists | D22 |
| Search | Add results to bucket | Kept; explicit selection first, otherwise choose strong/all shown | D5 |
| Search | Saved-search controls | Removed from the active UI; buckets keep chosen results | D5 |
| Folders | Open Folder button | Removed; Open Library Root changes the single root | D32 |
| Folders | Library switcher / create-library menu | Removed; inactive configurations remain stored | D11, D32 |
| Folders | Library list / names / root / top-level-versus-drill-down editor | Changed to one root action; identity and legacy options survive | D32 |
| Folders | Folder rows and disclosure buttons | Kept; recursively expose image-containing subfolders | D32 |
| Folders | Enable/disable folder | Kept; disabled rows can be enabled without scanning their children | D32 |
| Buckets | Bucket rows and counts | Kept, scoped to the active identity and enabled root | D3, D4 |
| Buckets | Nested disclosure buttons | Kept, with inherited parent membership | D4 |
| Buckets | New Bucket (+) / name / parent / colour | Kept in the compact editing sheet | D27 |
| Buckets | Rename / change colour | Kept in the bucket context menu; originals are untouched | D3 |
| Buckets | Move under parent / promote / move children | Kept in the bucket context menu with cycle checks | D4 |
| Buckets | Delete bucket | Kept with effect/confirmation; image files stay | D3 |
| Buckets | Drag images onto a bucket | Kept; same background writer as picker and repeat | D5, D31 |
| Buckets | Add to Bucket picker / New Bucket inside picker | Kept as a compact click-away picker | D25, D27 |
| Buckets | Add to last bucket / Ctrl+Shift+B | Added, named in image menu and persisted with library identity | D31 |
| Buckets | Import/export controls | Removed from Preferences; membership data stays intact | D11, D32 |
| Image menu | Filmstrip right-click / Shift+F10 / Menu key | Kept, compact, with the current image/selection context | D21, D27 |
| Image menu | Viewer right-click | Added using exactly the filmstrip menu and action group | D30, D33 |
| Image menu | Info / Ctrl+I | Kept; expands/pins the same readout card | D31 |
| Image menu | Open in default viewer / Return | Kept | D21 |
| Image menu | Show in File Manager | Kept | D21 |
| Image menu | Copy to Clipboard / Ctrl+C | Kept; Compare has its own shortcut | D21, D29 |
| Image menu | Crop Copy | Kept for one image, in the viewer | D28 |
| Image menu | Export as WebP / Ctrl+E | Kept for one or many images | D13 |
| Image menu | Add / remove bucket membership | Kept in the shared menu and writer | D5, D31 |
| Image menu | Copy/Move to Folder | Kept under File Actions; disabled while reviewing/cropping | D21, D28 |
| Image menu | Rename/F2 and Duplicate/Ctrl+D | Kept under File Actions for one image | D21 |
| Image menu | Move to Trash/Delete | Kept with immediate system trash and Undo; no source overwrite | D25 |
| Viewer | Scroll zoom / drag pan / Ctrl+0 fit | Kept | D27 |
| Viewer | Double-click / F11 fullscreen | Kept; contextual mode controls survive fullscreen | D11, D28 |
| Viewer | Fit/1:1 entries in main menu | Removed; Ctrl+0 and contextual Compare controls remain | D30, D33 |
| Viewer | Show Overlay switch / Alt+Return | Removed; legacy preference value remains stored unused | D30, D33 |
| Viewer | Metadata pill and separate Info pill | Changed to one movement readout that expands on hover or pins by click/key | D27, D31 |
| Viewer | Resolution/format/below-4K/buckets readout | Kept; other metadata is in expanded Info | D18, D31 |
| Viewer | Filename/location/dates/size/EXIF/resolution score | Kept in expanded Info; Escape closes it before fullscreen exits | D18, D31 |
| Viewer | Canvas | Kept neutral in light, dark and high contrast | D27 |
| Compare | Context action / Ctrl+Shift+C | Kept/added for at least two images, guarded while typing or in modes | D29 |
| Compare | Side by Side / Split / Flip | Kept in the contextual top bar | D28 |
| Compare | Linked zoom / Fit / 100% | Kept in the same bar | D28 |
| Compare | Split divider / zoom / pan / Space flip | Kept | D8, D28 |
| Compare | Left/right image dropdowns | Replaced by a horizontal candidate row and Replace Left/Right target | D29 |
| Compare | Candidate badges, filenames and details | Kept; L/R identify the current pair | D29 |
| Compare | Keep Left/Keep Right / old [ and ] keys | Changed to labelled Keep buttons and arrow keys; old dismiss keys removed | D28 |
| Compare | Count / Undo / Done | Kept with simple remaining count; Ctrl+Z and Escape | D28 |
| Survey | Grid / dismiss × / Delete / arrow navigation | Added for narrowing a selection without file changes | D30 |
| Survey | Undo / Ctrl+Z / Done / Escape | Added; restores original order and returns survivors selected | D30 |
| Crop | Small preview dialog | Replaced by the full viewer canvas | D28 |
| Crop | 16:9/Free / live dimensions / destination | Kept in the contextual bar | D28 |
| Crop | Drag selection/corners and key hints | Kept; hints below canvas | D28 |
| Crop | Cancel/Escape and Save Copy/Enter | Kept; unique lossless WebP, original untouched | D28 |
| Export | Folder / chooser / Wallpapers | Kept in one in-window sheet through native chooser Select/Cancel | D13, D25 |
| Export | Lossless / lossy quality / optional fit dimensions | Kept; only downscales | D13 |
| Export | Export / Cancel / Escape / click-away | Kept; one start path and cancellation path | D25 |
| Export | Progress/cancel/result notice | Kept; completed outputs survive cancellation, partials do not | D13 |
| Copies | Find Copies / progress / close | Kept in Library; exact and indexed resized/converted copies | D14 |
| Copies | Keep/tick controls / Compare / Done | Kept; Compare returns to the caller's decisions | D8, D14 |
| Copies | Move to Trash / whole-group warning | Kept with explicit confirmation and changed-file protection | D14, D25 |
| Library Overview | Library/format/low-resolution/disabled counts | Kept in Library Overview | D22 |
| Library Overview | Fingerprints/copies/buckets/missing counts | Kept; moved out of Preferences | D22, D33 |
| Library Overview | Index progress/device/model/rebuild/open model folder | Kept; rebuild still confirms | D22, D33 |
| Preferences | Multiple libraries / bucket transfer / overlay / cache rows | Removed; values and user data remain stored | D32, D33 |
| Preferences | Library root and Use Graphics Card | Kept, using existing window actions/settings | D32, D33 |
| Help | Separate Manual/Shortcuts/About main-menu entries | Changed to one Help sheet; existing actions remain shared | D30, D33 |
| Help | Automatic-tag cleanup / tags-to-buckets entries | Moved from main menu to Help → Library Maintenance; confirmations/backups stay | D17, D33 |
| Help | Sheet close / Escape / keyboard navigation | Kept; no extra application windows | D26 |

## Verification evidence

- GTK regression tests cover the shared viewer menu/action target, Help→Shortcuts,
  retired metadata preference remaining inert, seven bundled icons, root data
  preservation, Compare/Survey survivors and unchanged originals, export chooser
  Select/Cancel and output bytes, minimum/wide layouts and fixed menu placement.
- `data/style.css` is the only static stylesheet. `ui/look.rs` owns the sole
  application provider and appends only validated user-colour rules.
  `palette.rs` owns the bucket palette and Cairo/snapshot ink tokens.
- Icons are original CC0 SVGs; source/licence: `sharpr/data/icons/README.md`.
- Light/dark and forced high-contrast captures use the private off-screen
  compositor. Forced contrast is checked through `AdwStyleManager:high-contrast`.
- The final manual pass is still required for acceptance. Automated checks
  establish implementation/verification, not the user's verdict.
