# Sharpr Security and Privacy Audit

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Report orientation: practical local-desktop security, privacy, and data-safety risks

## Executive Summary

Sharpr has a good security posture for a local image curation app because its product identity is local-first, non-destructive, viewer-first, and explicit about original files remaining untouched. The strongest architectural choice is using Glycin for the main viewer path while keeping long-running work off the GTK main thread.

The main risks are not exotic remote exploits. They are practical desktop-product risks: non-Glycin decoding still exists in thumbnail, export, JPEG XL, metadata, quality, hash, and upscale paths; ComfyUI uploads full source images to a configured HTTP endpoint; benchmark logs can contain private file paths; the Flatpak manifest currently grants broad read access to home; and delete/trash behavior needs consistently visible confirmation and failure handling.

## High-Risk Issues

No immediate high-risk issue was found that obviously contradicts Sharpr's local-first model in normal default use. The closest high-risk area is ComfyUI when pointed at a non-local server: Sharpr uploads the source image and downloads generated output over the configured URL from `comfyui-url`, which defaults to `http://127.0.0.1:8188` but is user-editable.

Mitigation:

- Treat ComfyUI as a network boundary in UI text and docs: "Uploads the selected image to this ComfyUI server."
- Warn when the URL host is not loopback or localhost.
- Keep ComfyUI hidden behind the existing advanced/upscale preference path.
- Add request timeouts to all ComfyUI calls, not only health checks.

Affected files/modules:

- `sharpr/src/upscale/backends/comfyui.rs`
- `sharpr/data/io.github.hebbihebb.Sharpr.gschema.xml`
- `sharpr/data/manual.md`
- `sharpr/src/ui/preferences.rs`

## Medium-Risk Issues

### Split Image Decoding Surface

The README correctly advertises secure Glycin-based viewing, but Sharpr also decodes images through other paths:

- Thumbnail and hash work in `sharpr/src/thumbnails/worker.rs`
- Export and resize in `sharpr/src/export/mod.rs`
- JPEG XL helpers and temporary PNG handoff in `sharpr/src/jxl.rs`
- Upscale finalization through `image::ImageReader` in `sharpr/src/upscale/runner.rs`
- Metadata extraction through rexiv2/GExiv2 in `sharpr/src/metadata/*` and `sharpr/src/image_pipeline/worker.rs`

This is normal for an image application, but the security promise should say "viewer decoding uses Glycin" rather than implying every code path is sandboxed.

Mitigation:

- Document the decoder boundary in developer docs.
- Prefer Glycin for preview/display and keep non-Glycin decoders in worker-only paths.
- Add tests for corrupt/huge images in thumbnail, export, JXL, and upscale finalization.
- Enforce pixel-count and file-size guardrails before worker decode where practical.

### Benchmark Logs Include Private Paths

Benchmark events include full source paths in thumbnail, folder, hash, virtual view, and metadata events. Benchmark logging is opt-in through `SHARPR_BENCH`, which is good, but logs can still expose private folder names, filenames, and library structure.

Affected file:

- `sharpr/src/bench.rs`

Mitigation:

- Document that benchmark logs contain local file paths.
- Add an optional redaction mode that hashes or basename-only logs paths.
- Store benchmark logs under cache by default, not project or user-selected document locations.

### Flatpak Filesystem Access Is Broad

The manifest grants:

- `--filesystem=xdg-pictures`
- `--filesystem=home:ro`

This makes early testing easy but weakens the sandbox story for Flathub. A photo viewer may reasonably need broad library access, but a serious release should explain why and prefer portal-mediated folder selection where possible.

Affected file:

- `sharpr/packaging/io.github.hebbihebb.Sharpr.yml`

Mitigation:

- Reassess whether `home:ro` is required for the default release.
- Prefer document/file chooser portal access plus persisted user-selected library roots.
- If broad access remains, state it honestly in app metadata and README.

### Original File Safety Needs End-to-End Review

Sharpr is mostly non-destructive: tags, collections, quality, phash, and pipeline data are stored in SQLite databases rather than embedded in original files. Export and upscale write to configured output folders. Delete uses GIO trash rather than direct deletion in window/filmstrip flows.

Remaining risk:

- Trash behavior may be reachable by keyboard shortcut and filmstrip action; destructive intent should be consistently confirmed or undoable.
- Output path collision handling should stay centralized and tested.
- Temporary paths in `std::env::temp_dir()` should avoid predictable names where possible.

Affected files/modules:

- `sharpr/src/ui/window.rs`
- `sharpr/src/export/mod.rs`
- `sharpr/src/upscale/runner.rs`
- `sharpr/src/jxl.rs`

Mitigation:

- Add a confirmation or undo toast for trash.
- Keep export/upscale collision tests as required regression tests.
- Use securely-created temporary files/directories for future temp-file work.

## Low-Risk Polish Issues

- Metadata privacy: EXIF/GPS is displayed and indexed for dimensions/quality, but Sharpr does not appear to write EXIF/XMP/IPTC back to source files. Export behavior should explicitly state whether metadata is preserved or stripped.
- SQLite portability: library and tag databases live under `dirs::data_local_dir()/sharpr`, which is appropriate, but README should say where Sharpr stores local library data and how to remove it.
- Path handling: ignored-folder checks use `Path::starts_with`; symlink/canonical path behavior should be documented and tested.
- Network timeouts: ComfyUI health check has a timeout; upload, queue, poll, and download should also have bounded timeouts.
- Error messages: avoid surfacing full private paths in user-visible errors unless useful for the user's immediate action.

## Things Sharpr Already Does Well

- Main viewer path is Glycin-oriented.
- Original organization data is stored out-of-band in SQLite.
- AI upscale UI is hidden by preference by default.
- ComfyUI defaults to loopback.
- Long-running work uses background workers and main-thread result draining.
- Folder ignore state is respected by index reconciliation.
- Interrupted pipeline recovery exists in the persistent index.
- Benchmark logging is opt-in rather than always-on.

## Practical Mitigations

1. Add a developer-facing decoder map: Glycin viewer path, worker thumbnail path, export path, JXL path, metadata path, upscale path.
2. Add a ComfyUI privacy warning for non-loopback URLs and network timeouts for every HTTP call.
3. Add benchmark path redaction or a README warning.
4. Reconsider `home:ro` for Flatpak release builds.
5. Add corrupt/huge image tests for worker/export/upscale decode paths.
6. Add trash confirmation or undo, then manually test Delete and filmstrip trash.
7. Document where SQLite databases live and how to remove them.

## Honest Privacy Promise

Sharpr is a local-first, non-destructive image library viewer. It stores tags, collections, quality scores, hashes, pipeline history, and library indexes in local SQLite databases under the user's data directory and does not modify original image files for normal curation workflows. Network access is only needed for user-configured integrations such as ComfyUI or build-time packaging downloads; using a remote ComfyUI server uploads the selected image to that server. Benchmark logs are opt-in and may contain local file paths.
