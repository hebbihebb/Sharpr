# Sharpr Manual QA Checklist

Use this checklist before a release or after any significant change to Sharpr.

## Thumbnail loading

- [ ] Open a folder with many images and confirm thumbnails appear without freezing the app
- [ ] Scroll rapidly through the filmstrip; thumbnails should load progressively without errors
- [ ] Open a folder containing a corrupt or unsupported file; the app should skip it gracefully

## Folder switching

- [ ] Switch between folders rapidly while thumbnails are still loading; the filmstrip should update correctly and show the new folder's images
- [ ] Open a folder on a different drive or path; contents should load correctly

## Keyboard-only navigation

- [ ] Navigate the sidebar folder tree using arrow keys
- [ ] Move through the filmstrip using arrow and Tab keys without touching the mouse
- [ ] Open the viewer on the selected image using the keyboard
- [ ] Open and dismiss dialogs, including preferences and about, using keyboard only
- [ ] ⚠️ flag any keyboard paths that are currently broken or missing

## Collections

- [ ] Create a new collection and add images to it
- [ ] Switch to a collection view; the filmstrip should show only that collection's images
- [ ] Sub-collections should be visible and selectable in the sidebar

## Tasks panel

- [ ] Start an export or upscale task and confirm it appears in the Tasks panel
- [ ] Progress should update while the task runs
- [ ] Completed tasks should show their output files
- [ ] ⚠️ known gaps: empty states when there are no tasks yet, failure display, and generated-output accept/discard flow are not yet fully implemented

## Compare mode

- [ ] Select two images and enter compare mode
- [ ] Both images should be displayed side by side
- [ ] ⚠️ known gap: the filmstrip in compare mode may show the previous folder's images bleeding below the compared pair; this is a known rough edge

## Generated outputs

- [ ] Export an image and confirm the output file is created in the expected location
- [ ] Run an upscale and confirm the output appears
- [ ] ⚠️ known gaps: generated outputs do not yet inherit source tags/collections, and are not yet auto-added to Upscaled/Exports output collections

## Trash

- [ ] Select an image and press Delete; the file should be sent to system trash
- [ ] ⚠️ known gap: there is no confirmation dialog before trashing; the file is sent to trash immediately on Delete key press

## App naming and metadata

- [ ] The app title bar, about dialog, and desktop entry should all display "Sharpr"
- [ ] ⚠️ known bug: commit a527a13 changed the display name to "Skerpa"; this has not been reverted yet, so you will see "Skerpa" in the UI until that is fixed
