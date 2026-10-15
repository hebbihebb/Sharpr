# Sharpr Manual

Sharpr is a local-first image curation viewer for GNOME. It makes no network connections. Visual search runs a model on your own computer, from files you install yourself.

## Library And Filmstrip

Choose a folder in the Library sidebar to load its images. Thumbnails decode in the background, with visible images prioritized over preload work. Click an image to view it; Ctrl-click adds or removes images from the selection, Shift-click selects everything from the last clicked image to this one, and Ctrl+A selects all. A selection made this way counts even when it is down to one image: actions such as export, Delete, and Add to Bucket use it rather than the image you are viewing.

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

Put `-` in front of a word to leave things out: `tower -water` finds towers without the images that look most like water (and files named after it), `-png` leaves out PNGs, and `-16:9` leaves out 16:9 images. Each becomes a "Not …" chip when you type a space after it or press Enter; click it to edit it as text again. A minus word leaves out the tenth of the library that looks most like it. If some still show through (a tower with a little water in it, say), use two minus signs: `tower --water` leaves out the 30% most like water, a "Strictly not water" chip, at the cost of hiding some good results too.

The line under the search box says how well the results match and how much of the library is indexed. "Only weak matches" means the words may not fit anything there. While indexing runs, a small ring in the header shows its progress; Main Menu → Library → Library Overview… shows the details, and can rebuild the index from scratch.

Visual search needs the EmbeddingGemma 2 model, installed once into the folder shown in Preferences → Library → Visual Search. Indexing then runs in the background at the lowest priority, on the graphics card when a GPU build is installed (Preferences → Library → Use Graphics Card): about 20 minutes for a library of 6,000 images, against several hours on the processor. It resumes after a restart. Until the model is installed, or before any images are indexed, search matches file names only.

## Buckets

To keep search results, click **Add to Bucket…** under the search box. With images selected it adds those; otherwise it offers the strong matches or everything shown, then asks for the bucket, where **New Collection…** makes a new one. A quick way to pick: Shift-click the last good result to select everything above it, then Ctrl-click any you don't want. Several searches can fill the same bucket.

Buckets are hand-picked groups of images; the files stay in their folders. A bucket remembers each image by its content, so it keeps it after the file is moved or renamed outside Sharpr. Add the selected images with Ctrl+B or by dragging them onto a bucket in the sidebar.

Tags are retired. Main Menu → Library → Turn Tags into Buckets… offers each old tag as a new bucket. Your tags themselves are kept, unused, and Main Menu → Library → Remove Old Automatic Tags… clears the format and resolution tags Sharpr once added (a backup is made first).

## Copies And Resolution

Main Menu → Library → **Find Copies…** looks among the images in view (the open folder, bucket, or search results) and finds exact copies (the same file in several places) and resized or converted copies of the same picture, such as a 1080p JPEG of a 4K PNG, or a WebP conversion. Crops, mirrored versions, and merely similar pictures should not count: every copy must have the same shape as the one kept and look the same as it in a small colour version. Look over each group before moving anything. Each group lists resolution, format, size, and folder with the highest resolution first, marked **Keep**. Nothing is ticked: tick what should go, or use **Select All Extras** (everything but each Keep) and **Select None**. A group's **Compare** button opens it in Compare, the copy marked Keep on the left; dismiss the ones you don't want, and **Done** brings you back with those ticked and the rest not. **Move to Trash…** asks before anything moves, warns when a whole group is ticked, and reports files it couldn't move. A file that changed after the search stays where it is, and so does the rest of a group whose kept copy changed or disappeared. Finding resized and converted copies needs the visual-search index; without it, only exact copies are found. On a large library, comparing similar pictures can take a little while; the window counts them as it goes, and closing it stops the search.

To find low-resolution images, type `below 4k` (or `<1080p`) in search; `4k` finds images of at least 3840×2160 in either orientation.

## Compare

Select two or more images (Ctrl-click, Shift-click, or Ctrl+A), then right-click one and choose **Compare Selected Images**. Compare shows them two at a time, in filmstrip order, and never changes files.

- **Side by Side**, **Split**, and **Flip** change how the pair is shown. In Split, drag the divider. In Flip, **Space** switches between the two; the badge of the hidden image is dimmed. Space also switches to Flip from the other modes.
- **Zoom:** scroll to zoom at the pointer, drag to pan, double-click for 100% or back to fit. **Fit** and **100%** do the same from the toolbar.
- **Linked zoom** (on by default) keeps both images on the same region, even when they have different resolutions, as for a 4K original and its 1080p copy. Turn it off to zoom and pan each side separately in Side by Side.
- **Choose** the image for either side from the picker below that side: it lists every image not yet dismissed, numbered in filmstrip order, with thumbnails; type to filter. Picking the image on the other side swaps them.
- **Keep Left** (**←**) or **Keep Right** (**→**) keeps that side and dismisses the other image; the next candidate takes the other's place. These choices never trash files. **Undo** or **Ctrl+Z** brings back the last image dismissed. The footer shows how many images remain.
- **Done** (or **Escape**) returns to browsing with the images you didn't dismiss selected.

The side panels make way while Compare is open. Delete, Rename (F2), Duplicate (Ctrl+D), and the image menu (Shift+F10) are off, as they are while cropping. An image that can't be read shows the error below its picker. Search and filter chips make way for the mode controls in the top bar; returning restores them.

## Export As WebP

Select images and press Ctrl+E, or right-click → **Export as WebP…**. Choose the folder (**Wallpapers** is one click), lossless or a lossy quality, and optionally **Fit Within** a size such as 3840×2160, which shrinks larger images and never enlarges smaller ones. The compact export sheet stays open while you choose a folder, keeping your options. Click **Export** to start; **Cancel**, Escape, the close button, or a click outside cancels. The export runs in the background: a notification counts progress and can cancel it, keeping the images already finished and leaving no partial file. Each copy is a new file (`name.webp`, `name-1.webp`, …); originals are never changed.

**Crop Copy…** on the right-click menu crops in the viewer at full size: the side panels make way (press F11 for fullscreen), and you drag the selection to move it or its corners to resize it, in 16:9 or freely. The top bar shows 16:9/Free, the crop's live pixel dimensions, the destination, **Cancel**, and **Save Copy**. **Save Copy** (or Enter) saves the selection as a new lossless WebP in the export folder, Wallpapers unless you chose another; Escape cancels. If the image changes on disk while you crop, Save Copy refuses rather than crop the new file.

## Preferences

Preferences manages libraries, collection import/export, metadata overlay visibility, and thumbnail cache size. Its Library page also shows fingerprint and visual-search indexing progress, and the model folder.
