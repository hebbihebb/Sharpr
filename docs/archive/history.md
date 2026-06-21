# Development History

This is a compact historical summary. It replaces the old `HISTORY.html` as the place for context that may help future agents understand how Sharpr got here.

## April 2026

- Initial GTK4/Libadwaita app scaffold and Rust project setup.
- Core folder browsing, filmstrip, viewer, metadata, and thumbnail loading arrived quickly.
- Background decode, metadata, quality scoring, persistent index, tags, collections, and duplicate detection were added.
- Local AI tag suggestions and upscale backends were explored.
- A key pattern emerged: when custom implementation became fragile, using existing GTK/GNOME/system/library capabilities usually produced better results.

## May 2026

- Focus shifted toward reliability, Tasks, compare, GNOME polish, and reducing stale-result bugs.
- `FocusedImageSet` became the conceptual model for non-folder filmstrip views.
- Context menus moved toward `GMenu` plus `GAction`.
- Preferences and dialogs moved closer to Libadwaita/HIG patterns.
- Several removed or deferred scopes were clarified: rotate/flip editing, ONNX downloader, sharpness backfill, splash screen, import workflow, saved searches, batch rename, and embedded metadata writing.
- The next major feature direction became external edit workflow and image families.

## Lessons

- Protect thumbnail and filmstrip responsiveness above speculative architecture work.
- Keep folders as truth and SQLite as support state.
- Add generation counters to stale-prone background flows.
- Use platform widgets and libraries before custom code.
- Keep agent docs short enough to be read and maintained.
