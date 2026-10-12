# Sharpr GTK And Libadwaita Reference

## Design Direction

Sharpr should feel like a calm, fast GNOME review desk. The primary workflows are:
- folder review in the viewer and filmstrip
- buckets and manual tags
- search and filters
- Compare
- Resize, Crop, and WebP copies
- explicit trash

## UI Rules

- Prefer standard GTK and Libadwaita widgets, actions, dialogs, popovers, overlays, and accessibility behavior over custom UI.
- Use `GAction`s for menu commands and their sensitivity. Compare is enabled only when exactly two filmstrip images are selected.
- Search, filters, and sort apply to the current folder or bucket unless the user explicitly picks the whole library. Clearing a search restores the scope, selection, scroll position, and current sort.
- Names & tags is the default search mode. Experimental visual search must never replace it by default.
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
