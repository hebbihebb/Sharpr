# Sharpr Manual

Sharpr is a local-first image curation viewer for GNOME. It makes no network connections. Visual search runs a model on your own computer, from files you install yourself.

## Library And Filmstrip

Choose a folder in the Library sidebar to load its images. Thumbnails decode in the background, with visible images prioritized over preload work. Click an image to view it; Ctrl-click adds or removes images from the selection, Shift-click selects everything from the last clicked image to this one, and Ctrl+A selects all.

## Viewer Shortcuts

| Shortcut | Action |
| --- | --- |
| Alt + ← / → | Previous / next image |
| Ctrl + Scroll | Zoom in / out |
| Ctrl + 0 | Fit image to the viewer |
| Alt + Return | Toggle metadata overlay |
| Ctrl + F | Search: describe the images you want; filter words such as 4k, png or 16:9 become chips |
| Ctrl + B | Add the selected images to a bucket |
| Ctrl + A | Select all images shown |
| Ctrl + E | Export the selection as WebP |
| Ctrl + I | Image info: file, folder, camera, resolution score, buckets |
| Shift + F10 | Image menu: copies, compare, file actions |
| F9 | Toggle Library sidebar |
| F11 | Toggle presentation mode |
| Delete | Move selected images to trash |

## Visual Search

Start typing anywhere, or press Ctrl+F, and describe what you are looking for: `mountains at dusk`, `woman on a sofa`, `watercolor`. Images whose file names contain the words come first, then the closest images by content, 200 at a time; **Show More** adds the next 200. Search covers the current folder or bucket unless **Whole library** is on. Filter words such as `4k`, `png` or `16:9` become chips and narrow the results first.

Put `-` in front of a word to leave things out: `tower -water` finds towers without the images that look most like water (and files named after it), `-png` leaves out PNGs, and `-16:9` leaves out 16:9 images. Each becomes a "Not …" chip; click it to edit it as text again.

The line under the search box says how well the results match and how much of the library is indexed. "Only weak matches" means the words may not fit anything there. While indexing runs, a small ring in the header shows its progress; Main Menu → Library → Library Overview… shows the details, and can rebuild the index from scratch.

Visual search needs the EmbeddingGemma 2 model, installed once into the folder shown in Preferences → Library → Visual Search. Indexing then runs in the background at the lowest priority, on the graphics card when a GPU build is installed (Preferences → Library → Use Graphics Card): about 20 minutes for a library of 6,000 images, against several hours on the processor. It resumes after a restart. Until the model is installed, or before any images are indexed, search matches file names only.

## Buckets

To keep search results, click **Add to Bucket…** under the search box. With images selected it adds those; otherwise it offers the strong matches or everything shown, then asks for the bucket, where **New Collection…** makes a new one. A quick way to pick: Shift-click the last good result to select everything above it, then Ctrl-click any you don't want. Several searches can fill the same bucket.

Buckets are hand-picked groups of images; the files stay in their folders. A bucket remembers each image by its content, so it keeps it after the file is moved or renamed outside Sharpr. Add the selected images with Ctrl+B or by dragging them onto a bucket in the sidebar.

Tags are retired. Main Menu → Library → Turn Tags into Buckets… offers each old tag as a new bucket. Your tags themselves are kept, unused, and Main Menu → Library → Remove Old Automatic Tags… clears the format and resolution tags Sharpr once added (a backup is made first).

## Duplicates And Resolution

Duplicates uses perceptual hashes to group visually similar files and loads them into the filmstrip without changing files. To find low-resolution images, type `below 4k` (or `<1080p`) in search; `4k` finds images of at least 3840×2160 in either orientation.

## Compare

Ctrl-click until exactly two filmstrip images are selected, then right-click one of them and choose **Compare Selected Images**. The images appear as neutral left and right panes. Close Compare to return to the viewer. Missing or corrupt files show an error in their pane.

## Export As WebP

Select images and press Ctrl+E, or right-click → **Export as WebP…**. Choose the folder (**Wallpapers** is one click), lossless or a lossy quality, and optionally **Fit Within** a size such as 3840×2160, which shrinks larger images and never enlarges smaller ones. The export runs in the background: a notification counts progress and can cancel it, keeping the images already finished and leaving no partial file. Each copy is a new file (`name.webp`, `name-1.webp`, …); originals are never changed.

**Crop Copy…** on the right-click menu saves the chosen part of an image as a new lossless WebP in the same folder.

## Preferences

Preferences manages libraries, collection import/export, metadata overlay visibility, and thumbnail cache size. Its Library page also shows fingerprint and visual-search indexing progress, and the model folder.
