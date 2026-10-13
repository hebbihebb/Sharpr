# Sharpr Manual

Sharpr is a local-first image curation viewer for GNOME. It does not use runtime networking or AI services.

## Library And Filmstrip

Choose a folder in the Library sidebar to load its images. Thumbnails decode in the background, with visible images prioritized over preload work. Click an image to view it; Ctrl-click adds or removes images from the action selection.

## Viewer Shortcuts

| Shortcut | Action |
| --- | --- |
| Alt + ← / → | Previous / next image |
| Ctrl + Scroll | Zoom in / out |
| Ctrl + 0 | Fit image to the viewer |
| Alt + Return | Toggle metadata overlay |
| Ctrl + T | Open tag editor |
| Ctrl + F | Search names and tags; filter words such as 4k, png or 16:9 become chips |
| Ctrl + B | Add the selected images to a bucket |
| Ctrl + 1 / 2 | Viewer / Tags |
| [ / ] | Cycle Viewer and Tags |
| F9 | Toggle Library sidebar |
| F11 | Toggle presentation mode |
| Delete | Move selected images to trash |

## Tags And Collections

The tag editor adds and removes manual tags. Sharpr no longer adds format or resolution tags: type `png`, `4k` or `below 4k` in search instead, and use Main Menu → Library → Remove Old Automatic Tags… to clear the ones added in the past (a backup is made first). Collections are virtual groupings backed by tags; source files stay in their folders.

## Duplicates And Resolution

Duplicates uses perceptual hashes to group visually similar files and loads them into the filmstrip without changing files. To find low-resolution images, type `below 4k` (or `<1080p`) in search; `4k` finds images of at least 3840×2160 in either orientation.

## Compare

Ctrl-click until exactly two filmstrip images are selected, then choose **Image → Compare Selected Images**. The images appear as neutral left and right panes. Close Compare to return to the viewer. Missing or corrupt files show an error in their pane.

## Resize Copy

Choose **Image → Resize Copy…** for the current image.

- Width and height start at the displayed, orientation-corrected dimensions.
- Aspect ratio is locked by default and can be unlocked.
- Enlargement and reduction use Lanczos resampling.
- Formats are JPEG, PNG, WebP, and JPEG XL. Quality is disabled for PNG.
- The default folder is `~/Pictures/Resized`; the chosen folder is remembered.
- Names use `<name>-resized-<width>x<height>.<ext>` with a numeric suffix when needed.

Resize Copy writes a new file, does not copy embedded metadata, removes partial output after failure, and never changes the source.

## Preferences

Preferences manages libraries, collection import/export, metadata overlay visibility, and thumbnail cache size.
