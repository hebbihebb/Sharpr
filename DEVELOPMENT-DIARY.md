# Sharpr — Development Diary

A human-readable account of how this application came to be, written from the git history.

---

## Day 1 — April 4, 2026: The Beginning

Everything started with a single large commit. Rather than building up piece by piece from a bare project, Sharpr was designed and sketched out in full before any code was written, and then the entire initial structure was committed at once. In one go we had the three-panel layout (sidebar on the left, image strip along the bottom, big viewer in the middle), the folder browser, the thumbnail loading system, a basic metadata viewer, and the foundations for everything that came later. It was a bold way to start — but it meant we hit the ground running.

---

## Day 2 — April 5, 2026: The Big Push

This was the longest and most productive day of the whole project. A huge number of features landed in quick succession.

**Tidying loose ends before moving forward.** Before charging ahead, we did a review pass to fix five small gaps that had been left in the initial design. Things like missing wiring between parts of the app, or features that were half-connected.

**Settings that survive restarts.** We migrated from a hand-rolled config file to the proper Linux system for saving application preferences (called GSettings). This meant things like which folder you had open last, or your zoom level, would be remembered between sessions. We also finalised the app's official identifier at this point — `io.github.hebbihebb.Sharpr`.

**Finding duplicate photos.** We added the first piece of duplicate detection. The technique works by turning each image into a kind of visual fingerprint (using something called a difference hash, or dHash). Two photos that look nearly identical will have fingerprints that are very similar. Later in the day we wired this up to scan whole folders and built a special "Duplicates" entry in the sidebar that groups the matches together automatically.

**Zoom and fit-to-screen.** A proper viewer needs zoom controls. We added a button in the header that toggles between "fit the image to the window" and "show it at actual size". Simple, but essential.

**Fullscreen mode.** Press F11 and the whole window expands to fill the screen. Press it again and it comes back. A one-liner to add but a big quality-of-life improvement.

**Delete key sends images to the bin.** Pressing Delete moves the currently selected photo to the system trash rather than permanently deleting it immediately — a much safer approach.

**A proper hamburger menu and About dialog.** The app got a tidy dropdown menu in the top-right corner, following the GNOME Human Interface Guidelines. This also brought in an About dialog with the app name, version, and links to the source.

**AI upscaling.** We integrated a third-party AI upscaling tool (Real-ESRGAN) that can enlarge photos while preserving detail. A dropdown was added to the viewer so you can pick which AI model to use — some are better for photographs, others for illustrations.

**Rotate and flip.** You can now rotate an image left or right, or flip it horizontally. The changes happen immediately in the viewer, and later in the day we wired it up to actually save those changes back to the file on disk.

**Before/after comparison slider.** When you upscale or edit an image, a draggable divider lets you slide between the original and the modified version side by side. A bug was found where the checkerboard background (used to show transparency) was rendering incorrectly — that was fixed the same day.

**Instant navigation with pre-loading.** Rather than waiting for the next image to load after you press the arrow key, the app now loads the images either side of the current one in the background while you're looking at the current photo. Flipping through a folder became noticeably snappier.

**Splash screen.** While the app is starting up, it now shows a branded splash screen instead of a blank window. The image is bundled directly into the app binary rather than loaded from disk.

**Disk thumbnail cache.** When the app loads thumbnails, it first checks a local cache on disk before going to the trouble of opening and decoding the original image. This makes re-opening the same folder much faster.

**A fix for a stubborn worker bug.** When you switch folders quickly, the thumbnail loading queue could get stuck processing thumbnails for the old folder even after you'd moved on. We fixed this by giving each folder a generation number — any work queued for an older generation gets thrown away automatically.

**Flatpak packaging and an app icon.** The app can now be installed as a Flatpak — the standard self-contained package format for Linux desktop apps. We also added the app icon at this stage. (A couple of corrections to the package manifest followed shortly after.)

**Performance improvements across the board.** At the end of this long day, we bundled several loading improvements: reading the small preview image embedded inside JPEG files before loading the full thing (almost instant first-paint), using a faster JPEG decoder for scaling thumbnails, and making the filmstrip populate itself as images are found rather than waiting until the whole folder is scanned.

**README added.** A screenshot and description were added to the top-level README file for anyone landing on the project page.

---

## Day 3 — April 6, 2026: Speed and Reliability

With the features largely in place, focus shifted to making everything faster and more reliable.

**Smarter thumbnail loading.** The filmstrip now uses two separate queues: one for the thumbnails you can see right now, and one for pre-loading the ones just off-screen. Visible thumbnails always get priority — no more waiting while the app pre-loads something you haven't scrolled to yet.

**No duplicate decode requests.** If the app asked for the same thumbnail twice (which could easily happen while scrolling), it would previously start decoding it twice. We added tracking so a second request for the same image just waits for the first one to finish.

**A memory cache for decoded images.** Once an image is fully decoded and ready to display, it's kept in memory for a while so switching back to it is instant. The cache holds a limited number of images and drops the oldest ones when it gets full.

**EXIF preview images.** Many cameras embed a small JPEG preview inside the raw image file. We added the ability to show that preview immediately — you see something straight away, then the full-quality version loads behind it.

**Fixing a colour corruption bug.** When saving a rotated or flipped image, colours were coming out wrong — the image data was in a slightly different format internally (BGRA) than what the save function expected (RGBA). A careful conversion fixed it.

**Fixing the filmstrip display.** Thumbnails in the filmstrip weren't always updating correctly on screen. This was traced to a GTK-specific mechanism for notifying widgets that a value has changed — once we used the right approach, the thumbnails showed up reliably.

**Code formatting pass.** All the Rust code was run through the standard formatter to make it consistent.

---

## Day 4 — April 7, 2026: Polish and Stability

**A crash on startup was fixed.** Shortly after a refactor, the app started crashing immediately on launch with an unhelpful error about something already being in use. The root cause was a subtle ordering issue — a piece of data was being accessed while it was already being read somewhere else. Reordering the startup sequence fixed it.

**Several quality-of-life improvements landed together:**
- Thumbnails no longer jump around or reload unnecessarily when you scroll
- The app remembers which folder you had open and returns to it on the next launch
- A small number badge appears on each thumbnail showing its position in the folder (so you know you're on image 12 of 47, for example)
- Right-clicking a thumbnail brings up a context menu
- You can now search for images by their tags

**Compile errors from a batch of external contributions were fixed.** Some code written by an AI assistant (Codex) had a few issues — a library API had changed, there was a borrow ordering problem, and a variable had been renamed. These were all sorted out.

---

## Day 5 — April 8, 2026: Playing Nice with the System

**Thumbnails from the system cache.** The Linux desktop has a standard location where apps store thumbnail images — the freedesktop thumbnail cache, usually found in `~/.cache/thumbnails`. Other apps like the file manager write their thumbnails there too. Sharpr now reads from and writes to this shared cache, which means: if you've already browsed a folder in your file manager, Sharpr can use those thumbnails without decoding anything. And the thumbnails Sharpr generates are available to other apps too. This was a significant speed improvement for the first load of large folders.

**Quality scoring and smart folders.** April 8 also brought in a new image-quality system aimed at wallpaper collections. Each image gets an "IQ" score based on its dimensions, file size, and format, and the sidebar gained virtual folders like *Excellent*, *Good*, and *Needs Upscale*. The first version worked, but it turned out to be looking only at whatever images were currently visible in the filmstrip. That led to confusing behaviour: you had to open a real folder first, and switching from one quality bucket to another could leave you with an empty result set. The fix was to build a proper whole-library index of lightweight image metadata and cache the quality calculations, so the quality folders now work globally instead of recalculating from the current view.

**The metadata overlay was redesigned.** The original quality indicator sat up above the image as a separate block, which made the viewer feel busier than it needed to. We replaced it with a much more compact bottom-right on-screen display: a two-line chip showing dimensions, file format, file size, and the IQ score with a small segmented indicator. The first pass had a bug where the chip would never actually appear, even though the metadata was loading correctly. After some debugging we discovered that the visibility logic was checking whether child widgets were visible while their parent container was still hidden — so the chip could never "wake up". Once the visibility state was tracked correctly, the new compact overlay behaved as intended.

**Tagging became visible and discoverable.** There was already code in the app to edit tags, but it was hidden behind the `T` key, which conflicted with the filmstrip's search capture behaviour. Pressing `T` would start a search instead of opening the tag editor, so the feature effectively felt broken. Rather than trying to force the shortcut to win, we made tagging visible in the viewer itself: a new bottom-left tag chip now shows the current tags and sits next to a `+` button. Clicking either one opens the existing tag popover. The shortcut was changed to `Ctrl+T`, which avoids the search conflict, and the help overlay and empty-state text were updated to match.

**A small UX refinement to tag display.** The very first version of the bottom-left tag chip only showed one tag and then a count of the rest. That technically worked, but it hid too much. We refined the summary so it now shows up to three tags before falling back to a counter, which feels much more informative while still staying compact enough not to steal attention from the image.

**A dedicated keyboard shortcuts overlay.** GTK4 has a built-in mechanism for showing all of an app's keyboard shortcuts in a single modal dialog — the "help overlay". We added one, accessible from the hamburger menu or by pressing Ctrl+?. Every shortcut in the app is documented there, following the GNOME style guide for how to present them. (A fix shortly after corrected some of the shortcut definitions that were using the wrong syntax — GTK printed warnings about them on startup.)

**A Preferences window.** The app now has a proper Preferences window (Ctrl+comma, or via the hamburger menu) with three pages: Library (set your default photo folder), Upscaler (choose the default AI model), and Appearance. Previously these settings were either inaccessible or scattered across different parts of the UI. Hooking up the library-root preference meant the sidebar would now scan whichever folder you configured, rather than always defaulting to Pictures, Downloads, and Home.

**The upscaler became a dialog.** Rather than hiding AI model selection in a menu item, we replaced the old upscaler menu entries with a proper "Upscale Image…" dialog. It asks which model to use before starting — cleaner, and consistent with how desktop apps handle options that need confirmation before a long operation.

**Some early startup quirks were cleaned up.** On a fresh launch, the Duplicates sidebar row was sometimes highlighted even though no scan had been run — fixed by making the app open the first real folder on startup instead. The tags view had also lost its hamburger menu button in a recent refactor, leaving it without a title bar; that was restored by restructuring where the toolbar lives. The tag browser chip design also got a visual refresh at this point.

**README and build version.** A build-time version number was wired in so the About dialog always shows the correct version. The README was refreshed, and a `CLAUDE.md` guidance file was added for future AI-assisted development sessions.

---

## Day 6 — April 11, 2026: Performance, Polish, and Production Readiness

After a couple of days away, a concentrated round of improvements landed — mostly focused on making the app feel fast and correct under real-world conditions.

**A background operations indicator.** Before this change, starting a long upscale and then switching folders left the app looking frozen or showing a spinner that appeared at unpredictable times. We replaced the old OSD progress bar with a proper bottom-left "pill" indicator — inspired by the file-transfer progress widget in GNOME Files. Any background operation registers itself immediately when it starts, so the UI never looks stuck. A small popover lists all active jobs with individual progress bars. Completed rows vanish after a few seconds, and the pill fades out when there's nothing running. Upscaling, duplicate detection, and thumbnail loading all feed into it.

**A deep fix for large folders.** Opening a folder with thousands of images was causing the UI to freeze for several seconds. The root cause: all the filesystem scanning was happening on the GTK event loop, blocking everything until the scan finished. The fix involved splitting the library manager's scan into two parts — a pure filesystem pass (`scan_folder_raw`) that's safe to run on a background thread, and a separate step that creates the GTK objects from those results on the main thread. This eliminated roughly 4,200 blocking system calls from the event loop. Large folders now show the UI immediately, with images appearing as they're found rather than all at once at the end.

**Thumbnail loading now survives folder switches.** Previously, switching folders would cancel the thumbnail worker queue and clear the memory cache. This meant that returning to a folder you'd already visited would re-decode everything from scratch. Both behaviours were changed: the background workers now drain their queues uninterrupted across folder switches, and the decoded-image cache is preserved. Returning to a folder you've already browsed is now instant.

**OSD bubbles unified.** The app had grown three separate floating overlay chips — the metadata chip, the operations indicator, and the tag browser — and they'd each accumulated slightly different opacity, font sizes, and positioning. All three were aligned to share the same visual style: matching opacity, font scale, and a consistent 16 px gutter from the overlay edge.

**Move to Trash in duplicates mode.** When reviewing duplicate photos, you can now right-click any thumbnail and choose "Move to Trash". The option only appears in the duplicates view (it's hidden in normal folder browsing), and it goes through the same safe GIO trash flow as the Delete key shortcut.

**Thread safety and cleanup fixes.** The underlying EXIF library (`rexiv2`, which wraps the C library `libexiv2`) is not thread-safe. The viewer was spawning concurrent metadata-loading threads that could all call into it simultaneously — causing occasional crashes on certain JPEGs. All rexiv2 calls are now serialised behind a global lock. Separately, a bug where the upscaler's temporary intermediate PNG file wasn't being deleted after a failed encode was also fixed.

**Smart defaults for upscaling.** The upscaler now remembers your preferred model (Standard or Anime) from the Preferences window and pre-selects it in the Upscale dialog. A choice of output format (PNG or JPEG) was also added, so you're not forced to always get a JPEG back.

**App icon properly bundled.** The app icon was being loaded from the filesystem at runtime, which meant it could be missing in certain desktop configurations or during the startup sequence. It's now compiled directly into the app binary via GResource and registered with the GTK icon theme on startup — ensuring it appears correctly everywhere: taskbar, About dialog, window title bar, and the X11 title bar.

**Dock icon and startup fixes.** GNOME Shell was not correctly linking the running app window to the launcher icon in X11 sessions. The cause was a capitalisation mistake in the `StartupWMClass` field in the desktop file (`sharpr` instead of `Sharpr`). The splash window was also being created in a way that confused Wayland's app-ID tracking. Both were corrected.

**A collection of smaller fixes.** Sidebar section ordering, duplicate perceptual hash detection accuracy, OSD positioning, and zoom/pan behaviour in the before/after comparison view all received targeted corrections in a focused bug-fix pass.

---

## Day 7 — April 12, 2026: Housekeeping

A short but necessary tidying day. Several files that had crept into the repository without being useful to anyone else were removed: internal AI tooling configuration, development scaffolding from an earlier prototype, and a leftover support directory. The `.claude` assistant settings folder was also moved out of version control and added to `.gitignore`. The README was updated to reflect all the features that had landed since it was last refreshed.

None of this changed anything visible in the app, but it cleaned up the repository so it presents itself clearly to anyone who looks at it.

---

## Day 8 — April 13, 2026: Library, Tags, AI Suggestions, and Help

A dense day of improvements touching four distinct areas of the app.

**The Library panel was redesigned.** The original implementation used a `NavigationSplitView` as the outer shell of the main window, which is a widget designed for navigating between pages — not quite the right fit for a persistent library drawer. It was causing layout edge cases, particularly on wide screens where the panel should stay pinned open alongside the viewer rather than replacing it. We replaced it with an `AdwOverlaySplitView`, which is closer to how GNOME Files arranges its sidebar: the library can either float as an overlay or pin itself to the side, collapsing cleanly on narrow windows. A dedicated `F9` shortcut toggles the panel open and closed. Getting the collapse and restore behaviour correct took several passes — four follow-up commits landed the same day to fine-tune breakpoints, visibility state, and the signal that triggers auto-hide — but the end result is a layout that behaves correctly at any window size.

**A small GTK deprecation was fixed.** The tag browser was calling `StyleContext::add_provider_for_display` using an API that had been marked deprecated in a recent GTK update. It was swapped for the current equivalent before it could cause warnings or future breakage.

**The metadata and tag overlays now share a single toggle.** Previously, pressing `Alt+Return` (or using the View menu's "Show Metadata" item) only hid the bottom-right metadata pill, leaving the bottom-left tag bubble still visible. This was a design inconsistency — the two chips belong to the same overlay layer and should disappear together. Both are now controlled by a single "Show Overlay" action. The tag bubble respects the hidden state on image navigation too, so it won't reappear when you move to the next photo while the overlay is off.

**Auto-tagging was cleaned up.** From the beginning, the tag system had been automatically generating tags from every image it scanned — pulling words out of filenames, camera model strings, lens names, focal lengths, ISO values, and years. In practice this flooded the tag browser with hundreds of entries that no one chose: fragments of filenames, camera model substrings, years, all appearing alongside the handful of tags the user had actually typed. The whole auto-indexing pass was removed. In its place, each image now receives two quiet automatic tags when it is first scanned: its **file format** (e.g. `jpg`, `png`, `webp`) and a **resolution bucket** (`4k`, `1080p`, or `720p`). These are inserted with `INSERT OR IGNORE`, so manually added tags are never overwritten. The tag browser was also fixed to show tags that appear on even a single image — it had been filtering them out with a `>= 2` threshold that was left over from when auto-tagging was creating noise.

**AI-powered tag suggestions arrived.** The first genuinely new ML feature landed: a sparkles button (★) in the tag editor popover that analyses the current image using a locally-running neural network and proposes 3–5 tags. The model is MobileNetV2, a compact image-classification network (~14 MB) from the ONNX Model Zoo, run entirely on-device using `tract-onnx` — a pure-Rust ONNX inference engine. No data leaves the machine; there is no API key, no subscription, and no internet required. When you click the sparkles button, a spinner appears briefly while the model runs (typically under 200 ms on a modern CPU), then a row of semi-transparent suggestion chips appears below the tag entry. Clicking a chip accepts it and adds it to the image's permanent tags; pressing "Add All" accepts the whole row at once. The model knows roughly a thousand standard categories drawn from the ImageNet training set — it does well on animals, landscapes, architecture, and common objects, though it is comically wrong on abstract or stylised images. The architecture was deliberately designed with future swappability in mind: the inference logic sits behind a `SmartTagger` trait, so a better model or an online API could be dropped in later without touching the UI code.

**The About dialog got a proper icon and credits.** The app icon had been showing as a generic white placeholder in the About window since it was first added. The underlying cause was a path mismatch: the GResource bundle was storing the icon under `icons/hicolor/512x512/apps/` but GTK's `add_resource_path` mechanism expects icons directly at `512x512/apps/` relative to the given resource prefix. Correcting the alias in the GResource XML fixed it immediately. The About dialog was also expanded with three new credit sections listing the key technologies the app is built on: GTK4 and Libadwaita for the UI, Rust, and the libraries behind the main features (tract-onnx, image-rs, SQLite/rusqlite, rexiv2/GExiv2, and RealESRGAN-NCNN-Vulkan for upscaling).

**A written manual was added.** Rather than relying solely on the keyboard shortcuts overlay, Sharpr now ships a short user manual (~two A4 pages) covering every major feature: the library panel, filmstrip, viewer shortcuts, metadata overlay, tags and AI suggestions, duplicates, editing, and preferences. The text lives in a Markdown file bundled inside the app binary via GResource. A new "Manual" entry in the hamburger menu opens it in a floating `adw::Dialog` window with a simple Markdown renderer: headings scale to different sizes, `**bold**` and `*italic*` are rendered correctly via Pango markup, bullet points and monospace table rows are styled appropriately. No external dependencies were needed — the renderer is a small hand-written function that converts Markdown blocks into GTK label widgets.

---

## Where Things Stand

As of the latest entry, Sharpr is a fully working image library viewer for the Linux desktop with:

- Browse photos by folder with a scrollable thumbnail strip
- Adaptive Library panel (pins open on wide screens, overlays on narrow ones, F9 to toggle)
- Full-resolution viewer with zoom, fit-to-screen, and fullscreen
- Rotate, flip, and save images
- AI-powered upscaling (with smart model and format defaults) and before/after comparison
- Automatic duplicate detection
- Tag-based searching with a SQLite-backed tag database
- Visible tag editing directly from the viewer, with auto-applied format and resolution tags
- On-device AI tag suggestions via MobileNetV2 (tract-onnx, no internet required)
- EXIF metadata display
- Metadata-aware image quality scoring and smart folders
- Unified overlay toggle hiding both the metadata chip and tag bubble together
- Keyboard shortcuts help overlay, a Preferences window, and a built-in user manual
- Background operations indicator showing live progress for all long-running tasks
- Fast loading even on large folders, with all filesystem work off the main thread
- Multiple layers of caching (memory, disk, system freedesktop thumbnail cache)
- App icon and splash screen bundled into the binary
- Packaged as a Flatpak for easy installation

The development moved quickly — from nothing to a feature-complete app in just eight days — thanks to a combination of careful upfront planning, AI-assisted code writing, and focused review and bug-fix passes between each major feature.
