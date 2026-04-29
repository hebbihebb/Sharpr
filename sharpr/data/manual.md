# Sharpr Manual

Sharpr is a fast, native image library viewer for GNOME.

## Setting Up AI Upscaling

Sharpr supports three upscale backends. Choose one in **Preferences → Upscale backend**:

| Backend | Requires | Best for |
|---|---|---|
| **CLI** (default) | RealESRGAN-NCNN-Vulkan binary | Fastest GPU upscaling, no setup beyond the binary |
| **ONNX** | Nothing — models download on demand | No external tools; works without a GPU |
| **ComfyUI** | A running ComfyUI server | Custom workflows, your own model library |

### CLI backend — RealESRGAN-NCNN-Vulkan

**Step 1 — Download**

[RealESRGAN-NCNN-Vulkan releases](https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan/releases)

Download the Linux build and extract it to a permanent location, for example `~/.local/share/realesrgan/`. The folder must contain the binary and a `models/` subfolder with the `.bin` and `.param` files.

**Step 2 — Set the binary path**

Open **Preferences** and paste the full path into the **Upscaler binary** field:

`/home/yourname/.local/share/realesrgan/realesrgan-ncnn-vulkan`

### ONNX backend — Swin2SR

Select **ONNX** as the backend in Preferences and choose a model:

- **Lightweight ×2** (8 MB) — fast, good general quality
- **Compressed ×4** (55 MB) — optimised for compressed or low-quality source images
- **Realworld ×4** (55 MB) — highest quality for photographs

The model file is downloaded automatically on first use and stored in `~/.local/share/sharpr/models/`. No GPU required; inference runs on CPU via ONNX Runtime. Transparency (alpha channel) is preserved.

### ComfyUI backend

For testing, first open **Preferences → Advanced** and enable **Show AI Upscale** so the upscale action appears in the main UI. Then open **Preferences → Upscaler**, confirm the **ComfyUI backend** toggle is on, and enter your server URL. On a remote machine over Tailscale, use the server IP directly, for example `http://100.121.114.22:8188`, rather than `localhost`.

Use **Test Connection** to verify Sharpr can reach the server. When you run an upscale, Sharpr uploads the source image, runs the bundled RealESRGAN ×4 workflow, and downloads the result automatically. The current ComfyUI integration only switches between the standard and anime RealESRGAN models, and the remote workflow itself always executes at ×4.

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

The **Upscale** action runs AI upscaling using whichever backend is configured in Preferences (CLI, ONNX, or ComfyUI). A before/after comparison slider lets you inspect the result before committing or discarding.

## Preferences

Open **Preferences** from the menu (⋮) to adjust the thumbnail cache size, choose the upscale backend, and configure other options.
