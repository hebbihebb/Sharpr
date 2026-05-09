# Sharpr Accessibility and Keyboard Navigation Audit

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Report orientation: practical GTK/libadwaita accessibility and keyboard-only review

## Executive Summary

Sharpr already has a meaningful keyboard foundation: a shortcuts overlay, README shortcut table, viewer shortcuts, window-level navigation shortcuts, tooltips for many icon buttons, and GNOME-native GTK/libadwaita widgets. The biggest accessibility risks are in custom/composed widgets, focus order across the multi-pane shell, color-only quality/status cues, and destructive or modal workflows that may not be comfortable without a mouse.

The first accessibility goal should be simple: a user should be able to open a folder, move through the sidebar/filmstrip/viewer, inspect metadata, tag an image, compare images, run/export tasks, and recover from dialogs using only the keyboard.

## Owner Decisions / Product Corrections

- Keyboard navigation/accessibility is core, not polish.
- The filmstrip is a defining feature and must be comfortable without a mouse.
- Collections and Tasks are central workflows and need first-class keyboard paths.
- Compare/task-result views behave like virtual folders by populating the filmstrip and need predictable focus/selection behavior.
- Manual QA is wanted; keyboard-only QA should be part of the standard checklist.

## Biggest Accessibility Risks

- Focus order is likely accidental in parts of the main shell because the app is code-built rather than template-driven.
- Custom drawing appears in color swatches and overlay widgets; these may need explicit labels or accessible roles.
- Quality scoring is purely resolution-based with text tier labels ("720p or lower", "1080p", "4K+", etc.) already shown. Other status indicators (task state, errors) should also have text alongside any color coding.
- Icon-only buttons often have tooltips, but tooltips are not a full substitute for accessible names.
- Delete/trash is keyboard-accessible through `Delete`; destructive intent should be confirmed or undoable.
- Popovers/dialogs need consistent Escape behavior, default buttons, and initial focus.
- Compare, tasks, collections, and tag-edit flows may be harder to reach or operate without a mouse than the core viewer.

## Affected Files and Widgets

- `sharpr/src/ui/window.rs`: global shortcuts, shell, delete/trash, menus, folder and virtual-view wiring
- `sharpr/src/ui/viewer.rs`: viewer shortcuts, metadata/tag overlays, zoom controls, smart tag action
- `sharpr/src/ui/filmstrip.rs`: tile navigation and trash requests
- `sharpr/src/ui/sidebar.rs`: library/folder/collection navigation
- `sharpr/src/ui/compare_page.rs` and `compare_item.rs`: compare navigation and controls
- `sharpr/src/ui/tasks_page.rs`: task status rows and actions
- `sharpr/src/ui/preferences.rs`: advanced/upscale/network preferences
- `sharpr/src/ui/metadata_chip.rs`, `tag_card.rs`, `tag_browser.rs`, `filter_bar.rs`: custom or composed controls
- `sharpr/data/help-overlay.ui`
- `sharpr/data/manual.md`
- `README.md`

## First Manual Keyboard-Only QA Script

1. Launch Sharpr with compiled schemas:

   ```bash
   cd sharpr
   glib-compile-schemas data/
   GSETTINGS_SCHEMA_DIR=data cargo run
   ```

2. Without using the mouse, open the main menu and reach folder/library controls.
3. Open or select a folder from the sidebar.
4. Move focus into the filmstrip and select next/previous images.
5. Use `Alt+Left`, `Alt+Right`, `Ctrl+0` (fit to window), `F11`, and `Alt+Return`.
6. Open the shortcuts overlay with `?`, close it with Escape.
7. Open tag editing with `Ctrl+T`, add/remove a tag, and return to the viewer.
8. Open Duplicates and Quality views from the sidebar or menu.
9. Enter Compare, switch compared items, and exit Compare.
10. Open Tasks, inspect queued/completed/failed work, reach generated outputs, and return to the filmstrip.
11. Create or select a collection and verify the workflow works without pointer-only gestures.
12. Open Preferences, navigate every visible control, and close with Escape or the window close shortcut.
13. Trigger Delete on a test copy and verify confirmation/undo/failure behavior is understandable.
14. Repeat with high contrast and large text enabled in GNOME settings.

## What Can Wait

- Full screen-reader perfection for every custom widget can follow the first keyboard-only pass.
- Translations can wait until the app text stabilizes.
- Advanced shortcut customization is not necessary for v1.
- Automated accessibility testing can come later; manual GTK accessibility review is higher value first.

## Success Criteria

For the next release, Sharpr should pass a keyboard-only walkthrough for browsing, filmstrip navigation, collections, Tasks, generated outputs, comparing, exporting/upscaling, and preferences. Every icon-only control in the main workflows should have a tooltip and accessible name, and every color-coded state should also have text.
