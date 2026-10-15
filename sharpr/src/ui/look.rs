//! Sharpr's look (Phase 9, D26), prototype. Libadwaita keeps light and dark,
//! the accent colour, and toasts; this stylesheet gives Sharpr its own shape:
//! sharp corners, a monospace readout for numbers and filters, hairlines
//! instead of raised bars, compact menus and pickers, and images on a
//! neutral canvas.

use gtk4::{gdk, prelude::*};

const CSS: &str = r#"
window.sharpr {
  --sharpr-mono: "JetBrains Mono", "Adwaita Mono", monospace;
  --sharpr-radius: 4px;
}

/* The top row: one flat bar per column, all 42 px, so the hairline below
   runs straight across. */
headerbar.sharpr-top {
  min-height: 42px;
  padding: 0 4px;
  box-shadow: inset 0 -1px alpha(currentColor, 0.10);
}
/* A pane with two top bars (the viewer's header and chip bar) gets 3 px
   above and below from Libadwaita, which would push its hairline down. */
window.sharpr toolbarview > .top-bar .collapse-spacing {
  padding-top: 0;
  padding-bottom: 0;
}
/* Libadwaita pads the inside 6-7 px; with 4 px the window buttons fit. */
headerbar.sharpr-top > windowhandle > box {
  padding: 4px 6px;
}
headerbar.sharpr-top button,
headerbar.sharpr-top menubutton > button {
  min-height: 26px;
  min-width: 26px;
  padding: 2px 6px;
}
headerbar.sharpr-top entry {
  min-height: 28px;
}

/* Sharp corners: the name, taken literally. */
window.sharpr button:not(.circular),
window.sharpr menubutton > button,
window.sharpr entry,
window.sharpr spinbutton,
window.sharpr dropdown > button,
window.sharpr .navigation-sidebar > row {
  border-radius: var(--sharpr-radius);
}
window.sharpr windowcontrols > button {
  border-radius: 999px;
}
/* Window buttons a little smaller, so every bar is the same 42 px. */
headerbar.sharpr-top windowcontrols > button {
  padding: 3px;
}
headerbar.sharpr-top windowcontrols > button > image {
  padding: 3px;
}
window.sharpr popover > contents {
  border-radius: 8px;
}
window.sharpr .osd.toolbar {
  border-radius: 6px;
}

/* The readout face: numbers, filters, the corner pill, key hints. */
window.sharpr .numeric,
window.sharpr .sharpr-keys,
window.sharpr .metadata-osd {
  font-family: var(--sharpr-mono);
}
/* The variant: the readout face everywhere. */
window.sharpr.sharpr-mono-all,
window.sharpr.sharpr-mono-all * {
  font-family: var(--sharpr-mono);
}
window.sharpr.sharpr-mono-all {
  font-size: 0.94em;
}

/* Compact menus: 26 px rows, tight padding. */
popover.menu > contents {
  padding: 4px;
}
popover.menu modelbutton {
  min-height: 26px;
  padding: 0 8px;
  border-radius: 3px;
  font-size: 0.93em;
}
popover.menu separator {
  margin: 3px 2px;
}
popover.menu label.title {
  font-size: 0.85em;
  padding: 2px 8px;
}

/* Compact pickers. */
popover.sharpr-picker > contents {
  padding: 8px;
}
popover.sharpr-picker button {
  min-height: 26px;
  padding: 2px 10px;
}
popover.sharpr-picker row {
  min-height: 28px;
}
/* Libadwaita's settings rows are 50 px tall; 34 px in a picker. */
popover.sharpr-picker row > box.header {
  min-height: 34px;
}
popover.sharpr-picker row button {
  min-height: 22px;
  padding: 2px 8px;
}

/* Preferences and shortcuts share the main window's face and geometry.
   The content scrolls within the sheet at the minimum window size. */
window.sharpr .sharpr-sheet headerbar {
  min-height: 42px;
  box-shadow: inset 0 -1px alpha(currentColor, 0.10);
}
window.sharpr .sharpr-sheet row > box.header {
  min-height: 40px;
}
window.sharpr .sharpr-sheet button:not(.circular) {
  min-height: 26px;
  padding: 2px 10px;
}
window.sharpr .sharpr-sheet list.boxed-list {
  border-radius: var(--sharpr-radius);
}

/* A tighter sidebar: it is for jumping, not reading. */
window.sharpr .sharpr-sidebar .navigation-sidebar > row {
  min-height: 24px;
  padding: 0 6px;
}
window.sharpr .sharpr-sidebar label {
  font-size: 13px;
}
window.sharpr .sharpr-sidebar .caption-heading {
  font-size: 11px;
  letter-spacing: 0.06em;
  opacity: 0.7;
}
window.sharpr .sharpr-sidebar .caption.dim-label {
  font-family: var(--sharpr-mono);
  font-size: 11px;
}
window.sharpr .sharpr-filmstrip .caption {
  font-size: 11px;
}

/* Filter chips: small square-cornered tags in the readout face. */
window.sharpr .sharpr-filter-chip > button {
  min-height: 22px;
  padding: 0 5px;
  border-radius: 0;
  font-family: var(--sharpr-mono);
  font-size: 12px;
  font-weight: 500;
  color: var(--window-fg-color);
  background: color-mix(in srgb, var(--accent-bg-color) 14%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 45%, transparent);
}
window.sharpr .sharpr-filter-chip > button:first-child {
  border-top-left-radius: 3px;
  border-bottom-left-radius: 3px;
  padding-left: 7px;
}
window.sharpr .sharpr-filter-chip > button:last-child {
  border-top-right-radius: 3px;
  border-bottom-right-radius: 3px;
  min-width: 18px;
  padding: 0 3px;
  -gtk-icon-size: 12px;
}
window.sharpr .sharpr-filter-chip.negative > button {
  background: color-mix(in srgb, var(--error-bg-color) 14%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--error-color) 45%, transparent);
}

/* The corner pill: a compact readout, sharp, no glow. */
window.sharpr .metadata-osd {
  padding: 5px 9px;
  border-radius: 4px;
  background-color: rgba(16, 16, 18, 0.84);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08);
  font-size: 12px;
}
window.sharpr .metadata-osd-meta {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.92);
}
window.sharpr .metadata-osd-below {
  font-size: 10px;
  font-weight: 700;
  color: #f0a83a;
  padding: 0 4px;
  border-radius: 2px;
  box-shadow: inset 0 0 0 1px rgba(240, 168, 58, 0.6);
}
window.sharpr .metadata-osd-buckets {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.6);
}

window.sharpr .survey-tile {
  padding: 8px;
  border-radius: 4px;
  color: white;
}
window.sharpr gridview.sharpr-survey { background: #1c1c1e; }

/* Key hints in Compare and crop. */
window.sharpr scrollbar.sharpr-fast-scroll.vertical {
  min-width: 14px;
}
window.sharpr scrollbar.sharpr-fast-scroll.vertical slider {
  min-width: 10px;
  min-height: 44px;
  margin: 2px;
  border-radius: 4px;
}
window.sharpr .compare-candidates listview {
  background: transparent;
}
window.sharpr .compare-candidates row {
  padding: 2px;
}
window.sharpr .compare-candidate {
  padding: 4px;
  border: 1px solid transparent;
  border-radius: 4px;
}
window.sharpr .compare-candidate.compare-current {
  border-color: var(--accent-color);
}
window.sharpr .compare-side {
  padding: 1px 5px;
  border-radius: 4px;
  background: var(--accent-bg-color);
  color: var(--accent-fg-color);
  font-weight: bold;
}
window.sharpr .sharpr-keys {
  font-size: 11px;
  color: alpha(currentColor, 0.65);
}
window.sharpr .sharpr-keys-osd {
  background-color: rgba(16, 16, 18, 0.72);
  color: rgba(255, 255, 255, 0.78);
  padding: 3px 10px;
  border-radius: 4px;
}

/* A neutral canvas behind images, in both themes. */
.sharpr-canvas {
  background-color: #1c1c1e;
}

/* The corner chip shows while the pointer moves, and fades when still. */
.sharpr-chip {
  transition: opacity 250ms ease-out;
}
.sharpr-chip.sharpr-idle {
  opacity: 0;
  transition: opacity 600ms ease-in;
}
"#;

/// Loads the stylesheet once for the whole display, above the older
/// per-widget sheets it restyles.
pub fn install(widget: &impl IsA<gtk4::Widget>) {
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if DONE.with(|d| d.get()) {
        return;
    }
    let Some(display) = gdk::Display::default() else {
        return;
    };
    super::fonts::install(widget);
    DONE.with(|d| d.set(true));
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(CSS);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 10,
    );
}
