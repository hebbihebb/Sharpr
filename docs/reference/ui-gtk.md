# Sharpr GTK And Libadwaita Reference

## Design Direction

Sharpr should feel like a calm, fast GNOME review desk. The workflows and the eight UX principles are in `product.md`:
- find
- curate in buckets
- prepare wallpapers
- clean up
- compare
- view fullscreen

This file covers how to build them in GTK.

## UI Rules

- Prefer standard GTK and Libadwaita widgets, actions, dialogs, popovers, overlays, and accessibility behavior over custom UI.
- Use `GAction`s for menu commands and their sensitivity.
- Compare currently needs exactly two selected images. Phase 8 extends it to N images in pairwise knockout (D8).
- Search, filters, and sort apply to the current folder or bucket unless the user explicitly picks the whole library. Clearing a search restores the scope, selection, scroll position, and current sort.
- Open search by typing anywhere or pressing Ctrl+F (D7). Keep Space free for viewing and Compare.
- Recognized filter words in the search box become removable chips in a wrapping bar under the header (D16). The bar wraps so it never widens the window. The combined Sort and Filter menu adds these same chips (D27). Clicking a chip's label turns it back into a quoted search word. Filter words that complete the word being typed are offered as flat buttons in the same bar, rather than in a popover, which would take focus from the entry.
- Words in the search box go to visual search when its model is installed (Phase 5). File-name matches come first, then the closest images, 200 at a time. A status line under the header always says what happened: match strength and coverage ("4,210 of 5,812 images indexed"), or that only file names were matched and why. Never show weak matches as if they were good ones (principle 5).
- Tags are retired (D1, Phase 5). Add no tag UI; buckets are how images are gathered.
- Keep the interface quiet. Only resolution with format, and bucket membership, are always visible; everything else belongs in the Info panel (D18).
- Destructive or broad actions need confirmation that states their effect. These include deleting a tag, deleting a bucket, trashing files, and bulk tag removal. When an action affects buckets, name them.
- Every background failure surfaces as a toast or dialog. Never let a failed read look like an empty result.
- Copy dialogs keep the oriented source aspect ratio by default. Crop defaults to a movable 16:9 selection. PNG output disables the quality control.
- The operation indicator is informational and does not navigate anywhere.
- Every meaningful icon-only control needs an accessible name or tooltip.
- Keyboard-only navigation is an acceptance path. Delete and other single-key shortcuts must not fire while a text entry has focus.

## Threading

See `architecture.md`. In UI code this means:
- Never decode, scan, hash, read headers, query SQLite in bulk, or touch files in a callback.
- Never move GTK objects into worker threads.
- Apply worker results only after checking a generation counter.

## Styling And Layout

- Prefer Adwaita style classes and theme variables. Overlays must stay readable in light, dark, and high-contrast themes.
- Support the 1024 × 600 logical-pixel minimum window size.

## Approved Look (D26–D27)

- Libadwaita supplies adaptive sheets, theme/accent handling, and toasts. Sharpr's shared look lives in `sharpr/src/ui/look.rs`; the main window has `.sharpr` and `.sharpr-mono-all`.
- `sharpr/data/fonts/` contains the official JetBrains Mono v2.304 variable fonts and OFL licence, embedded in GResource. `ui/fonts.rs` registers them once with Pango (≥ 1.56), using process-owned anonymous files held open for later font reads. No desktop font installation or user-cache extraction is needed. CSS prefers `JetBrains Mono`; the platform monospace face remains a fallback. About includes the font licence.
- Preferences and Keyboard Shortcuts are `AdwPreferencesDialog` and `AdwShortcutsDialog` sheets presented in the main window. They inherit its font and use `.sharpr-sheet` for compact rows and headers. Keep the existing `win.show-preferences` and `win.show-help-overlay` actions.
- Quick pickers use popovers and dismiss on click-away. Preferences and other editing sheets use their own close controls and Escape.
- One Sort and Filter menu adds the same removable chips as typing filter words. The old independent Filter popover is hidden and scheduled for removal.
