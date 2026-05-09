# Sharpr Release and Flathub Readiness Audit

Date: 2026-05-09  
Sharpr checkout: `/home/hebbi/Projects/Sharpr`  
Report orientation: practical checklist for publishing as a serious GNOME/Flatpak app

## Executive Summary

Sharpr already has the core release shape of a GNOME application: app ID, desktop file, metainfo, icon, GSettings schema, GResource assets, Flatpak manifest, README, manual, shortcuts overlay, about dialog, and a documented native and Flatpak run path. It is close enough to treat release work as polish and risk reduction, not a ground-up packaging effort.

The biggest blockers before a public release are metadata quality, sandbox clarity, screenshot accuracy, dependency/bundled-binary review, and a repeatable release checklist. Flathub readiness needs stricter attention to AppStream completeness, Flatpak permissions, license clarity for bundled Real-ESRGAN and ONNX Runtime artifacts, and clean install behavior.

## Already Acceptable

- App ID is consistent: `io.github.hebbihebb.Sharpr`.
- Desktop, metainfo, schema, icon, GResource, manual, and help overlay exist under `sharpr/data/`.
- Flatpak manifest exists at `sharpr/packaging/io.github.hebbihebb.Sharpr.yml`.
- README documents Flatpak build, native development, shortcuts, and product identity.
- About dialog provides project URL, issue URL, license, developers, and acknowledgements.
- GSettings schema includes window size, library settings, export/upscale preferences, and pipeline history cap.
- Native run path is documented and matches repo instructions.
- Flatpak manifest builds vendored Rust sources offline and caps jobs to reduce memory pressure.

## Must Fix Before Public Release

1. Align product name everywhere.

   Current metadata uses both "Sharpr" and "Skerpa". Decide the public name and make app name, metainfo, README, icon identity, about dialog, and repository docs consistent.

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

## Should Fix Before Flathub

- Validate AppStream metadata with `appstreamcli validate`.
- Validate desktop file with `desktop-file-validate`.
- Confirm icon sizes and install paths meet Flathub expectations.
- Reassess runtime version `"50"` and SDK extension versions against current Flathub availability at release time.
- Avoid claims that all AI workflows are offline if ComfyUI may be remote.
- Ensure a clean first-run experience when no library is configured and no previous GSettings exist.
- Add a short Help or manual section for where Sharpr stores local indexes and how to reset them.
- Confirm GSettings migration behavior for legacy keys such as `upscaler-output-format` and `library-root`.
- Test Flatpak file chooser, "show in file manager", drag/open flows if supported, and trash behavior inside the sandbox.
- Confirm build reproducibility after `sync-flatpak-source.sh` and `flatpak-cargo-generator`.

## Nice-to-Have Polish

- Add a concise "What Sharpr is not" line: not a full editor, not a cloud photo service, not a Lightroom replacement.
- Add metainfo developer/contact quality improvements if Flathub review asks for them.
- Add localized strings later, but do not block a first personal open-source release on translations.
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
- Pass: open a Pictures folder, navigate images, switch folders, and close/reopen app
- Pass: tag image, create collection, reopen app, verify persistence
- Pass: run duplicate view, quality view, export, and an upscale task
- Pass: test Delete/trash and cancellation/failure paths
- Pass: verify screenshots, app name, summary, release notes, URLs, and license fields
- Pass: verify no public release text overpromises privacy or offline behavior

## Practical Release Position

Sharpr can be released as a personal open-source GNOME image curation app once metadata, screenshots, privacy wording, permissions, and licensing are cleaned up. Flathub readiness is mostly a packaging and trust-polish task, not a major architecture blocker.
