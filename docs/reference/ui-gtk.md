# Sharpr GTK And Libadwaita Reference

This is a short Sharpr-specific UI reference. It replaces the old broad GTK manual as the agent-facing guide. Use official GTK, Libadwaita, GNOME HIG, and gtk-rs docs when exact API behavior matters.

## Design Direction

- Sharpr should feel like a GNOME-native review desk: calm, fast, keyboard-friendly, and content-first.
- Prefer standard GTK and Libadwaita widgets. They bring accessibility, focus, keyboard, style, and adaptive behavior for free.
- Do not build custom widgets when a standard action row, preferences row, dialog, popover menu, toolbar, banner, status page, or split view is the right fit.
- Keep primary workflows efficient: folder review, filmstrip navigation, compare, Tasks, tags, collections, and trash.
- Avoid landing-page or marketing-style UI inside the app. Sharpr is a work tool.

## Widget Choices

- Preferences: use `AdwPreferencesWindow`, `AdwPreferencesPage`, `AdwPreferencesGroup`, and row widgets such as `AdwActionRow`, `AdwSwitchRow`, `AdwComboRow`, `AdwSpinRow`, and `AdwEntryRow`.
- Menus: prefer `GMenu` plus `GtkPopoverMenu` plus scoped `GAction`s for context menus.
- Dialogs: use Libadwaita dialog patterns. Avoid unnecessary Cancel buttons where close already means cancel.
- Empty states: use status-page style surfaces and concrete next actions.
- OSD overlays: keep image overlays compact and readable. Use Adwaita style classes where possible; image-surface OSDs may need stronger contrast than normal cards.
- Compare page: use the planned centered floating OSD toolbar, not the viewer metadata chip pattern.

## Actions, Menus, And Shortcuts

- Prefer actions over ad hoc button-only behavior for commands that need menus, shortcuts, or sensitivity rules.
- Keep action names, menu labels, shortcut help, README shortcuts, and in-app manual aligned.
- Do not start a broad action-bus refactor. Use actions where they naturally fit menus/shortcuts.
- Use icon-only controls only when the icon is conventional and the control has a tooltip/accessibility label.

## Accessibility

- Every meaningful icon-only control needs an accessible name.
- Keyboard-only navigation is an acceptance path, not a bonus.
- Do not make hover, precise pointer movement, or secondary click the only way to reach important functionality.
- Test dialogs, popovers, sidebar, filmstrip, viewer, Tasks, compare, and preferences with keyboard navigation.
- Keep text short, concrete, and user-facing.

## Threading And Responsiveness

- Never block the GTK main thread with decoding, filesystem scans, hashing, model loading, exports, upscaling, or network calls.
- Do not move GTK objects into worker threads.
- Use generation counters for stale-prone UI result flows.
- Preserve the thumbnail worker's visible-first scheduling and preload behavior.

## Styling

- Prefer Adwaita style classes and CSS variables.
- Avoid hardcoded colors except for image-overlay contexts where contrast requires it.
- Check light, dark, and high-contrast behavior for UI surfaces that add CSS.
- Keep minimum window size support aligned with the documented 1024 x 600 logical pixel target.

## Research Rule

Before building custom UI infrastructure, check whether GTK, Libadwaita, GIO actions, portals, or an existing crate already solves the problem. Use custom code for Sharpr workflow behavior, not for platform primitives.
