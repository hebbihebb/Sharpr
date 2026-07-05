# Sharpr

A fast, local-first image curation viewer for Linux, built with GTK4, Libadwaita, and Rust.

![Sharpr — Dark mode showing image curation and the high-resolution viewer](./screenshot.webp)

## Features

- Responsive folder browsing with visible-first background thumbnails.
- Full-resolution viewing, zoom, metadata, and presentation mode.
- Manual tags, automatic format/resolution tags, and virtual collections.
- Duplicate detection and resolution-based quality views.
- Generic side-by-side Compare for exactly two selected filmstrip images.
- Non-destructive Resize Copy to JPEG, PNG, WebP, or JPEG XL.
- No runtime networking, model downloads, or AI processing.

Folders remain the source of truth. Sharpr never modifies original image files; Resize Copy creates a uniquely named output, and Delete moves files to the system trash only after an explicit action.

## Build And Run

### Flatpak

```bash
cd sharpr/packaging
bash sync-flatpak-source.sh
flatpak-cargo-generator ../Cargo.lock -o cargo-sources.json
flatpak-builder --force-clean --user --install build-dir io.github.hebbihebb.Sharpr.yml
flatpak run io.github.hebbihebb.Sharpr
```

The manifest targets GNOME 50 and requires the GNOME SDK plus the Rust stable SDK extension.

### Native Development

On Fedora, install `gtk4-devel`, `libadwaita-devel`, `gexiv2-devel`, `pkg-config`, and `gcc`, then run:

```bash
cd sharpr
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR=data cargo run
```

## Core Shortcuts

| Key | Action |
| --- | --- |
| Alt+Left / Alt+Right | Previous / next image |
| Ctrl+Scroll | Zoom |
| Ctrl+0 | Fit image |
| Ctrl+T | Open tag editor |
| Ctrl+1 / Ctrl+2 | Viewer / Tags |
| F9 | Toggle library sidebar |
| F11 | Toggle presentation mode |
| Delete | Move selected images to trash |
| ? | Show all shortcuts |

Minimum supported window size: 1024 × 600 logical pixels.

## License

GPL-3.0-or-later
