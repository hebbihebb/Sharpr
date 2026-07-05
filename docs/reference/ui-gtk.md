# Sharpr GTK And Libadwaita Reference

## Design Direction

Sharpr should feel like a calm, fast GNOME review desk. Folder review, filmstrip navigation, Compare, tags, collections, quality views, Resize Copy, and explicit trash are the primary workflows.

## UI Rules

- Prefer standard GTK and Libadwaita widgets, actions, dialogs, popovers, overlays, and accessibility behavior.
- Use actions for menu commands and sensitivity. Compare is enabled only when exactly two filmstrip images are selected.
- Compare uses neutral left/right image language and returns to the viewer when closed.
- Resize Copy exposes explicit width and height, starts locked to the oriented source aspect ratio, and disables quality for PNG.
- Keep the transient operation indicator informational; it does not navigate to a Tasks page.
- Every meaningful icon-only control needs an accessible name or tooltip.
- Keyboard-only navigation is an acceptance path.

## Threading

- Never decode full images, scan folders, hash, or resize on the GTK main thread.
- Never move GTK objects into worker threads.
- Use generation counters for stale-prone UI results.
- Preserve visible-thumbnail priority over preload work.

## Styling And Layout

- Prefer Adwaita style classes and theme variables.
- Keep image overlays compact and readable in light, dark, and high-contrast themes.
- Support the documented 1024 × 600 logical-pixel minimum.
