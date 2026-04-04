# Sharpr — Development Setup

## Prerequisites

### Rust toolchain
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
rustup default stable
```

### System libraries (Fedora / RHEL)
```bash
sudo dnf install \
    gtk4-devel \
    libadwaita-devel \
    gexiv2-devel \
    pkg-config \
    gcc
```

### System libraries (Ubuntu / Debian)
```bash
sudo apt install \
    libgtk-4-dev \
    libadwaita-1-dev \
    libgexiv2-dev \
    pkg-config \
    build-essential
```

### Minimum versions required
| Library      | Minimum version |
|-------------|-----------------|
| GTK4         | 4.12            |
| Libadwaita   | 1.5             |
| GExiv2       | 0.14            |
| Rust         | 1.75 (stable)   |

## Build

```bash
cd sharpr
cargo build                   # debug build
cargo build --release         # release build
cargo run                     # run directly
```

## Verify GTK version
```bash
pkg-config --modversion gtk4
pkg-config --modversion libadwaita-1
pkg-config --modversion gexiv2
```

## Code quality
```bash
cargo clippy -- -D warnings   # lints
cargo fmt                     # format
```

## Architecture overview

```
src/
├── main.rs          → entry point, rexiv2 init
├── app.rs           → AdwApplication subclass
├── ui/window.rs     → AdwApplicationWindow, 3-pane layout, breakpoints
├── ui/sidebar.rs    → Folder tree explorer (SidebarPane)
├── ui/filmstrip.rs  → GtkListView thumbnail strip (FilmstripPane)
├── ui/viewer.rs     → Full-res image preview, zoom, pan (ViewerPane)
├── ui/metadata_chip.rs → Floating EXIF overlay (MetadataChip)
├── model/           → ImageEntry + FolderNode GObjects, LibraryManager
├── thumbnails/      → Background thumbnail decode worker
├── metadata/        → rexiv2 EXIF/XMP wrapper
├── upscale/         → NCNN subprocess runner (Phase 5)
└── config/          → JSON settings (Phase 6: GSettings)
```

## Key patterns used

- **GTK widget subclassing**: `mod imp { ... }` + `glib::wrapper!` + `#[glib::object_subclass]`
- **Background work**: `std::thread::spawn` + `async_channel` + `glib::MainContext::spawn_local`
- **Shared state**: `Rc<RefCell<AppState>>` (main thread only, no Arc/Mutex on GTK objects)
- **Adaptive layout**: `AdwNavigationSplitView` → `AdwOverlaySplitView` + `AdwBreakpoint`

## Phase status

| Phase | Status | Description |
|-------|--------|-------------|
| 1     | ✅ Done | Cargo scaffold + data model GObjects |
| 2     | ✅ Done | Three-pane adaptive UI shell |
| 3     | ⏳ Next | Folder sidebar + async thumbnail worker wiring |
| 4     | ⏳ Todo | Full image preview + metadata overlay |
| 5     | ⏳ Todo | AI upscaling integration |
| 6     | ⏳ Todo | Flatpak packaging |

## Next steps (Phase 3)

1. Wire thumbnail requests from the filmstrip factory's `bind` signal to `ThumbnailWorker`
2. Implement folder tree expansion in `SidebarPane` using `GtkTreeListModel`
3. Test with 500-image folder for performance regression
