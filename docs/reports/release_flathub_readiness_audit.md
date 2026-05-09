# Sharpr GNOME Polish and Packaging Readiness Audit

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Report orientation: practical GNOME polish checklist, not a Flathub release goal

## Executive Summary

Sharpr already has the core shape of a GNOME application: app ID, desktop file, metainfo, icon, GSettings schema, GResource assets, Flatpak manifest, README, manual, shortcuts overlay, about dialog, and a documented native and Flatpak run path. Flathub is not a real target right now, so the packaging guidance should be treated as a quality checklist rather than a release gate.

The biggest polish gaps are metadata quality, sandbox clarity, screenshot accuracy, dependency/bundled-binary review, clean install behavior, and a repeatable manual QA checklist.

## Owner Decisions / Product Corrections

- Flathub is not a real target, but GNOME polish still matters.
- Name remains Sharpr for now.
- Translations are not a priority.
- Keyboard navigation/accessibility is core.
- Manual QA is wanted and slower tests are acceptable if they catch important regressions.

## Already Acceptable

- App ID is consistent: `io.github.hebbihebb.Sharpr`.
- Desktop, metainfo, schema, icon, GResource, manual, and help overlay exist under `sharpr/data/`.
- Flatpak manifest exists at `sharpr/packaging/io.github.hebbihebb.Sharpr.yml`.
- README documents Flatpak build, native development, shortcuts, and product identity.
- About dialog provides project URL, issue URL, license, developers, and acknowledgements.
- GSettings schema includes window size, library settings, export/upscale preferences, and pipeline history cap.
- Native run path is documented and matches repo instructions.
- Flatpak manifest builds vendored Rust sources offline and caps jobs to reduce memory pressure.

## Must Fix For GNOME Polish

1. Align product name everywhere.

   Current metadata uses both "Sharpr" and "Skerpa". Owner decision is Sharpr for now, so make app name, metainfo, README, icon identity, about dialog, and repository docs consistent.

2. Replace draft screenshot metadata.

   `io.github.hebbihebb.Sharpr.metainfo.xml` points to `DesignDocs/UI design draft_v2.png`. AppStream screenshots should show the actual app, preferably current release UI, with stable URLs.

3. Clarify the privacy and network story.

   The metainfo says AI upscaling works "all offline and locally", but ComfyUI can upload images to a user-configured HTTP server. Reword to "local by default" or explicitly call out configurable local/remote backends.

4. Review Flatpak permissions.

   `--filesystem=home:ro` is broad. Decide whether public builds can rely on portal-selected libraries plus `xdg-pictures`, or document why read-only home access is necessary.

5. Confirm bundled binary/model licensing.

   Real-ESRGAN NCNN Vulkan and ONNX Runtime are downloaded and installed by the Flatpak manifest. Their licenses and notices need to be listed in README, about dialog acknowledgements, or packaged license docs.

6. Add release notes beyond the initial MVP.

   Metainfo currently lists only `0.1.0` with an MVP description. Public release should include current features: SQLite library index, tags/collections, duplicate detection, quality scoring, export, upscale backends, and major fixes.

## Packaging Quality Checklist

- Validate AppStream metadata with `appstreamcli validate`.
- Validate desktop file with `desktop-file-validate`.
- Confirm icon sizes and install paths follow GNOME/AppStream expectations.
- Reassess runtime version `"50"` and SDK extension versions before distributing Flatpak builds.
- Avoid claims that all AI workflows are offline if ComfyUI may be remote.
- Ensure a clean first-run experience when no library is configured and no previous GSettings exist.
- Add a short Help or manual section for where Sharpr stores local indexes and how to reset them.
- Confirm GSettings migration behavior for legacy keys such as `upscaler-output-format` and `library-root`.
- Test Flatpak file chooser, "show in file manager", drag/open flows if supported, and trash behavior inside the sandbox.
- Confirm build reproducibility after `sync-flatpak-source.sh` and `flatpak-cargo-generator`.

## Nice-to-Have Polish

- Add a concise "What Sharpr is not" line: not a full editor, not a cloud photo service, not a Lightroom replacement.
- Add metainfo developer/contact quality improvements if Flathub review asks for them.
- Leave translations for later; they are not a priority now.
- Add in-app empty states for "No library selected", "No images", "No duplicates", "No tasks", and "AI upscale hidden".
- Add a visible first-run folder action that explains local-first/non-destructive behavior without becoming a landing page.

## Files to Inspect or Change

- `README.md`
- `sharpr/data/io.github.hebbihebb.Sharpr.metainfo.xml`
- `sharpr/data/io.github.hebbihebb.Sharpr.desktop`
- `sharpr/data/io.github.hebbihebb.Sharpr.gschema.xml`
- `sharpr/data/manual.md`
- `sharpr/data/help-overlay.ui`
- `sharpr/src/app.rs`
- `sharpr/packaging/io.github.hebbihebb.Sharpr.yml`
- `sharpr/packaging/sync-flatpak-source.sh`
- `sharpr/packaging/cargo-sources.json`

## Release Candidate Checklist

- Pass: `cd sharpr && cargo build`
- Pass: focused behavior tests for current release changes
- Pass: `./check.sh` when preparing an actual release candidate
- Pass: Flatpak build/install/run from a clean checkout
- Pass: AppStream and desktop validation
- Pass: first run with empty settings
- Pass: open a folder, navigate the filmstrip, switch folders rapidly, and close/reopen app
- Pass: tag image, create collection, generate output, verify tag/collection inheritance behavior
- Pass: run duplicate view, quality view, compare/task-result virtual views, export, and an upscale task
- Pass: complete keyboard-only navigation through sidebar, filmstrip, viewer, collections, Tasks, compare, dialogs, and preferences
- Pass: test Delete/trash and cancellation/failure paths
- Pass: verify screenshots, app name, summary, release notes, URLs, and license fields
- Pass: verify no public release text overpromises privacy or offline behavior

## Practical Release Position

Sharpr can be polished as a serious GNOME image quality review app without treating Flathub as the target. AppStream, desktop-file, permission, license, and screenshot checks still matter because they improve trust and integration.
