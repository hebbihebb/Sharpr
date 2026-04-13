# Sharpr Manual

Sharpr is a fast, native image library viewer for GNOME.

## Setting Up AI Upscaling

Sharpr can upscale images using **RealESRGAN-NCNN-Vulkan**, a free open-source tool that runs entirely on your GPU. You need to download and set it up once.

### Step 1 — Download RealESRGAN-NCNN-Vulkan

Go to the releases page and download the Linux build (zip or tar.gz):

[RealESRGAN-NCNN-Vulkan releases](https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan/releases)

### Step 2 — Extract the archive

Extract the downloaded archive to a permanent location, for example:

`~/.local/share/realesrgan/`

The folder should contain the `realesrgan-ncnn-vulkan` binary and a `models/` subfolder with the `.bin` and `.param` model files.

### Step 3 — Set the binary path in Preferences

Open Sharpr → menu **(⋮)** → **Preferences** → paste the full path to the binary in the **Upscaler binary** field, for example:

`/home/yourname/.local/share/realesrgan/realesrgan-ncnn-vulkan`

Save Preferences. The **Upscale** action in the viewer is now available.

## Library

The left panel shows your folder tree. Click any folder to load its images into the filmstrip. Use the **Library** toggle in the header to pin or hide the panel. The panel collapses automatically on narrow windows.

## Filmstrip

The filmstrip shows thumbnails for all images in the current folder. Click a thumbnail to open it in the viewer. Use the **left and right arrow keys** to move between images. Thumbnails are decoded in the background and preloaded for adjacent images.

## Viewer

The viewer displays the selected image at full resolution. Click and drag to pan when zoomed in.

| Shortcut | Action |
|---|---|
| ← → Arrow keys | Previous / next image |
| Ctrl + Scroll | Zoom in / out |
| Ctrl + 0 | Reset zoom to fit window |
| Z | Toggle 1:1 pixel view |
| Alt + Return | Toggle metadata and tag overlay |
| Ctrl + T | Open tag editor |

## Metadata Overlay

Press **Alt + Return** or use the View menu to toggle the overlay. It shows image dimensions, format, file size, and an image quality score (IQ) in the bottom-right corner. The tag bubble appears bottom-left when the image has tags.

## Image Quality

Sharpr scores each image with an **IQ score** (0–100) based on resolution and file size. The score is shown in the metadata overlay and colour-coded: green is good, amber is fair, red needs attention. Use the Quality filters in the sidebar to show only images above or below a threshold.

## Tags

Press **Ctrl + T** with an image selected to open the tag editor. Type a tag name and press Enter to add it. Click the × on a tag chip to remove it. Images automatically receive format and resolution tags (e.g. **jpg**, **1080p**) on first load.

The **sparkles button (★)** in the tag editor runs local AI analysis and suggests tags based on image content. Click a suggestion to accept it, or press **Add All** to accept everything. Suggestions are not saved until accepted.

The **Tags** section in the sidebar lists every tag in your library with an image count. Click a tag to filter the filmstrip to matching images. Click × next to a tag to delete it from all images.

## Duplicates

The **Duplicates** section uses perceptual hashing (dHash) to find visually similar images. Groups are shown in the filmstrip so you can compare and delete unwanted copies.

## Editing

Use the **View menu (⋮)** to rotate an image 90° clockwise or counter-clockwise, or to flip it horizontally or vertically. Changes are applied in memory and written back to the original file when you press **Save Edit**. Press **Discard** to revert.

The **Upscale** action runs AI upscaling via RealESRGAN-NCNN-Vulkan when the tool is installed. A before/after comparison slider is shown so you can inspect the result before committing. Set the upscaler binary path in Preferences.

## Preferences

Open **Preferences** from the menu (⋮) to adjust the thumbnail cache size, set the path to the upscaler binary, and configure other options.
