# Viewer Thumbnail Placeholder

## Objective
Show a low-resolution thumbnail placeholder in the `ViewerPane` while a full-resolution image is decoding on the background thread. This improves perceived performance for large files.

## Key Files & Context
- `src/ui/viewer.rs`: The `load_image` function is responsible for setting the image. It handles cache checks and dispatches a background task if the image is missing from the fast path.
- `src/model/library.rs`: The `LibraryManager` manages the active and LRU thumbnail caches (`cached_thumbnail(&Path) -> Option<Texture>`).

## Implementation Steps
1. In `src/ui/viewer.rs`, within `FilmstripPane` or `ViewerPane` `load_image` method, locate the line where `imp.picture.set_paintable(None::<&gdk4::Paintable>);` is currently called (this clears the previous image immediately).
2. Inside `load_image` (`src/ui/viewer.rs`), locate the "Slow path" block where the spinner is started.
3. Before or while starting the spinner, retrieve the cached thumbnail for the requested path via `imp.state.borrow().as_ref().and_then(|rc| rc.borrow().library.cached_thumbnail(&path))`.
4. If a `Texture` is returned, wrap it as a `Paintable` and call `imp.picture.set_paintable(Some(texture.upcast_ref::<gdk4::Paintable>()))`.
5. Ensure that the zoom/scaling behaviour preserves the aspect ratio and scales up appropriately (the existing picture properties should already handle `can-shrink=true` and fitting).

## Verification & Testing
1. Launch the application with `cargo run`.
2. Scroll to a large (e.g., 20MB or >4k) image in the filmstrip.
3. Click to select the large image.
4. Verify that the thumbnail instantly appears scaled-up in the viewer while the loading spinner is active.
5. Verify that the full-resolution image replaces the thumbnail smoothly once loading is complete.
6. Run `cargo clippy -- -D warnings` and `cargo test`.
