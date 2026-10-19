# Sharpr Design Language

How Sharpr looks and behaves on screen. Read it before any user-facing change, together with `product.md` (what the app is for) and `ui-gtk.md` (how to build it in GTK). It gathers decisions D26, D27, D40, D45 and D46. Changing a rule here needs the user's approval and a decision-log entry, as for any product decision.

## Direction (D46)

- **Sharpr has its own design, not GNOME's.** The end state is plain GTK4 with Sharpr's own components, the way Strata does it (`docs/research/strata-design-and-agent-setup.md`). Libadwaita is on its way out. It stays only until each part it provides has been replaced.
- **Dark only.** Sharpr is a tool for looking at images, on a neutral dark canvas. It has one theme, designed and tested once. High contrast is still respected.
- **Gradually, never in one go.** New UI is built from Sharpr components. UI an agent is already changing moves over at the same time. Replacing what Libadwaita still provides (toasts, dialogs, split views, sheets, rows, the style manager) is its own Work Order item, one piece at a time. No broad rewrites, as `AGENTS.md` says.
- **Until a part is replaced, it is styled to match.** A Libadwaita widget that is still in use wears Sharpr's tokens and geometry, so the window never looks half and half.

## Principles

1. **Images first.** The canvas gets the space. Controls sit under or beside the images, never in a floating dock over them. The only things over an image are the corner readout, the key hints in crop, and guides the user turns on.
2. **A precise instrument.** One monospace face, square corners, hairlines, numbers that line up. Technical values (resolution, counts, sizes, formats) are shown exactly.
3. **Quiet until needed.** Only resolution with format and bucket membership are always visible (principle 6, D18). The rest shows on hover, when pinned, or in context. A control for something that can't happen right now isn't shown. For example, Compare hides Replace when only two images are left (D45).
4. **Actions sit next to what they affect.** Each control sits beside the thing it acts on, centred on the axis it belongs to, so the eye and the pointer travel little. Compare's Keep buttons are mirrored on the centre line (D45).
5. **The keys are part of the interface.** A shortcut is shown on the control it triggers (`← Keep Left`), not in a separate legend. Every task can be done from the keyboard.
6. **Honest state.** Loading, empty, failed and not indexed yet each look different. A failure shows where it happened, for example inside the image it concerns. It is never shown as an empty result.
7. **Sharp, not decorated.** No glow, gradients or heavy shadows. A hairline or a slightly lighter surface separates things.

## Tokens

The tokens live in `sharpr/data/style.css`. New CSS uses token names, never colour literals; only the token block holds raw values. Today some values are still literals or Libadwaita variables (`--accent-color`, `--window-fg-color`, `--error-color`); they move into tokens when their area is changed.

### Colour

| Token | Value | Use |
| --- | --- | --- |
| `--sharpr-canvas-bg` | `#1c1c1e` | Behind images: viewer, Compare, crop |
| `--sharpr-canvas-fg` | white | Text drawn on the canvas |
| OSD surface | `rgba(16, 16, 18, 0.84)`, inset hairline `rgba(255,255,255,0.08)` | Readout card, key hints, anything over an image |
| Text, three levels | white at 0.92 / 0.70 / 0.60 | Main, secondary, dim |
| Hairline | `alpha(currentColor, 0.10)` | Under bars, between panes |
| Hover / press | `alpha(currentColor, 0.06)` / `0.12` | List rows, flat buttons |
| Accent | the system accent, for now | Selection, current item, focus, chips |
| `--sharpr-warning` | `#f0a83a` | The below-4K hint and other cautions |
| Error | Libadwaita's error colour, for now | Negative chips, failures |
| Bucket colours | `palette.rs` | A bucket's own colour, its dots and pills |

Still to define as tokens: the window and sidebar surfaces, the accent (for when Libadwaita's accent goes), error, and success.

### Shape and size

- **Corners:** `--sharpr-radius` 4 px for buttons, entries, rows, cells and the readout. Chips use 3 px. Popovers use 8 px. Window buttons stay round.
- **Spacing:** a 2 px step, used mostly at 2, 4, 6, 8 and 12 px. Tight is the default.
- **Heights:** top bars 42 px, with one hairline running straight across every column. Buttons 26 px. Chips 22 px. Sidebar rows 24 px, menu rows 26 px, picker rows 28–34 px, sheet rows 40 px.
- **Smallest window:** 1024 × 600 logical pixels. Every screen must work at that size.

### Type

- **One face: JetBrains Mono**, bundled (`sharpr/data/fonts/`, `ui/fonts.rs`), for the whole interface (`.sharpr-mono-all`, 0.94 em).
- **Sizes:** 13 px body and sidebar, 12 px readout and chips, 11 px captions, key hints and section headings, 10 px tags such as "Below 4K".
- **Weights:** 500 for chips, 600–700 for short emphasis. Section headings are 11 px, slightly spaced (0.06 em) and dimmed.
- **Writing:** sentence case and few words. Numbers are shown exactly ("3840×2160", "12 remaining"). Labels name what will happen ("Keep Left", "Save Copy").

### Motion

Appearing takes 250 ms ease-out; fading when idle takes 600 ms ease-in. Nothing moves to show off, and motion follows the desktop's reduce-animation setting.

## Components

Each pattern lists where it lives today. Build new UI from these. If a pattern is missing, add it here with the user's approval rather than inventing a one-off.

| Component | What it is | Today |
| --- | --- | --- |
| **Top bar** | One flat 42 px bar per column, hairline below, no titles: an empty bar over the sidebar, the sidebar toggle and Sort and Filter over the filmstrip, search centred over the viewer. Modes (Compare, crop) swap their controls into the viewer's bar (D28) | `headerbar.sharpr-top`, `ui/window/mode_header.rs` |
| **Sidebar** | Narrow and tight, for jumping rather than reading: 24 px rows, dim 11 px section headings, counts in the mono caption | `.sharpr-sidebar`, `ui/sidebar.rs` |
| **Filmstrip cell** | A thumbnail with a faint tile while loading, the selection as a 2 px accent outline, small index and bucket pills | `.sharpr-filmstrip`, `.multi-selected`, `.sharpr-thumbnail-loading` |
| **Readout card** | The corner pill on the canvas: one line (resolution · format, Below 4K, buckets) that shows while the pointer moves and expands into Info on hover; click or Ctrl+I pins it (D18) | `.metadata-osd`, `ui/metadata_chip.rs` |
| **Chip** | A small square-cornered tag in the mono face, tinted with the accent (negative: the error colour), with × to remove; its label can turn back into text | `.sharpr-filter-chip`, Rename's extension chip |
| **Menu** | A compact popover menu: 26 px rows, 4 px padding | `popover.menu` |
| **Picker** | A popover for a quick choice (bucket, folder), dismissed by clicking away | `popover.sharpr-picker` |
| **Sheet** | A focused editing surface in the main window with its own close control and Escape (Rename, D40; Preferences; export) | `.sharpr-sheet`, `ui/window/rename_sheet.rs` |
| **Key hints** | A line of keys floating at the bottom of the canvas, only where keys aren't shown on controls (crop) | `.sharpr-keys`, `.sharpr-keys-osd` |
| **Action with its key** | A button whose label carries its key (`← Keep Left`); the main decision on a screen is the clearest button | Planned for Compare (D45) |
| **Candidate strip** | A centred horizontal thumbnail row, current items marked L/R, with the remaining count beside it; hovering a thumbnail shows halves to pick a side | Compare (D29, revised by D45) |
| **Status line** | One quiet line under the bar that says what a search or scan did and how much it covered (principle 5) | Search status |
| **Notice** | A short message with an optional Undo (trash, moves) | Libadwaita toasts, to be replaced (D46) |
| **Confirmation** | States the effect and names what is affected before anything destructive | Libadwaita alert dialogs, to be replaced (D46) |

## Layout

- **The columns are sidebar | filmstrip | viewer.** The filmstrip and the large viewer are the heart of Sharpr. Every other screen sits around them, never instead of them.
- **Modes take over the viewer, not the window.** Compare and crop use the viewer's area and its top bar. Leaving a mode puts everything back as it was.
- **Controls under the canvas centre on what they serve.** Per-image details sit on their image. Shared actions sit on the centre line (D45).
- **Floating surfaces are small:** the readout, key hints, popovers and sheets. Nothing covers the middle of an image unless the user asked for it.
- **The canvas is neutral.** Image colours must never be tinted by the interface.

## Before handing back UI work

- Build it from the components above, with tokens rather than new colour literals, and in the mono face.
- Take off-screen screenshots (`tools/gtk-offscreen.py`) at 1024 × 600 and at a wide size, with high contrast on and off, and look at them yourself before showing the user.
- Check the keyboard path. Give every icon-only control an accessible name or tooltip.
- If the work needs a pattern this file lacks, propose it to the user, then add it here with the decision.
