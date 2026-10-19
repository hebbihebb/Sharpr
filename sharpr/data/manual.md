# Sharpr Manual

Sharpr is a local-first image curation viewer for GNOME. Visual search runs a model on your own computer, from files you install yourself; its managed runtime uses loopback only.

## Library And Filmstrip

Main Menu → Library → **Open Library Root…** chooses the one library folder, retaining its buckets and known files. Its image-containing subfolders appear in the sidebar. Disabled folders stay excluded, including from Info’s identical-copy count and background bucket re-linking. Narrowing the root hides outside members; widening restores them. Choose a sidebar folder to load its images. Thumbnails decode in the background, with visible images prioritized over preload work. Click an image to view it; Ctrl-click adds or removes images from the selection, Shift-click selects everything from the last clicked image to this one, and Ctrl+A selects all. A selection made this way counts even when it is down to one image: actions such as export, Delete, and Add to Bucket use it rather than the image you are viewing.

## Viewer Shortcuts

| Shortcut | Action |
| --- | --- |
| Alt + ← / → | Previous / next image |
| Scroll | Zoom in / out at the pointer |
| Ctrl + 0 | Fit image to the viewer |
| Ctrl + F | Search: describe the images you want; filter words such as 4k, png or 16:9 become chips |
| Ctrl + B | Add the selected images to a bucket |
| Ctrl + Shift + B | Add to the last successful bucket again |
| Ctrl + Shift + C | Compare the selected images |
| Ctrl + A | Select all images shown |
| Ctrl + E | Export the selection as WebP |
| Ctrl + I | Image info: file, folder, camera, resolution score, buckets |
| Shift + F10 | Image menu: copies, compare, file actions |
| F9 | Toggle Library sidebar |
| Double-click or F11 | Toggle fullscreen presentation; Escape exits after closing Info |
| Delete | Move selected images to trash |

The corner readout appears on pointer movement. Hover expands that same card into Info; leaving collapses it. Click or Ctrl+I pins it; Escape closes it. Right-click the large image for the same menu as the filmstrip. The keyboard image menu also works in fullscreen.

## Visual Search

Start typing anywhere, or press Ctrl+F, and describe what you are looking for: `mountains at dusk`, `woman on a sofa`, `watercolor`. Images whose file names contain the words come first, then the closest images by content, 200 at a time; **Show More** adds the next 200. Search covers the current folder or bucket unless **Whole library** is on. Updating a bucket keeps your current search and filters; clearing the search shows its updated members. Filter words such as `4k`, `png` or `16:9` become chips and narrow the results first.

Put `-` in front of a word to leave things out: `tower -water` finds towers without the images that look most like water (and files named after it), `-png` leaves out PNGs, and `-16:9` leaves out 16:9 images. Each becomes a "Not …" chip when you type a space after it or press Enter; click it to edit it as text again. A minus word leaves out the tenth of the library that looks most like it. If some still show through (a tower with a little water in it, say), use two minus signs: `tower --water` leaves out the 30% most like water, a "Strictly not water" chip, at the cost of hiding some good results too.

**Show Similar** on the image menu finds the pictures that look like that one. It runs as a search over the folder or bucket you're in, or the whole library when **Whole library** is ticked: a "similar to …" chip shows the image, which comes first. **Exact** lists versions of the picture: copies, and resized, converted, cropped or edited ones. **Close** adds the nearest pictures, at least a dozen, and whole series that stand out, such as bursts and AI variants. **Similar** (the default) lists every other picture closest first, 200 at a time with **Show More**. Other versions of the image in its own folder show only under Exact. Select some and Compare them to keep the best; nothing is moved or trashed unless you choose it. Filter chips still narrow the list, and removing the chip or pressing Escape goes back. An image that isn't indexed yet shows alone. Info says how many pictures across the library stand out as close to the image, such as "6 close images" (its versions elsewhere and whole series, not the nearest dozen every picture has); click it to open Show Similar at Close with Whole library ticked.

The line under the search box says how well the results match and how much of the library is indexed. "Only weak matches" means the words may not fit anything there. Main Menu → Library → Library Overview… shows indexing progress and can rebuild the index from scratch.

Visual search needs the EmbeddingGemma 2 model, installed once into the model folder available from Library Overview. Indexing then runs in the background at the lowest priority, on the graphics card when a GPU build is installed (Preferences → Library → Use Graphics Card): about 20 minutes for a library of 6,000 images, against several hours on the processor. It resumes after a restart. Until the model is installed, or before any images are indexed, search matches file names only.

## Buckets

When opening an older library, **Preparing library…** may appear while Sharpr backs up and upgrades its buckets in the background. If this fails, a message explains why: folder browsing stays available, and bucket controls remain disabled until the upgrade succeeds. Your originals and legacy tags stay intact.

To keep search results, click **Add to Bucket…** under the search box. With images selected it adds those; otherwise it offers the strong matches or everything shown, then asks for the bucket, where **New Bucket…** makes a new one. A quick way to pick: Shift-click the last good result to select everything above it, then Ctrl-click any you don't want. Several searches can fill the same bucket.

Buckets are hand-picked groups of images; the files stay in their folders. A bucket remembers each image by its content, so it keeps it after the file is moved or renamed outside Sharpr. Add the selected images with Ctrl+B or by dragging them onto a bucket in the sidebar.

**Edits.** Crop Copy, Export as WebP and Duplicate put the original and the new copy in a bucket under **Edits**: **Cropped**, **Exported** or **Duplicated**, made the first time they are needed. To compare a copy with its original, open that bucket, select the two, and press Ctrl+Shift+C; it works wherever you saved the copy, as long as it is inside your library. Remove images, or the whole bucket, when you are done; the files stay. Preferences → Library → **Collect Edits in Buckets** turns this off.

Tags are retired. Main Menu → Help → Library Maintenance → Turn Tags into Buckets… offers each old tag as a new bucket. Your tags themselves are kept, unused, and Main Menu → Help → Library Maintenance → Remove Old Automatic Tags… clears the format and resolution tags Sharpr once added (a backup is made first).

## Copies And Resolution

Main Menu → Library → **Find Copies…** looks among the images in view (the open folder, bucket, or search results) and finds exact copies (the same file in several places) and resized or converted copies of the same picture, such as a 1080p JPEG of a 4K PNG, or a WebP conversion. Crops, mirrored versions, and merely similar pictures should not count: every copy must have the same shape as the one kept and look the same as it in a small colour version. Look over each group before moving anything. Each group lists resolution, format, size, and folder with the highest resolution first, marked **Keep**. Nothing is ticked: tick what should go, or use **Select All Extras** (everything but each Keep) and **Select None**. A group's **Compare** button opens it in Compare, the copy marked Keep on the left; dismiss the ones you don't want, and **Done** brings you back with those ticked and the rest not. **Move to Trash…** asks before anything moves, warns when a whole group is ticked, and reports files it couldn't move. A file that changed after the search stays where it is, and so does the rest of a group whose kept copy changed or disappeared. Finding resized and converted copies needs the visual-search index; without it, only exact copies are found. On a large library, comparing similar pictures can take a little while; the window counts them as it goes, and closing it stops the search.

To find low-resolution images, type `below 4k` (or `<1080p`) in search; `4k` finds images of at least 3840×2160 in either orientation.

## Compare

Select two or more images (Ctrl-click, Shift-click, or Ctrl+A), then right-click one and choose **Compare Selected Images**. Compare shows them two at a time, in filmstrip order, and never changes files.

- **Side by Side**, **Split**, and **Flip** change how the pair is shown. In Split, drag the divider. In Flip, **Space** switches between the two; the badge of the hidden image is dimmed. Space also switches to Flip from the other modes.
- **Zoom:** scroll to zoom at the pointer, drag to pan, double-click for 100% or back to fit. **Fit** and **100%** do the same from the toolbar.
- **Linked zoom** (on by default) keeps both images on the same region, even when they have different resolutions, as for a 4K original and its 1080p copy. Turn it off to zoom and pan each side separately in Side by Side.
- **Choose** from the thumbnail row below the pair. Select Replace Left or Replace Right (Right is the default), then choose a thumbnail. L/R badges identify the pair. Choosing the opposite image swaps sides without dismissing either.
- **Keep Left** (**←**) or **Keep Right** (**→**) keeps that side and dismisses the other image; the next candidate takes the other's place. These choices never trash files. **Undo** or **Ctrl+Z** brings back the last image dismissed. The footer shows how many images remain.
- **Done** (or **Escape**) returns to browsing with the images you didn't dismiss selected.

The side panels make way while Compare is open. Delete, Rename (F2), Duplicate (Ctrl+D), and the image menu (Shift+F10) are off, as they are while cropping. An image that can't be read shows an error beside its filename. Search and filter chips make way for the mode controls in the top bar; returning restores them.

## Export As WebP

Select images and press Ctrl+E, or right-click → **Export as WebP…**. Choose the folder (**Wallpapers** is one click), lossless or a lossy quality, and optionally **Fit Within** a size such as 3840×2160, which shrinks larger images and never enlarges smaller ones. The compact export sheet stays open while you choose a folder, keeping your options. Click **Export** to start; **Cancel**, Escape, the close button, or a click outside cancels. The export runs in the background: a notification counts progress and can cancel it, keeping the images already finished and leaving no partial file. Each copy is a new file (`name.webp`, `name-1.webp`, …); originals are never changed.

**16:9 Guide** (Ctrl+G, or the right-click menu) outlines the largest 16:9 area over the image and dims the rest: an image wider than a wallpaper overhangs at the sides, a taller one above and below. Drag a corner square to resize it, or its outline to move it; dragging inside still pans a zoomed image. The guide stays where it is while you zoom and move between images; hide and show it again to fit it to the image. While it is shown, Crop Copy of the viewed image (or Ctrl+Shift+G) starts with the selection on the guide. The guide changes no files.

**Crop Copy…** on the right-click menu crops in the viewer at full size: the side panels make way (press F11 for fullscreen), and you drag the selection to move it or its corners to resize it, in 16:9 or freely. The top bar shows 16:9/Free, the crop's live pixel dimensions, the destination, **Cancel**, and **Save Copy**. **Save Copy** (or Enter) saves the selection as a new copy in the source format in the export folder, Wallpapers unless you chose another; Escape cancels. PNG and TIFF keep their sample depth and transparency; WebP and JPEG XL crops use lossless encoding. JPEG, AVIF and HEIC crops are re-encoded at maximum quality, so another lossy encoding step is possible. Crop Copy does not silently convert formats; unsupported sources report an error. Animated GIF crops keep the displayed first frame. Batch Export as WebP stays a separate action. If the image changes on disk while you crop, Save Copy refuses rather than crop the new file.

**Rename and Move.** File Actions → Rename (F2) opens with the name selected, ready to type over; the extension stays as a chip beside it (click the chip to change it too), and Enter renames. Rename and Move to Folder carry bucket membership and retained legacy tags to the new path. A missing image’s bucket or tag records still reserve its name: choose another name when renaming, or let Move choose the next free one.

**Copy to Clipboard.** The image-menu command prepares the full image in the background, in its displayed orientation. You can keep browsing; repeated copy requests keep the latest image. A failure is reported without replacing the clipboard contents.

**File copies.** Duplicate (Ctrl+D) and File Actions → Copy to Folder create new files with unique names, choosing `name (copy)` and numbered copies when needed. Existing files and links are never replaced.

## Preferences

Preferences holds the library root and graphics-card indexing setting. Library Overview holds counts, progress, and the model folder. Help has the guide, shortcuts, About, and the legacy tag maintenance tools.
