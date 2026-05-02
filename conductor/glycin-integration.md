# Glycin Viewer Integration Plan

## Objective
Refactor the image preview architecture in Sharpr to use the GNOME `glycin` library instead of the custom `turbojpeg`/`jxl-oxide` pipeline. This will simplify the codebase, increase security via sandboxing, and provide robust format support, relying on a pure on-demand loading strategy rather than complex prefetching.

## Background & Motivation
The current architecture uses a custom background thread pool (`PreviewWorker`) and pure Rust crates to decode images into raw RGBA bytes. While highly optimized for zero-latency prefetching, it requires significant custom code and manual memory management. Using `glycin` aligns Sharpr more closely with modern GNOME technologies (like the Loupe image viewer), delegating the complex task of secure, multi-format image decoding to standard desktop libraries. The user has elected to use a **Pure On-Demand** loading strategy, meaning we will remove the existing prefetching logic to prioritize architectural simplicity.

## Key Files & Context
- `Cargo.toml`: Needs the `glycin` dependency.
- `src/ui/viewer.rs`: The core `ViewerPane` where images are requested and displayed. Needs to be updated to use `glycin::Loader` instead of `PreviewHandle`.
- `src/ui/window.rs`: Where the application wires up workers and handles prefetch callbacks. Needs trimming.
- `src/image_pipeline/worker.rs` & `mod.rs`: The custom preview decoding logic, which can be mostly removed (keeping `MetadataWorker`).
- `src/model/library.rs`: The state cache, which can have its prefetch tracking removed.

## Implementation Steps

### 1. Dependency Updates
- Add `glycin = "2"` to `Cargo.toml`.

### 2. Remove Prefetching and Custom Preview Worker
- **`src/image_pipeline/worker.rs` & `mod.rs`:**
  - Delete `PreviewWorker`, `PreviewHandle`, and all related decode functions (`decode_preview`, `decode_jpeg_rgba_scaled`, etc.).
  - Retain `MetadataWorker` and `MetadataHandle` as they are still used for the `MetadataChip`.
- **`src/model/library.rs`:**
  - Remove methods and fields related to prefetching: `take_prefetch`, `insert_prefetch`, `prefetch_in_flight`, `clear_prefetch_in_flight`, `mark_prefetch_in_flight`.
- **`src/ui/window.rs`:**
  - Remove the initialization of `PreviewWorker` and the `prefetch_result_rx` listener loop.
  - Remove calls to `trigger_prefetch` and `queue_prefetch`.

### 3. Refactor ViewerPane (`src/ui/viewer.rs`)
- Remove `preview_handle` from `imp::ViewerPane`.
- Remove the `set_preview_worker` method and its background loop.
- **Update `load_image`:**
  - Instead of submitting to a custom worker queue, spawn a local async task on the GTK main thread:
    ```rust
    let picture = imp.picture.clone();
    let spinner = imp.spinner.clone();
    let error_label = imp.error_label.clone();
    let path_clone = path.clone();
    // ... update generation checks ...
    
    glib::MainContext::default().spawn_local(async move {
        let file = gio::File::for_path(&path_clone);
        let result = async {
            let image = glycin::Loader::new(file).load().await?;
            let frame = image.next_frame().await?;
            Ok::<_, glycin::Error>(frame.texture())
        }.await;
        
        spinner.stop();
        spinner.set_visible(false);
        
        match result {
            Ok(texture) => {
                picture.set_paintable(Some(texture.upcast_ref::<gdk4::Paintable>()));
            }
            Err(e) => {
                error_label.set_text(&format!("Could not load image: {}", e));
                error_label.set_visible(true);
            }
        }
    });
    ```
- **Adapt Transform/Edits:**
  - Update `apply_transform` to extract raw bytes via `imp.picture.paintable().and_downcast::<gdk4::Texture>().map(|t| t.download())` instead of relying on the removed `current_rgba` cache. (Alternatively, disable the manual rotate/flip UI if it becomes too slow/complex without raw bytes, per the user's allowance for UI changes).

## Verification
- **Build & Run:** Ensure the application builds successfully with the `glycin` dependency.
- **Navigation:** Verify that clicking an image in the filmstrip successfully loads and displays it in the viewer using `glycin` without crashing.
- **Format Support:** Test loading various formats (JPEG, PNG) to ensure `glycin-loaders` handles them correctly.
- **Performance:** Verify that the UI remains responsive (no main thread blocking) during the async `glycin` load.

## Migration & Rollback
If `glycin` proves incompatible or too slow on certain distributions without `glycin-loaders` installed, this refactor can be reverted by checking out the previous commit on this `experiments` branch.