# Learning from Strata: window chrome, fonts, icons, and agent setup

Research only. Nothing here is *planned*. Adopting any idea that changes a rule in `docs/reference/ui-gtk.md` or `product.md` needs the user's approval and a decision-log entry.

- **Subject:** [lgse/strata](https://github.com/lgse/strata), a keyboard-first GTK4 file manager in Rust (MIT), read at `v1.0.0` (`0a086aa`).
- **Sharpr baseline:** the `viewer-reset` working tree on October 12, 2026, plus `screenshot.webp`. That screenshot dates from May 2026, so its details may be out of date.
- Paths below in `code` are Strata paths unless they start with `sharpr/`.

## Summary

Strata looks cohesive because it treats styling as a small system: one stylesheet, nine color tokens, one bundled font, one icon family, and one size scale. Every surface is built from those parts. Two things matter most for Sharpr:

1. **Strata does not use Libadwaita.** It draws its own header row, window buttons, dialogs, and theme engine on top of plain GTK4. Sharpr is an Adwaita app by rule (`ui-gtk.md`: "Prefer Adwaita style classes and theme variables"). Copying Strata's look wholesale would mean reversing that rule, and it is not recommended.
2. **The parts worth taking don't depend on dropping Adwaita.** They are discipline, not widgets: one stylesheet, named color tokens, bundled namespaced icons for Sharpr's own concepts, tabular and monospace numbers for technical values, quieter and thinner chrome, and an off switch for decoration. Each one fits inside Adwaita.

Recommended order, smallest and safest first:

| # | Change | Why | Effort |
| --- | --- | --- | --- |
| 1 | Move all CSS into one `sharpr/data/style.css`, loaded automatically by `AdwApplication` | Today 13 `CssProvider`s in 7 files load styling, two of them from resource files; one file is the precondition for a consistent look | Small |
| 2 | Define the bucket palette and overlay colors once, as named tokens | The same 8-color bucket palette is copied into 5 files, and the overlays hardcode `rgba(…)` values | Small |
| 3 | Make the three header bars flat and quiet | Raised headers and the "Library", "Photos", and "View" titles are the heaviest chrome on screen | Small–medium |
| 4 | Use `.numeric` or `.monospace` for resolution, counts, and format badges | Gives Strata's "precise instrument" feel where it carries meaning, without a bundled font | Small |
| 5 | Bundle a few namespaced symbolic icons for Sharpr concepts (bucket, compare, crop, WebP export, below-4K) | Generic system icons such as `bookmark-new` and `emblem-system` don't say what Sharpr does | Medium |
| 6 | Add mechanical guardrails for agents: Clippy `disallowed-methods` for file writes, and a `[lints]` table | Turns the "originals are never modified" invariant from prose into a compile error | Small |

## 1. Strata's design language

### What you see

![Strata Columns view](https://raw.githubusercontent.com/lgse/strata/0a086aa3bdb251842a1e8413e2a6b35970d0b409/docs/assets/strata-columns.png)

- **One chrome row.** The header is a single 40 px row: sidebar toggle, then a breadcrumb location bar that takes all free space, then a run of 16 px icon-only actions that ends with Strata's own minimize, maximize, and close buttons (`src/ui/window/composition/layout.rs`, `Header::new`). There is no window title. GTK's title buttons are turned off (`set_show_title_buttons(false)`) and redrawn with Lucide icons, so they match the other actions exactly. The maximize icon swaps to a "restore" glyph when the window is maximized (`bind_maximize_icon`). Each window button can be hidden in Settings.
- **Flat surfaces, hairline borders.** Headers are `@strata_surface` with `border-bottom: 1px solid alpha(@strata_border, 0.6)` and `box-shadow: none` (`src/style.css` around line 92). Buttons are transparent until hovered, then show `alpha(accent, 0.15)`. A press shows `alpha(accent, 0.22)` and `transform: scale(0.97)`, over 160 ms. The usual corner radius is 5 px, and cards use 9–12 px.
- **Accent as the only color.** Interface icons, the focus ring (`outline: 2px solid @strata_accent`), selection, and active tabs all use the one accent. Everything else is background, surface, text, or dim text.
- **Optional glow.** Popovers, cards, and dialogs carry `box-shadow: 0 0 14px alpha(@strata_glow, 0.24)`. `@strata_glow` is defined as either the accent or `transparent`, so one preference ("element glow") turns every glow off at once (`src/ui/theme.rs`, `apply_tokens`). This is the most reusable trick in the stylesheet.
- **Small, dim section labels.** Sidebar groups ("PINNED", "DEVICES") are `0.78em`, weight 700, in dim text (`.sidebar-heading`). Captions in dialogs use uppercase with `letter-spacing: 0.12em`.
- **Contextual footer.** A 24 px footer lists the shortcuts that work in the current focus, and F1 opens a searchable reference (`src/ui/shortcut_footer.rs`).
- **Skeletons, not spinners.** Loading folders show grey `alpha(@strata_dim_text, 0.22)` blocks shaped like rows (`.loading-skeleton`, `src/ui/loading_skeleton.rs`).

The window outline in the screenshots comes from the Hyprland/Omarchy compositor, not from Strata.

### How it is built

- **Nine semantic tokens.** `bg`, `surface`, `text`, `accent`, `danger`, `muted`, `highlight`, `border`, and `dim_text` (`docs/themes.md`). The theme manager writes them as `@define-color strata_*` into one CSS provider at runtime (`tokens_css` in `src/ui/theme.rs`). The 7,091-line `src/style.css` uses only those names; `AGENTS.md` bans hex colors for themeable elements.
- **95 bundled themes** live in `data/themes/catalog.toml`. Most are Tinted Base16 palettes mapped onto the nine tokens, with contrast overrides commented in place: text is at least 4.5:1 on the background and 3:1 on the surface. Custom themes are small TOML files in `~/.config/strata/themes/`.
- **One size scale.** Header height, control size, and icon size are computed from the root font size (`scale = root_font_px / 13`). A "text size" preference therefore resizes the whole chrome consistently, icons included (`set_interface_icon_scale`).
- **Motion helpers.** `src/ui/motion.rs` has one `animations_enabled()` check, which respects both the app's reduce-motion setting and `gtk-enable-animations`, plus two named easing curves: emphasized deceleration for entering and acceleration for leaving.

## 2. Sharpr's chrome today

The code paths are `sharpr/src/ui/window.rs` (`build_viewer_header`), `sharpr/src/ui/sidebar.rs`, and `sharpr/src/ui/filmstrip.rs`.

- **Three `AdwHeaderBar`s side by side:** Library (sidebar), Photos (filmstrip), and View (viewer). The viewer bar uses `ToolbarStyle::Raised`, and the window buttons sit at the far right. In the screenshot, the three titles and the raised bar make the top 48 px the busiest part of the window, above an image app's content.
- **Icons are all system icon names** (about 30 distinct `*-symbolic` names). They are generic: `bookmark-new-symbolic` stands in for buckets, `emblem-system-symbolic` for settings, and `pan-down-symbolic` for sort.
- **Styling is scattered.** Two resource stylesheets (`collection.css`, `viewer-osd.css`) are each loaded by hand from a widget. In total 13 `CssProvider`s are created, in `tag_card.rs`, `metadata_chip.rs`, `ops_indicator.rs`, `tag_browser.rs`, `filmstrip.rs`, `viewer.rs`, and `sidebar.rs`.
- **Hardcoded colors.** The bucket palette `"#57e389", "#62a0ea", …` is duplicated in `tag_browser.rs`, `collection_dialogs.rs`, `viewer.rs`, `filmstrip.rs`, and `sidebar.rs`. The metadata chip and filmstrip overlays use fixed `rgba(…)` values, and the sidebar maps folder icons to fixed hex colors.

None of this is broken, but it explains why Sharpr reads as "default GNOME" and Strata reads as "designed". Strata's look comes from consistency more than from any single widget.

## 3. Ideas for Sharpr's window chrome

### Tier 1: fits the current Adwaita rules

1. **One application stylesheet.** `AdwApplication` loads `style.css`, `style-dark.css`, and `style-hc.css` from the app's resource base path automatically. For Sharpr that path is `/io/github/hebbihebb/Sharpr/`, derived from the application ID. Add `sharpr/data/style.css` to the gresource, then move `collection.css`, `viewer-osd.css`, and the static inline providers into it. Generated per-color classes, such as the bucket dots, can stay generated, but they should reference the tokens.
2. **Sharpr tokens on top of Adwaita variables.** Strata's lesson is "name every color once", not "replace Adwaita". Sharpr runs on GNOME 50 (`packaging/io.github.hebbihebb.Sharpr.yml`), so Libadwaita's CSS variables (`--accent-bg-color`, `--window-bg-color`, `--view-bg-color`, `--headerbar-shade-color`, …) are available at runtime. Define a handful of Sharpr names in the stylesheet, such as an OSD background, OSD text, and the 8 bucket colors. Then delete the hardcoded `rgba(…)` and hex values in code. The bucket palette also belongs in one Rust constant.
3. **Quieter headers.**
   - Switch the viewer `ToolbarView` to `ToolbarStyle::Flat`.
   - Drop the static "Library" and "Photos" titles. The panes are self-evident, and principle 6 ("Quiet interface") argues against them. Keep the dynamic folder name if there is one.
   - Make the pane separators 1 px hairlines in the border color, which is Strata's `paned > separator` style.
   - Keep the standard window buttons (see Tier 3).
4. **Numbers that look like numbers.** Use Libadwaita's `.numeric` class (tabular figures) for resolution, counts, and zoom percentages. Consider `.monospace` for the resolution and format chip and the below-4K hint, which D18 makes the only always-visible metadata. This borrows Strata's technical, precise feel exactly where Sharpr's product says precision matters, at no cost.
5. **Small dim section labels.** Use the `.caption-heading` class with dimmed text for "Pictures" and "Collections" in the sidebar, in place of bold body text. Note that `.dim-label` is deprecated in recent Libadwaita in favor of `.dimmed`.
6. **Hover and press feedback.** Strata's `scale(0.97)` press and 160 ms background transition are pure CSS and work on Adwaita `.flat` buttons. Apply them sparingly, only to header and filmstrip-overlay buttons.
7. **One glow switch, used once.** An image app shouldn't glow everywhere. One place earns it: the selected filmstrip thumbnail could get an accent outline plus a soft accent shadow. Define it as a single token, as Strata does with `@strata_glow`, so it can be turned off or tuned in one line. This is a CSS class on the existing cell and leaves the thumbnail scheduler alone, which the filmstrip-performance invariant requires.
8. **Skeleton thumbnails.** Use a flat `alpha(currentColor, 0.08)` tile with the cell's aspect ratio while a thumbnail loads, instead of an empty or broken-looking cell. This is again CSS on the existing factory bind only. It would also make the known "stale empty thumbnail on first Compare entry" race less jarring, without fixing it.
9. **Tooltips only on icon-only buttons.** This is Strata's rule (`AGENTS.md`, "Tooltips"). For example, the filmstrip "Filter" button has both a visible label and a tooltip. Sharpr's rule that every icon-only control needs an accessible name already covers the other half.
10. **Use Libadwaita's animation helpers.** `AdwTimedAnimation` respects `gtk-enable-animations` and has easing presets. Strata wrote its own cubic Béziers only because it has no Libadwaita. For any new reveal, such as search, the Info panel, or chips, use an ease-out curve for entering and ease-in for leaving, matching Strata's two named curves.

### Tier 2: possible, but changes a documented rule

11. **A single unified top row,** Strata-style: sidebar toggle | page switcher | search | actions | window buttons, spanning the window above all panes. Adwaita can do this with one `AdwHeaderBar` in a `ToolbarView` that wraps the split views. However, it changes how the panes collapse at the `max-width: 1500px` and `800px` breakpoints, and it departs from the GNOME pattern of one header per pane. Prototype it on a branch and let the user compare screenshots. Don't decide it on paper.
12. **A bundled interface font.** Strata registers JetBrains Mono (variable font, 300 KB, OFL) through Fontconfig FFI, which needs `unsafe`, and sets it as the GTK font for the whole UI. For Sharpr:
    - Don't make the whole UI monospace. Filenames and menus read worse, and the GNOME default (Adwaita Sans and Adwaita Mono on GNOME 48+) already fits.
    - If a bundled face is wanted for the metadata chips, the Flatpak can ship it in `/app/share/fonts`, where Fontconfig finds it with no FFI and no `unsafe`. The native launcher build would need the same file installed. Weigh that against tip 4, which costs nothing.
13. **A contextual shortcut footer.** Strata's footer is good for a keyboard-first file manager. In Sharpr, it would conflict with principle 6 ("Quiet interface"). Sharpr already has the Ctrl+? overlay (`help-overlay.ui`). Skip the footer unless keyboard discoverability becomes a real complaint.

### Tier 3: don't copy

- **Custom window buttons.** Strata redraws minimize, maximize, and close to match its icons. In Adwaita, the standard buttons already follow the user's `gtk-decoration-layout` (left or right, which buttons appear) and the platform's accessibility behavior. Reimplementing them loses both for a small visual gain.
- **Tinted theme backgrounds.** Strata's default theme, Azure Glow, is navy (`#03111e`). The 95-theme catalog is perfect for a file manager and wrong for a wallpaper curator: a colored canvas shifts how an image's colors are perceived. If Sharpr ever adds theming, the viewer and Compare canvas should stay neutral (black or neutral grey), and only the accent should follow the theme. That matches the current "follow the system accent" behavior Libadwaita already gives for free.
- **A hand-written theme engine.** Strata needs one because it has no Libadwaita. Sharpr gets light, dark, high-contrast, and accent color from `AdwStyleManager`.

## 4. Icons

What Strata does (`AGENTS.md`, "Icons"; `src/assets.rs`; `THIRD_PARTY_LICENSES.md`):

- **Two sources, each with one job.** [Lucide](https://lucide.dev) (ISC license) supplies every interface icon, about 100 SVGs in `data/icons/scalable/actions/`. Catppuccin icons are used only for code and file types.
- **Namespaced names.** Every bundled icon is named `strata-*` (and `strata-ctp-*`), so it can never collide with or be replaced by the user's icon theme. Icons ship in a gresource registered with `IconTheme::add_resource_path`.
- **Geometry is never edited.** Lucide icons are copied unchanged. Their stroke color is set to a placeholder that the code string-replaces with the accent at render time (`recolor_icon_source`), then caches. When the theme changes, all registered icons are recolored.
- **Weight compensation.** Lucide draws 2-unit strokes on a 24-unit grid, while Catppuccin draws 1-unit strokes on 16. Strata scales strokes so icons placed side by side read at the same weight. It also thins strokes slightly on large icons (`icon_stroke_factor`). This detail is a big part of why Strata's icons look crisp.
- **Provenance is recorded.** The upstream version (Lucide 1.35.0) and license text are kept in `THIRD_PARTY_LICENSES.md`. A script regenerates the generated icon map, so it is never edited by hand.

What to take for Sharpr:

- **Keep system icons for standard actions,** such as open folder, menu, close, and zoom. They follow the user's icon theme, which GNOME users expect.
- **Bundle namespaced icons only for Sharpr's own concepts,** where no system icon says the right thing: bucket, add to bucket, Compare, crop, WebP export, below-4K, and duplicates. Name them `sharpr-*-symbolic`, put them under the existing `/io/github/hebbihebb/Sharpr` resource prefix (already registered with the icon theme in `sharpr/src/app.rs`), and record the source and version as Strata does.
- **Source.** GNOME's *Icon Library* app symbolic icons (CC0) match Adwaita's style most closely. Lucide matches Strata's look; it is an ISC license, and its stroke style is lighter and rounder than Adwaita's.
- **Caveat before choosing Lucide.** GTK recolors `-symbolic` icons by overriding *fill*, so stroke-only Lucide SVGs may not follow the theme color unchanged. That is why Strata does its own recoloring. Either convert the strokes to filled outlines once (for example with Inkscape's *Stroke to Path*) and commit the converted files, or verify stroke recoloring on the GNOME 50 runtime first. Don't copy Strata's runtime recoloring code; it exists to tint icons with the accent, which Sharpr doesn't need.

## 5. Agent setup and documentation

Strata is mostly agent-written: about 38 of its last 50 commits carry the owner as co-author. Its setup is similar in spirit to Sharpr's `AGENTS.md`, but it is larger and enforces more by tooling.

| Strata practice | Where | Sharpr today | Worth borrowing? |
| --- | --- | --- | --- |
| **Owner-consent clause:** every rule is a default the owner can override for the current task; a direct request is consent, and skipped checks must be reported, never shown as passed | `AGENTS.md`, top | Implicit | **Yes.** One short paragraph stops agents from refusing or re-asking when the user has already said "do it". |
| **Rules enforced by Clippy:** `clippy.toml` `disallowed-methods` bans raw key handlers outside `ui::keys` | `clippy.toml` | Invariants are prose only | **Yes, for data safety.** Ban `std::fs::write`, `rename`, `remove_file`, `File::create`, and `OpenOptions::new` outside the one module allowed to write (`file_ops`, export). Each allowed call site carries `#[expect(clippy::disallowed_methods, reason = "…")]`. "Originals are never modified" then becomes a compile error, not a hope. |
| **Strict `[lints]` table:** `unsafe_code = deny`, `unwrap_used = warn`, `allow_attributes_without_reason = deny`, `dbg_macro = deny` | `Cargo.toml` | None | Yes. `allow_attributes_without_reason` alone makes agents justify every suppression. Start with warnings, then tighten. |
| **Test-value rules:** no layout or pixel tests, no tests that repeat constants, every test parameter must change behavior, find the existing coverage owner before adding a test | `AGENTS.md`, "Test value" | Risk table only | Yes. Add 3–4 lines; this curbs agents' habit of padding the suite. |
| **Tests in an adjacent `tests.rs`** declared with `#[cfg(test)] mod tests;` | `AGENTS.md` | Mixed | Optional. Shorter production files help agents read them, but it isn't worth a migration. Apply it only to new modules. |
| **Scoped validation with explicit escalation:** rerun the failing test, not the whole suite; run the full suite only with named evidence of broad impact | `AGENTS.md`, `docs/e2e-testing.md` | Proportional table | Sharpr's table already does this. Add one sentence: "after a failure, rerun that test first". |
| **GUI isolation:** GTK tests run on a private Xvfb and a private D-Bus, never the user's display; GTK tests must not skip silently (`STRATA_REQUIRE_GTK_TESTS=1`) | `AGENTS.md`, `.cursor/CLOUD.md` | `gtk_tests.rs`; temp-directory rule for data | **Yes,** especially with two agents running in parallel on this machine. An agent test opening windows on the user's session is the GUI version of touching real data. |
| **Machine runbook separate from rules:** `.cursor/CLOUD.md` records verified commands, versions, timings, and known harmless warnings for one machine, with the date verified | `.cursor/CLOUD.md` | `docs/reference/development.md` mixes both | Low priority; Sharpr has one machine. |
| **Docs state what they are:** "This document is the target product specification, not a completion report." | `docs/10xer-mode.md` | `ROADMAP.md` and `product.md` are clear | Already good. |
| **Generated code is regenerated, never edited:** a script owns `catppuccin.rs` | `AGENTS.md`, "Icons" | `packaging/cargo-sources.json` is similar | Already followed. |
| **Comment policy in `CLAUDE.md`:** comment only what isn't obvious, briefly | `CLAUDE.md` | "Match surrounding code" | Optional one-liner. |
| **Spelling check in CI** (`typos.toml`) | `typos.toml` | None | Low value for a one-person repo. |
| **PR template with required visual evidence**, captures never committed | `.github/` | No PRs by design | No. Sharpr's equivalent is "tell the user exactly what to test". |
| **Skills pinned by lockfile** (`skills-lock.json`, `npx skills`) | repo root | `.claude/skills` and memories | No. It adds Node tooling for one skill. |

Strata's `AGENTS.md` is about 20 KB and growing. Sharpr's shorter file, which points to reference docs, is easier for an agent to follow fully. Borrow individual rules, not the length.

## 6. If the user wants to act on this

The Tier 1 items could become one small "chrome polish" milestone after the current Phase 1 data-safety work. They touch only CSS, header construction, and icon assets. Suggested acceptance criteria:

- All CSS lives in one `style.css`.
- No hex or `rgba` color literals remain in `sharpr/src/ui`, except the token definitions.
- The headers are flat and untitled where the pane is self-evident.
- Numbers use tabular figures.
- Light, dark, and high-contrast modes all stay readable at 1024 × 600.
- Filmstrip scroll and folder-switch bench logs are unchanged, because the skeleton and glow touch the cell factory's CSS.

The Clippy file-write ban is independent and could land at any time as part of the data-safety work. Tier 2 items need screenshots and a user decision first.
