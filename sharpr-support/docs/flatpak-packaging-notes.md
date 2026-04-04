# Flatpak Packaging Notes

## Current State

`packaging/com.example.Sharpr.yml` is a useful scaffold, but it is not yet ready for reproducible releases.

## Immediate Gaps

- The manifest builds the app from a local directory source.
- Rust dependency vendoring is not configured.
- The Real-ESRGAN section is placeholder-only.
- Runtime permissions have not been minimized against real app behavior yet.
- Packaging validation steps are not documented.

## Recommended Later-Phase Work

### 1. Make Rust builds reproducible

Use one of the standard Flatpak Rust approaches before release work:

- `flatpak-cargo-generator` with vendored Cargo sources
- a checked-in cargo vendor directory if the team prefers explicit source control

The current `cargo build --release` approach is acceptable for a scaffold but not for long-term reproducibility.

### 2. Revisit runtime and SDK versions

Confirm the GNOME runtime version against the GTK4 and Libadwaita versions actually required by the app at packaging time.

### 3. Tighten finish-args

Keep only the permissions required by shipped features:

- `--device=dri` if GPU rendering or Vulkan-based upscaling is required
- `--filesystem=xdg-pictures` only if broad library access remains necessary
- portal-first file access where possible
- avoid `--share=network` unless a concrete feature needs it

### 4. Package auxiliary binaries intentionally

If upscaling ships later:

- pin the Real-ESRGAN artifact version
- record checksums
- document model file locations under `/app/share`
- validate runtime GPU expectations inside Flatpak

### 5. Add packaging verification

Later-phase packaging checks should include:

```bash
flatpak-builder --user --install --force-clean build-dir packaging/com.example.Sharpr.yml
flatpak run com.example.Sharpr
flatpak-builder-lint packaging packaging/com.example.Sharpr.yml
```

## Desktop Integration Checklist

- desktop file installs correctly
- metainfo validates
- icon assets are present and installed
- GSettings schema is compiled during build
- app-id and desktop filename stay aligned

## Release Preparation Notes

Before treating Flatpak as release-ready, verify:

- clean build from a fresh builder directory
- no undeclared network dependency in build
- app launches under sandbox constraints
- file chooser and image library access behave as expected
- metadata dependency availability is confirmed in runtime
