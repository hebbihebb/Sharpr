# Sharpr Manual

Sharpr is a local-first image curation viewer for GNOME. It makes no network connections. Visual search runs a model on your own computer, from files you install yourself.

## Library And Filmstrip

Choose a folder in the Library sidebar to load its images. Thumbnails decode in the background, with visible images prioritized over preload work. Click an image to view it; Ctrl-click adds or removes images from the action selection.

## Viewer Shortcuts

| Shortcut | Action |
| --- | --- |
| Alt + ← / → | Previous / next image |
| Ctrl + Scroll | Zoom in / out |
| Ctrl + 0 | Fit image to the viewer |
| Alt + Return | Toggle metadata overlay |
| Ctrl + F | Search: describe the images you want; filter words such as 4k, png or 16:9 become chips |
| Ctrl + B | Add the selected images to a bucket |
| Ctrl + I | Image info: file, folder, camera, resolution score, buckets |
| Shift + F10 | Image menu: copies, compare, file actions |
| F9 | Toggle Library sidebar |
| F11 | Toggle presentation mode |
| Delete | Move selected images to trash |

## Visual Search

Start typing anywhere, or press Ctrl+F, and describe what you are looking for: `mountains at dusk`, `woman on a sofa`, `watercolor`. Images whose file names contain the words come first, then the closest images by content, 200 at a time; **Show More** adds the next 200. Search covers the current folder or bucket unless **Whole library** is on. Filter words such as `4k`, `png` or `16:9` become chips and narrow the results first.

The line under the search box says how well the results match and how much of the library is indexed. "Only weak matches" means the words may not fit anything there.

Visual search needs the EmbeddingGemma 2 model, installed once into the folder shown in Preferences → Library → Visual Search. Indexing then runs in the background at the lowest priority, on the graphics card when a GPU build is installed (Preferences → Library → Use Graphics Card): about 20 minutes for a library of 6,000 images, against several hours on the processor. It resumes after a restart. Until the model is installed, or before any images are indexed, search matches file names only.

## Buckets

Buckets are hand-picked groups of images; the files stay in their folders. A bucket remembers each image by its content, so it keeps it after the file is moved or renamed outside Sharpr. Add the selected images with Ctrl+B or by dragging them onto a bucket in the sidebar.

Tags are retired. Main Menu → Library → Turn Tags into Buckets… offers each old tag as a new bucket. Your tags themselves are kept, unused, and Main Menu → Library → Remove Old Automatic Tags… clears the format and resolution tags Sharpr once added (a backup is made first).

## Duplicates And Resolution

Duplicates uses perceptual hashes to group visually similar files and loads them into the filmstrip without changing files. To find low-resolution images, type `below 4k` (or `<1080p`) in search; `4k` finds images of at least 3840×2160 in either orientation.

## Compare

Ctrl-click until exactly two filmstrip images are selected, then right-click one of them and choose **Compare Selected Images**. The images appear as neutral left and right panes. Close Compare to return to the viewer. Missing or corrupt files show an error in their pane.

## Resize Copy

Right-click an image in the filmstrip and choose **Resize Copy…**. Crop Copy and WebP Copy are in the same menu. Shift+F10 or the Menu key opens it for the current image.

- Width and height start at the displayed, orientation-corrected dimensions.
- Aspect ratio is locked by default and can be unlocked.
- Enlargement and reduction use Lanczos resampling.
- Formats are JPEG, PNG, WebP, and JPEG XL. Quality is disabled for PNG.
- The default folder is `~/Pictures/Resized`; the chosen folder is remembered.
- Names use `<name>-resized-<width>x<height>.<ext>` with a numeric suffix when needed.

Resize Copy writes a new file, does not copy embedded metadata, removes partial output after failure, and never changes the source.

## Preferences

Preferences manages libraries, collection import/export, metadata overlay visibility, and thumbnail cache size. Its Library page also shows fingerprint and visual-search indexing progress, and the model folder.
