//! Wallslæd, the user's wallpaper rotator, remembers favorites and "never
//! show" choices by path. After Replace with upgrade (D49) and its Undo,
//! Sharpr tells it which paths changed, over the local session bus, so those
//! choices follow the file. Local IPC only: never a network call. The call
//! never starts Wallslæd (`NO_AUTO_START`) and never fails a replacement;
//! when Wallslæd isn't running, or is a version without `Relink`, the result
//! says so.

use std::path::PathBuf;

const BUS_NAME: &str = "io.github.hebbihebb.Wallslaed";
const OBJECT_PATH: &str = "/io/github/hebbihebb/Wallslaed";
const INTERFACE: &str = "io.github.hebbihebb.Wallslaed";

/// Sends `(old, new)` path pairs to Wallslæd; returns whether it took them.
pub type RelinkFn = dyn Fn(&[(PathBuf, PathBuf)]) -> bool + Send + Sync;

/// The real call. Any error, including no owner (not running) and
/// `UnknownMethod` (an older Wallslæd), counts as not delivered.
pub fn relink(pairs: &[(PathBuf, PathBuf)]) -> bool {
    use gtk4::gio;
    use gtk4::prelude::ToVariant;
    if pairs.is_empty() {
        return true;
    }
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return false;
    };
    let pairs: Vec<(String, String)> = pairs
        .iter()
        .map(|(old, new)| {
            (
                old.to_string_lossy().into_owned(),
                new.to_string_lossy().into_owned(),
            )
        })
        .collect();
    bus.call_sync(
        Some(BUS_NAME),
        OBJECT_PATH,
        INTERFACE,
        "Relink",
        Some(&(pairs,).to_variant()),
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        2000,
        gio::Cancellable::NONE,
    )
    .is_ok()
}

/// The pairs to send for a finished replacement: the original's (and the
/// other trashed copies') choices move to the new file; for its Undo, back
/// from the new file to the original.
pub fn pairs_for(done: &crate::upgrade::Replaced, undo: bool) -> Vec<(PathBuf, PathBuf)> {
    if undo {
        return vec![(done.placed.clone(), done.original.clone())];
    }
    std::iter::once(&done.original)
        .chain(&done.also_trashed)
        .filter(|old| **old != done.placed)
        .map(|old| (old.clone(), done.placed.clone()))
        .collect()
}
