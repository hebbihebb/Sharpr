# Implementation Plan: Redesigned Tag Popover

## Objective
Redesign the tag popover in the viewer to match the user's provided mockup. The new popover will be anchored to the "+" button in the tag pill, pop upwards, and feature distinct sections for "SELECTED", "SUGGESTED" (AI tags), and "ALL TAGS". It will include inline tag creation and a "Manage tags" link.

## Key Files & Context
- `sharpr/src/ui/viewer.rs`: Contains the `tag_osd`, `tag_add_button`, and the current `tag_popover`. This file will receive the bulk of the structural UI changes.
- `sharpr/src/ui/filmstrip.rs`: We will review how tags are requested and updated to ensure consistency.
- `sharpr/src/ui/tag_browser.rs`: May need a small hook or signal if we need to open the main Tag Manager from the new "Manage tags" link.

## Implementation Steps

### Phase 1: Structural UI Overhaul (`viewer.rs`)
1. **Anchor & Position**:
   - Remove the existing top-right `tag_anchor` from the overlay.
   - Parent the `tag_popover` directly to `tag_add_button`.
   - Set `tag_popover.set_position(gtk4::PositionType::Top)`.
   - Adjust the `tag_popover` CSS classes to apply rounded corners, soft shadows, and consistent padding (e.g., `12px radius`, matching GNOME HIG).

2. **Popover Content Layout**:
   - Replace the current flat `tag_flowbox` and `tag_entry` layout with a structured `gtk4::Box` (vertical).
   - Add a sticky search entry (`tag_entry`) at the top with a placeholder "Search tags...".
   - Add a `gtk4::ScrolledWindow` to contain the lists.

3. **Sections (Selected, Suggested, All)**:
   - Create three distinct list containers (e.g., using `gtk4::ListBox` for vertical rows, which is better for the checkmark layout than `FlowBox`).
   - Add section headers ("SELECTED", "SUGGESTED", "ALL TAGS") with appropriate styling (dimmed text, small caps).
   - The "SUGGESTED" section will hold the existing `suggestions_flow` logic, adapted to fit the new row style if needed.

### Phase 2: Row Design & Interaction
1. **Tag Row Widget**:
   - Design a custom row widget for the list boxes: a `gtk4::Box` containing an icon or checkmark on the left, the tag name in the center, and a subtle "drag" or "menu" icon on the right (optional, based on mockup).
   - Clicking a row in "ALL TAGS" moves it to "SELECTED" and applies the tag via `db.add_tag()`.
   - Clicking a row in "SELECTED" moves it to "ALL TAGS" and removes the tag via `db.remove_tag()`.

### Phase 3: Bottom Action Bar
1. **Create New Tag (Inline Reveal)**:
   - Add a bottom `gtk4::Box` containing a "+ New tag" button.
   - Clicking this button hides itself and reveals a `gtk4::Entry` (for the new tag name) alongside "Cancel" and "Create" buttons.
   - Pressing "Create" or hitting Enter adds the tag to the DB and selects it.

2. **Manage Tags Link**:
   - Add a "Manage tags" button with a link styling (or an external link icon) aligned to the right of the bottom bar.
   - Clicking this should trigger an event to open the main Tag Manager. We will look for an existing method like `show_tag_manager` or emit a signal that `app.rs` / `window.rs` catches.

### Phase 4: Data Binding & Refresh Logic
1. **Update `refresh_tag_chips`**:
   - Currently, `refresh_tag_chips` populates the old `tag_flowbox`. We need to rewrite this (or create a new `refresh_tag_popover_lists`) to populate the three list boxes based on the current image's tags.
   - Read from the `TagDatabase` to get the active image's tags and all available tags.
   - Filter "ALL TAGS" to exclude those already in "SELECTED".

2. **Search Filtering**:
   - Wire the top `tag_entry` to filter the visibility of rows in the list boxes based on the tag name.

## Verification & Testing
- Open the viewer with an image.
- Click the "+" button in the bottom left tag pill area.
- Verify the popover opens *upwards* from the button.
- Verify the three sections are visible.
- Click tags in the "ALL TAGS" list and verify they move to "SELECTED" and the tag pill is updated.
- Click a tag in "SELECTED" and verify it is removed.
- Use the "+ New tag" inline flow to create a new tag.
- Search for a tag and verify the lists filter correctly.