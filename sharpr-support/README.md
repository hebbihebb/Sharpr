# Sharpr

Sharpr is a Rust GTK4 + Libadwaita image library and viewer.

The codebase is early-stage. The current focus is on getting the core GTK shell, image loading flow, metadata display, and packaging story into a stable shape before broader feature work.

## Status

- GTK4 + Libadwaita desktop application
- Folder-based image library and viewer
- Background image decode and metadata loading
- Flatpak packaging scaffold present but not production-ready

## Repository Layout

```text
src/            Application code
data/           Desktop file, metainfo, GSettings schema
packaging/      Flatpak manifest skeleton
docs/           Contributor-facing project notes
DEVELOPMENT.md  Local setup and dependency notes
```

## Local Development

See [DEVELOPMENT.md](./DEVELOPMENT.md) for platform dependencies and local setup.

Recommended local verification loop:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build
```

## Documentation

- [CONTRIBUTING.md](./CONTRIBUTING.md)
- [docs/dev-log.md](./docs/dev-log.md)
- [docs/verification-workflow.md](./docs/verification-workflow.md)
- [docs/flatpak-packaging-notes.md](./docs/flatpak-packaging-notes.md)

## Near-Term Priorities

- Stabilize the current GTK UI flow
- Wire thumbnail generation end-to-end
- Add low-risk unit tests around pure helper logic
- Tighten packaging and contributor workflows
