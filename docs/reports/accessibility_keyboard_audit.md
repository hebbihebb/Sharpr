# Sharpr Accessibility and Keyboard Navigation Audit

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Report orientation: practical GTK/libadwaita accessibility and keyboard-only review

## Executive Summary

Sharpr already has a meaningful keyboard foundation: a shortcuts overlay, README shortcut table, viewer shortcuts, window-level navigation shortcuts, tooltips for many icon buttons, and GNOME-native GTK/libadwaita widgets. The biggest accessibility risks are in custom/composed widgets, focus order across the multi-pane shell, color-only quality/status cues, and destructive or modal workflows that may not be comfortable without a mouse.

The first accessibility goal should be simple: a user should be able to open a folder, move through the sidebar/filmstrip/viewer, inspect metadata, tag an image, compare images, run/export tasks, and recover from dialogs using only the keyboard.

## Biggest Accessibility Risks

- Focus order is likely accidental in parts of the main shell because the app is code-built rather than template-driven.
- Custom drawing appears in color swatches and overlay widgets; these may need explicit labels or accessible roles.
- Some status is color-coded, especially quality scoring in the metadata overlay/manual, and should also be represented in text.
- Icon-only buttons often have tooltips, but tooltips are not a full substitute for accessible names.
- Delete/trash is keyboard-accessible through `Delete`; destructive intent should be confirmed or undoable.
- Popovers/dialogs need consistent Escape behavior, default buttons, and initial focus.
- Compare, tasks, collections, and tag-edit flows may be harder to reach or operate without a mouse than the core viewer.

## Top 10 Keyboard and Accessibility Fixes

1. Define and test a primary focus order.

   Target order: sidebar, filmstrip, viewer controls/overlays, compare/tasks switcher, header controls, current dialog/popover.

2. Add accessible labels for icon-only buttons.

   Keep existing tooltips, but also set accessible names/descriptions where GTK exposes them, especially for main menu, sidebar toggle, page navigation, zoom, compare inspector, trash/remove, tag, smart-tag, and upscale controls.

3. Make quality/status text explicit.

   Do not rely only on green/amber/red. Metadata chips and task rows should expose labels such as "Good quality", "Fair quality", "Needs attention", "Failed", "Queued", or "Completed".

4. Audit custom widgets.

   Review `MetadataChip`, `TagCard`, `CompareItem`, color swatches, filmstrip tiles, and viewer overlays for accessible roles, names, and keyboard activation.

5. Ensure all dialogs set useful initial focus.

   Collection dialogs, tag editor, export/upscale dialogs, preferences, and confirmations should focus the first meaningful control and support Enter/Escape predictably.

6. Add keyboard access for sidebar sections and virtual views.

   Duplicates, quality filters, collections, disabled folders, and folder rows should be reachable and activatable without pointer-only gestures.

7. Verify filmstrip tile navigation.

   Arrow keys should move selection, Enter should open/select, context actions should have keyboard alternatives, and focus should remain visible.

8. Make compare workflow keyboard-friendly.

   Add predictable shortcuts or focusable controls for entering compare, switching compared items, moving the slider, accepting output, and leaving compare.

9. Improve shortcut discoverability.

   Keep `help-overlay.ui`, README, and manual synchronized. Add missing common actions only after confirming they do not conflict with text inputs.

10. Add a manual keyboard-only QA script to release checks.

   This should be run before public releases and after major UI changes.

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
5. Use `Alt+Left`, `Alt+Right`, `0`, `Z`, `F11`, and `Alt+Return`.
6. Open the shortcuts overlay with `?`, close it with Escape.
7. Open tag editing with `Ctrl+T`, add/remove a tag, and return to the viewer.
8. Open Duplicates and Quality views from the sidebar or menu.
9. Enter Compare, switch compared items, and exit Compare.
10. Open Tasks and Preferences, navigate every visible control, and close with Escape or the window close shortcut.
11. Trigger Delete on a test copy and verify confirmation/undo/failure behavior is understandable.
12. Repeat with high contrast and large text enabled in GNOME settings.

## What Can Wait

- Full screen-reader perfection for every custom widget can follow the first keyboard-only pass.
- Translations can wait until the app text stabilizes.
- Advanced shortcut customization is not necessary for v1.
- Automated accessibility testing can come later; manual GTK accessibility review is higher value first.

## Success Criteria

For the next release, Sharpr should pass a keyboard-only walkthrough for browsing, viewing, tagging, comparing, exporting/upscaling, and preferences. Every icon-only control in the main workflows should have a tooltip and accessible name, and every color-coded state should also have text.
