# Sharpr

<p align="center">
  A GNOME-native image library viewer for Linux, built with GTK4, Libadwaita, and Rust.
</p>

---

<p align="center">
  <img src=".github/assets/sharpr-2026-04-08.png" alt="Sharpr showing smart folders, filmstrip, viewer overlays, and image preview" width="900">
</p>

---

## Features

- **Three-pane library workflow** — smart folders on the left, a live filmstrip in the middle, and a large preview pane on the right
- **Smart folders** — browse duplicates, tags, search results, and global quality bands like `Excellent`, `Good`, and `Needs Upscale`
- **Fast viewer interactions** — fit/1:1 modes, Ctrl+scroll zoom, drag-to-pan, fullscreen, and background prefetching of nearby images
- **Compact viewer overlays** — bottom-right metadata and IQ chip plus bottom-left tag chip for quick tag editing
- **Tagging workflow** — click the tag pill or press `Ctrl+T` to open the inline tag editor; tag search and tag browser are both built in
- **Rotate, flip, and save** — non-destructive in-view transforms with explicit save/discard actions
- **AI upscaling** — Real-ESRGAN integration with model selection and before/after comparison
- **Shared thumbnail caching** — memory cache, on-disk cache, and freedesktop thumbnail cache support for fast folder reloads
- **Metadata-aware quality scoring** — explainable IQ score derived from resolution, size, and format for wallpaper curation

## Current State

Sharpr is already usable as a desktop image library browser and viewer. The current codebase includes:

- folder browsing with persistent last-folder restore
- thumbnail strip with incremental loading and right-click actions
- full-resolution preview with zoom, pan, fullscreen, and edit actions
- duplicate detection via perceptual hashing
- tag browser, tag search, and inline per-image tag editing
- compact GNOME-style metadata and quality OSD
- quality smart folders that work across the whole indexed library
- AI upscale workflow with pending-output comparison and commit/discard

The project is still under active development, but the app is beyond prototype stage and already covers the main browsing and curation loop.

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

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| Alt+Left / Alt+Right | Previous / Next image |
| Ctrl+Scroll | Zoom in/out |
| Ctrl+0 | Reset to Fit |
| F11 | Toggle fullscreen |
| Delete | Move to trash |
| Ctrl+T | Open tag editor |
| Alt+Return | Toggle metadata overlay |
| ? | Show all shortcuts |

## License

GPL-3.0-or-later
