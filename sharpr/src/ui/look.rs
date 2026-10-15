//! Look prototype (Phase 9, step 2b): the hybrid style. Libadwaita keeps light
//! and dark, the accent colour, and toasts; this stylesheet makes the top row,
//! menus, and pickers compact and puts images on a neutral canvas.

use gtk4::gdk;

const CSS: &str = r#"
/* One quiet top row. */
headerbar.sharpr-top {
  min-height: 38px;
  padding: 0 4px;
  box-shadow: inset 0 -1px alpha(currentColor, 0.10);
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
.sharpr-location {
  margin: 0 6px;
  font-feature-settings: "tnum";
}

/* Compact menus: 26 px rows, tight padding. */
popover.menu > contents {
  padding: 4px;
}
popover.menu modelbutton {
  min-height: 26px;
  padding: 0 8px;
  border-radius: 4px;
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

/// Loads the prototype stylesheet once for the whole display.
pub fn install() {
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(CSS);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
