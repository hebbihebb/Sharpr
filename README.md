# Sharpr

<p align="center">
  A GNOME-native image library viewer for Linux, built with GTK4, Libadwaita, and Rust.
</p>

---

<p align="center">
  <img src="./light.jpg" alt="Sharpr — three-pane layout with sidebar, filmstrip, and full-resolution viewer showing a mountain scene with metadata overlay" width="900">
</p>

<p align="center">
  <img src="./dark.jpg" alt="Sharpr — dark mode, Mountain collection selected, filmstrip with thumbnails and full-resolution viewer" width="900">
</p>

---

## What Sharpr Does

Sharpr is an image library viewer for Linux. It is made for browsing folders of images, finding useful groups of images, checking image quality, tagging images, and previewing edits before saving them.

You can use Sharpr to:

- browse image folders with a sidebar, thumbnail strip, and large preview
- open common folders or choose your own library folder
- disable folders you do not want Sharpr to scan or include in smart views
- sort images by name, date, or file type
- view images fullscreen, zoom in, zoom out, and pan around
- see basic image details such as size, dimensions, tags, and quality
- add and search tags
- find duplicate images
- group images by quality, such as good images or images that need upscaling
- rotate, flip, and save images
- upscale images with AI tools when configured
- add images to collections
- move images to trash from the app
- keep thumbnails cached so folders load faster after the first scan

## Current State

Sharpr is usable today as a desktop image browser and curation tool.

It can browse real folders, remember your last folder, load thumbnails in the background, show a large preview, search and edit tags, find duplicates, score image quality, and run AI upscaling if the required tool is installed.

The app stores its library information locally on your computer. Disabled folders are remembered and are left out of indexing, smart folders, search results, duplicate detection, quality views, and collections.

Sharpr is still being developed, but the main browsing, sorting, tagging, duplicate finding, quality checking, and upscaling workflows are already in place.

## Requirements

- GNOME 48 runtime (Flatpak) **or** GTK 4.14+ / Libadwaita 1.5+ natively
- For AI upscaling: `realesrgan-ncnn-vulkan` binary with model files in a `models/` subdirectory next to the binary

## Building

### Flatpak (recommended)

```bash
cd sharpr/packaging
flatpak-builder --force-clean --user --install build-dir io.github.hebbihebb.Sharpr.yml
flatpak run io.github.hebbihebb.Sharpr
```

> **Note:** `cargo-sources.json` must be present. Regenerate it after any `Cargo.lock` change:
> ```bash
> flatpak-cargo-generator ../Cargo.lock -o cargo-sources.json
> ```

### Native (development)

```bash
cd sharpr

# Install dependencies (Fedora example)
sudo dnf install gtk4-devel libadwaita-devel gexiv2-devel pkg-config gcc

cargo build
```

GSettings schemas must be compiled before running natively:
```bash
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR=data cargo run
```

## AI Upscaling setup

Download the `realesrgan-ncnn-vulkan` binary and place model files alongside it:

```
~/.local/bin/
  realesrgan-ncnn-vulkan
  models/
    realesrgan-x4plus.param
    realesrgan-x4plus.bin
    realesrgan-x4plus-anime.param
    realesrgan-x4plus-anime.bin
```

The Flatpak build bundles the binary and models automatically.

## Help

- Open **Manual** from the app menu for the bundled help window and setup guide
- Open **Keyboard Shortcuts** from the app menu, or press `?`, to browse available shortcuts

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| Alt+Left / Alt+Right | Previous / Next image |
| Ctrl+Scroll | Zoom in/out |
| Ctrl+0 | Reset to Fit |
| Z | Toggle 1:1 Pixels |
| F11 | Toggle fullscreen |
| Delete | Move to trash |
| Ctrl+T | Open tag editor |
| Alt+Return | Toggle viewer overlays |
| Ctrl+, | Open Preferences |
| ? | Show all shortcuts |

## License

GPL-3.0-or-later
