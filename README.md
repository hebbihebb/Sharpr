# Sharpr

<p align="center">
  A GNOME-native image library viewer for Linux, built with GTK4, Libadwaita, and Rust.
</p>

---

<p align="center">
  <img src="./light.jpg" alt="Sharpr in light mode — folder browser, filmstrip strip, and full-resolution viewer with metadata overlay" width="900">
</p>

<p align="center">
  <img src="./dark.jpg" alt="Sharpr in dark mode — collections sidebar, multi-select filmstrip, and viewer with quality score overlay" width="900">
</p>

---

## Features

- **Three-pane library workflow** — smart folders on the left, a live filmstrip in the middle, and a large preview pane on the right
- **Smart folders** — browse duplicates, tags, search results, and global quality bands like `Excellent`, `Good`, and `Needs Upscale`
- **Fast viewer interactions** — fit/1:1 modes, Ctrl+scroll zoom, drag-to-pan, fullscreen, and background prefetching of nearby images
- **Compact viewer overlays** — bottom-right metadata and IQ chip plus bottom-left tag chip for quick tag editing
- **Tagging workflow** — click the tag pill or press `Ctrl+T` to open the inline tag editor; tag search and tag browser are both built in
- **Local AI tag suggestions** — run on-device image tagging from the inline tag editor, review suggested tags, and accept individual tags or apply them all at once
- **Rotate, flip, and save** — non-destructive in-view transforms with explicit save/discard actions
- **AI upscaling** — Real-ESRGAN integration with a model and output format chooser, saved smart defaults, and a before/after comparison slider
- **Background operations indicator** — a GNOME Files–style pill in the bottom-left corner tracks all long-running tasks (upscaling, duplicate scanning, thumbnail loading) with live progress bars; auto-dismisses when idle
- **Filmstrip context menu** — right-click any thumbnail to open in the default viewer, show in the file manager, or move to trash (trash option appears in duplicates mode)
- **Filmstrip sort controls** — reorder the current folder by `Name`, `Date Modified`, or `Type` from the filmstrip header
- **Preferences window** — configure your default library folder, preferred upscale model, and appearance settings in a dedicated three-page preferences dialog
- **Session persistence** — restore the last library folder and window size between launches
- **Keyboard shortcuts overlay** — press `?` or open the hamburger menu to see every shortcut in a searchable GNOME-style help overlay
- **Built-in manual** — open the bundled help manual from the app menu for setup and workflow guidance
- **Shared thumbnail caching** — memory cache, on-disk cache, and freedesktop thumbnail cache support for fast folder reloads
- **Metadata-aware quality scoring** — explainable IQ score derived from resolution, size, and format for wallpaper curation

## Current State

Sharpr is already usable as a desktop image library browser and viewer. The current codebase includes:

- folder browsing with persistent last-folder restore
- thumbnail strip with incremental loading, position badges, sort controls, and right-click context menu
- full-resolution preview with zoom, pan, fullscreen, and edit actions
- duplicate detection via perceptual hashing
- tag browser, tag search, inline per-image tag editing, and local AI tag suggestions
- compact GNOME-style metadata, quality OSD, and tag overlays with a shared visibility toggle
- quality smart folders that work across the whole indexed library
- AI upscale workflow with model and format selection, saved defaults, and commit/discard comparison
- background operations indicator for all long-running tasks
- preferences window for library root, upscale defaults, and appearance
- keyboard shortcuts help overlay and bundled in-app manual
- persistent window size restore across sessions

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
