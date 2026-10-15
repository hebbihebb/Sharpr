//! The approved UI font, bundled for native and Flatpak builds (D27).
//!
//! Pango needs filenames. Anonymous, process-owned files expose the embedded
//! bytes without installing fonts or writing a cache into the user's home.

use std::cell::OnceCell;
use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};

use gtk4::prelude::*;

const RESOURCES: [&str; 2] = [
    "/io/github/hebbihebb/Sharpr/fonts/JetBrainsMono.ttf",
    "/io/github/hebbihebb/Sharpr/fonts/JetBrainsMono-Italic.ttf",
];

/// Register once on GTK's font map, before applying the stylesheet. Retain
/// the handles: Fontconfig may reopen a font when a new weight is requested.
pub fn install(widget: &impl IsA<gtk4::Widget>) {
    thread_local! {
        static FILES: OnceCell<Vec<File>> = const { OnceCell::new() };
    }
    FILES.with(|files| {
        files.get_or_init(|| {
            let Some(font_map) = widget.pango_context().font_map() else {
                crate::bench_warn!(
                    "ui.fonts.unavailable",
                    serde_json::json!({"error": "no font map"})
                );
                return Vec::new();
            };
            RESOURCES
                .into_iter()
                .filter_map(|resource| match add_font(&font_map, resource) {
                    Ok(file) => Some(file),
                    Err(error) => {
                        // Font failure must not prevent browsing; CSS retains
                        // the platform monospace fallback.
                        crate::bench_warn!(
                            "ui.fonts.unavailable",
                            serde_json::json!({
                                "resource": resource, "error": error.to_string(),
                            })
                        );
                        None
                    }
                })
                .collect()
        });
    });
}

fn add_font(font_map: &pango::FontMap, resource: &str) -> Result<File, Box<dyn std::error::Error>> {
    let bytes = gio::resources_lookup_data(resource, gio::ResourceLookupFlags::NONE)?;
    // SAFETY: the name is a valid C string and memfd_create takes no pointers
    // into Rust-owned mutable data. A negative return owns no descriptor.
    let fd = unsafe { libc::memfd_create(c"sharpr-font".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: this is a fresh descriptor owned solely by this File. CLOEXEC
    // keeps it out of the visual-search child processes.
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(bytes.as_ref())?;
    font_map.add_font_file(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
    Ok(file)
}
