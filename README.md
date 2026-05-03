# Sharpr

A high-performance, local-first image curation tool and viewer for Linux, built with GTK4, Libadwaita, and Rust.

![Sharpr — Dark mode showing image curation and the high-resolution viewer](./dark.jpg)

## Features

- **Zero-Latency Navigation:** Browse massive folders instantly with chunked loading and background thumbnail caching.
- **Modern Image Pipeline:** Secure, sandboxed image decoding via [Glycin](https://gitlab.gnome.org/GNOME/glycin), providing robust format support.
- **Pro Curation Workflow:** Organize images with virtual collections, smart tagging, and quality scoring—all stored in a local SQLite database so your original files remain untouched.
- **Intelligent Tools:** Detect duplicates via perceptual hashing and enhance images with local AI models (NCNN or ComfyUI).
- **Modern Linux Native:** Built with GTK4 and Libadwaita for a deeply integrated, adaptive GNOME experience.

## Getting Started

### Flatpak (Recommended)

```bash
cd sharpr/packaging
# Generate sources if Cargo.lock changed: flatpak-cargo-generator ../Cargo.lock -o cargo-sources.json
flatpak-builder --force-clean --user --install build-dir io.github.hebbihebb.Sharpr.yml
flatpak run io.github.hebbihebb.Sharpr
```

### Native Development

Requires Rust 1.75+, GTK 4.14+, and Libadwaita 1.5+.

```bash
cd sharpr
# Fedora: sudo dnf install gtk4-devel libadwaita-devel gexiv2-devel
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR=data cargo run
```

## Shortcuts

| Key | Action |
|-----|--------|
| Alt+Left / Right | Previous / Next image |
| Ctrl+Scroll / 0 | Zoom / Reset to Fit |
| Z | Toggle 1:1 Pixels |
| F11 | Toggle Fullscreen |
| Delete | Move to Trash |
| Ctrl+T | Open Tag Editor |
| ? | Show all shortcuts |

## License

GPL-3.0-or-later
