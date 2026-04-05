# Image and Thumbnail Loading Research: Sharpr vs. digiKam

This document analyzes the current image and thumbnail handling in Sharpr and compares it with the industry-standard approach used by digiKam to identify performance bottlenecks and potential improvements.

## 1. Current Sharpr Architecture

### Thumbnail Generation
- **Mechanism:** Background thread pool using the `image` crate.
- **Process:** Decodes the *entire* source image into memory, applies EXIF orientation, resizes using Lanczos3 (high quality but slow), and encodes to PNG.
- **Storage:** Individual PNG files in `~/.cache/sharpr/thumbnails-r1/`. Filenames are hashes of path + metadata (size, mtime).
- **Resolution:** Fixed at 160px height.

### Caching
- **Disk Cache:** Standard filesystem-based cache. Every thumbnail is a separate file.
- **Memory Cache:** 
    - `LibraryManager` holds up to 500 `gdk4::Texture` objects (LRU).
    - `prefetch_cache` stores up to 4 pre-decoded raw RGBA buffers for upcoming images.

### Image Loading (Full View)
- **Mechanism:** Decodes the full image on demand.
- **Prefetching:** Attempts to decode the next/previous images in the background into the 4-slot prefetch cache.

---

## 2. digiKam Architecture (The "Instant" Loading Secret)

### Multi-Database System
digiKam separates data into specialized SQLite/MySQL databases:
- `digikam4.db`: Core metadata.
- `thumbnails-digikam.db`: **Dedicated thumbnail storage.**
- `similarity.db`: Image fingerprints (hashes).

### Thumbnail Database Schema (Deep Dive)
Analysis of the `thumbsdb` library in digiKam reveals a highly optimized relational schema:
- **`Thumbnails` Table:** Stores the actual BLOB data (`data` column), `type` (PGF, JPEG, etc.), `modificationDate`, and an `orientationHint`.
- **Relational Mapping:** Instead of mapping a path directly to a BLOB, digiKam uses intermediate lookup tables:
    - **`UniqueHashes`:** Maps `(uniqueHash, fileSize)` -> `thumbId`. This allows the application to find a thumbnail even if a file is renamed or moved, as long as the content is the same.
    - **`FilePaths`:** Maps `path` -> `thumbId` for fast path-based lookups.
- **Normalization:** This structure ensures that if multiple files have the same content (duplicates), only one thumbnail BLOB is stored in the database.

### Thumbnail Storage & Format
- **Database BLOBs:** Thumbnails are stored as BLOBs, eliminating filesystem overhead.
- **PGF (Progressive Graphics File):** A wavelet-based format that is faster to decode and more compact than PNG/JPEG for thumbnails.
- **Orientation Hints:** Storing the EXIF orientation as an integer in the database allows the UI to rotate the thumbnail on-the-fly without re-encoding the binary data.

### Speed Optimizations
- **Embedded Previews:** digiKam prioritizes extracting **embedded JPEGs** from EXIF/RAW metadata.
- **Single-Query Retrieval:** Uses `INNER JOIN` to fetch metadata and BLOB data in a single database round-trip.
- **WAL Mode & Prepared Statements:** SQLite Write-Ahead Logging and pre-compiled SQL queries maximize throughput.
- **Database Triggers:** Automatically cleans up lookup entries when a thumbnail is deleted, maintaining referential integrity without application-level overhead.
- **Aggressive Memory Caching:** Configurable RAM cache for pre-decoded images.

### Priority & Concurrency Model
- **Task-Based Architecture:** Every thumbnail request is a `ThumbnailLoadingTask` added to a global queue.
- **Visual Priority:** digiKam uses a **prioritized queue**. Thumbnails for images currently visible in the viewport are moved to the front of the queue, while background pre-generation tasks (for images off-screen) are given lower priority.
- **Worker Threads:** A pool of worker threads picks tasks from the prioritized queue, ensuring the UI remains responsive and currently viewed images load first.

---

## 2b. digiKam Deep Implementation Details

This section documents the concrete algorithms and data structures behind digiKam's performance characteristics, sourced from direct codebase analysis. These are the transferable insights for reimplementation.

### 2b.1 Thumbnail Size Tier System (`thumbnailsize.h`)

DigiKam defines discrete size constants rather than arbitrary pixel counts:

```cpp
enum Size {
    Tiny        = 32,
    VerySmall   = 64,
    MediumSmall = 80,
    Small       = 100,
    Medium      = 142,
    Large       = 160,   // ← same as Sharpr's current size
    Huge        = 256,
    HD          = 512,
    MAX         = 1024
};
```

**The critical strategy:** Only ONE thumbnail version is stored on disk (typically Huge=256 or HD=512). All display sizes scale down from this at paint time. This means a filmstrip at 160px and a grid at 100px both hit the same cache entry. On HiDPI screens (`devicePixelRatio > 1.0`) it stores at HD(512) or MAX(1024) to avoid blurriness.

**Sharpr implication:** Storing at 256px instead of 160px allows the same cached file to serve a larger icon view or detail strip — no re-generation needed. The cost is slightly larger cache files (~2.5x), which is acceptable given SSD speeds.

---

### 2b.2 Three-Thread Priority Architecture (`itemthumbnailmodel.cpp`, `managedloadsavethread.h`)

DigiKam uses **three separate loading threads** with distinct priorities:

| Thread | Priority | Purpose |
| :--- | :--- | :--- |
| `loadingThread` | `NormalPriority` | Visible-in-viewport thumbnails; UI-critical path |
| `storageThread` | Medium | Disk/DB cache lookups only; no full decode |
| `preloadThread` | `LowestPriority` | Off-screen pre-generation; produces no UI textures |

The system defines six loading policies that govern queue insertion:

| Policy | Behavior | When Used |
| :--- | :--- | :--- |
| `FirstRemovePrevious` | Cancel all pending, start immediately | Folder switch / jump |
| `Prepend` | Insert at front; wait for current task | High-priority loads |
| `SimpleAppend` | Normal queue append | Standard scroll loading |
| `Preload` | Lowest priority; auto-cancelled when normal load arrives | Background prefetch |

**Sharpr implication:** The current flat 4-thread pool means background pregeneration competes with visible-tile loads. Splitting into two `async-channel` pairs — `(tx_visible, rx_visible)` run by 3 workers, `(tx_preload, rx_preload)` run by 1 lowest-priority worker — would eliminate this contention. The generation-counter approach Sharpr already uses maps to `FirstRemovePrevious`.

---

### 2b.3 Task Queue Deduplication

When a high-priority task arrives, digiKam scans the pending queue and removes lower-priority duplicates for the same file path before inserting. This prevents pathological behavior where rapidly scrolling a folder queues hundreds of load requests for files that scroll offscreen before they're processed.

**Sharpr implication:** The existing generation counter handles folder-switch cancellation, but fast scroll within a folder can still accumulate stale requests in the worker channel. A `HashSet<PathBuf>` of pending paths, protected by a `Mutex`, would allow filtering on enqueue. Clear the set when the generation changes.

---

### 2b.4 RAW File Half-Size Decode (`thumbnailcreator_engine.cpp`)

For RAW format files, digiKam forces a faster decode mode:

```cpp
fastRawSettings.rawPrm.halfSizeColorImage = true;  // Decode at 50% resolution
fastRawSettings.rawPrm.sixteenBitsImage   = false; // Force 8-bit output
fastRawSettings.optimizeTimeLoading();
```

Decoding at half-size means 25% of the total pixels compared to full-res. For a 24MP RAW (6000×4000), this produces a 3000×2000 result before the Lanczos downsample to 256px — still far more than needed, but 4× fewer pixels to decode. Combined with 8-bit output (vs 16-bit), total decode time drops dramatically.

**Sharpr implication:** When LibRaw or the `rawloader` crate is used, the equivalent flag is `half_size` in LibRaw's `LibRaw_output_params_t`. For JPEG files, `turbojpeg` supports scaled decode (1/2, 1/4, 1/8) at the DCT level — reading 1/8 of a 4K JPEG directly produces a 500px intermediate, avoiding most decode work.

---

### 2b.5 Three-Key Database Lookup (`thumbnailcreator_database.cpp`)

Thumbnail lookup attempts three keys in priority order:

1. **`customIdentifier`** — arbitrary string key, used for detail/face regions (`"path:x,y,w,h"`)
2. **`uniqueHash + fileSize`** — SHA-256-like content hash paired with byte count; survives file renames and moves
3. **`filePath`** — direct path string; fast for local use but breaks on moves

The database schema has three separate lookup tables that all map to a single `Thumbnails` row (BLOB + format + orientationHint). This means renaming a file does not orphan its thumbnail — the hash lookup still finds it.

**Sharpr implication:** The content-hash + fileSize key is the most valuable. Sharpr's current filename scheme uses a path hash (not a content hash), so moving files invalidates the cache. Adding an optional content-hash lookup (computed lazily in background, stored in a small SQLite side-table) would make the cache robust to reorganization.

---

### 2b.6 Cache Invalidation via File Watch (`loadingcache.h`)

```cpp
class LoadingCacheFileWatch {
    QHash<QString, QPair<qint64, QDateTime>> m_watchHash;
    // tracks: path → (fileSize, modificationDateTime)
};
```

When a file changes, the watcher auto-purges matching entries from both the in-memory cache and invalidates the disk-cache entry. No polling — events come from the OS file watcher (`QFileSystemWatcher`).

**Sharpr note:** Sharpr already encodes `file_size + mtime` in the cache filename (`{hash}-{size}-{mtime_secs}-{mtime_nanos}.png`), which is functionally equivalent and correct. This is a validation that the approach is sound.

---

### 2b.7 Preload vs. Pregenerate Distinction (`thumbnailloadthread.h`)

DigiKam distinguishes two background operations:

- **`preload(identifier)`** — load at low priority, emit completion signal (warms both disk and memory cache)
- **`pregenerate(identifier)`** — ensure thumbnail exists on disk/in DB only; **does not load into RAM** (fills disk cache without memory cost)

The recommended folder-open sequence:
1. Load visible viewport tiles at `NormalPriority` → pixels on screen ASAP
2. `pregenerate` remaining folder images at `LowestPriority` → fills disk cache silently
3. Once user scrolls into new area: promote those pending items to `Prepend` priority

**Sharpr implication:** Currently all non-visible tiles queue to the same workers as visible tiles. Adding a `pregenerate` path that writes to disk cache but does not send a result back to the UI thread (skipping the `gdk4::MemoryTexture` allocation) would reduce memory pressure during initial folder scan.

---

### 2b.8 Embedded Preview Extraction Priority (`thumbnailcreator.cpp`)

The full load fallback chain:

1. **Database hit** (~microseconds) — decompress BLOB, done
2. **Exif/IPTC embedded JPEG** (~5–50ms) — camera-embedded preview, no full decode
3. **Full decode + Lanczos** (~100ms–2s depending on format and size)

Cameras embed a full-quality JPEG preview in every RAW file (and most JPEGs contain an Exif thumbnail). Extracting this is 10–100× faster than decoding the full image.

**Sharpr implication:** `rexiv2` (already a Sharpr dependency) exposes `get_preview_image()` and `get_thumbnail()`. This is a zero-new-dependency win. The embedded preview is often 1920×1280 or larger, sufficient for the filmstrip at any density setting.

---

## 2c. digiKam Full Image Preview Loading (The Most Important Section)

This is the mechanism responsible for near-instant image display in the viewer — the 4–5 second problem Sharpr currently has.

### Why Sharpr Is Slow (Root Cause)

Sharpr's `decode_image_rgba()` (`viewer.rs:904–917`) calls `image::ImageReader::decode()` on the full file. For a typical 20 MB JPEG this expands to ~60–80 MB of uncompressed RGBA in memory. The pure-Rust `image` crate provides no way to request a scaled decode — it always decodes every pixel at full resolution, then the result is scaled down to fit the viewer. This is the entire 4–5 seconds.

The main thread never blocks (the spinner runs), but the background thread doing the decode just takes that long. Prefetch only helps for the immediately adjacent image — any jump, folder open, or first view hits the slow path.

---

### The digiKam Fix: libjpeg DCT-Level Scaling

**File:** `core/dplugins/dimg/jpeg/dimgjpegloader_load.cpp:156–270`

libjpeg has a built-in feature: before decompression begins, you can set `cinfo.scale_denom` to 2, 4, or 8. The decompressor then **skips DCT coefficient blocks** during the decode itself — it never reads or processes those bytes. The output image is proportionally smaller.

```cpp
int scaledLoadingSize = 1024;  // viewer area in pixels
int imgSize = qMax(cinfo.image_width, cinfo.image_height);
int scale = 1;

while (scaledLoadingSize * scale * 2 <= imgSize)
    scale *= 2;                // find the largest safe denominator

if (scale > 8) scale = 8;
cinfo.scale_denom *= scale;   // set BEFORE jpeg_start_decompress()

jpeg_start_decompress(&cinfo);
// libjpeg now decodes at 1/scale resolution natively
```

**Concrete example** — 6000×4000 JPEG displayed in a 1280×960 viewer:

| Path | Pixels decoded | Typical time |
| :--- | ---: | ---: |
| Full decode (Sharpr today) | 24,000,000 | 4–5 s |
| 1/2 scale (`scale_denom=2`) | 6,000,000 | ~1.2 s |
| 1/4 scale (`scale_denom=4`) | 1,500,000 | ~300 ms |
| 1/8 scale (`scale_denom=8`) | 375,000 | ~80 ms |

For a 1280px-wide viewer and a 6000px JPEG, scale=4 is the correct choice — it produces a ~1500px intermediate that downscales to 1280 with zero quality loss visible at normal viewing distance.

**Rust equivalent:** The `turbojpeg` crate exposes `tjDecompressToYUVPlanes`/`tjDecompress2` with a `jpegSubsamp` + scale factor parameter. The `mozjpeg` crate (which wraps libjpeg-turbo directly) also exposes `Decompress::scale()`. Either gives access to the same `scale_denom` mechanism. The `image` crate does not.

---

### Stage 1: Embedded Preview (Fastest Path for Lossy Files)

**File:** `core/libs/threadimageio/preview/previewtask.cpp:192–323`

Before attempting any decode, digiKam calls `loadExiv2Preview()`. Most JPEGs written by cameras or Lightroom/Darktable contain a full-quality embedded JPEG in the EXIF block — typically 1920×1280 or larger. Extracting this avoids decoding the main image entirely.

```cpp
MetaEnginePreviews previews(m_loadingDescription.filePath);
if (loadExiv2Preview(previews, sizeLimit))
    break;  // Done — use the embedded JPEG, never touched main image data
```

For Sharpr's use case (JPEG-focused), this applies to:
- Camera-shot JPEGs with full embedded previews (Canon, Nikon, Sony all embed them)
- WebP files (sometimes contain EXIF preview)

`rexiv2` (already a Sharpr dependency) exposes `Metadata::get_preview_images()` which returns a list of embedded previews sorted by size. This is a **zero-new-dependency** optimization — just call it before triggering the full decode.

---

### In-Memory Preview Cache (Navigation Becomes Instant)

**File:** `core/libs/threadimageio/fileio/loadingcache.h:115–310`

This is what makes navigation feel instant after the first view. digiKam maintains an LRU cache of recently decoded `DImg` objects (full preview images, not thumbnails) in RAM:

- Keyed by: `filepath + scaled_size + color_profile`
- Thread-safe via `CacheLock`
- Size-limited (configurable, typically 256–512 MB)
- **Before any decode attempt**, the loading thread checks this cache. A cache hit costs ~5ms (hash lookup + texture upload)

A critical detail: the cache stores the **scaled preview** (e.g., the 1500px intermediate), not the full-res image. So revisiting a 50 MB image costs the same as revisiting a 1 MB image.

Sharpr currently has **no equivalent** — the 4-slot `prefetch_cache` in `library.rs` stores raw RGBA bytes but only for ±1 neighbors, and evicts them after navigation. Viewing an image, switching away, and switching back triggers a full re-decode.

---

### Prefetch Chain (Zero Perceived Latency for Sequential Browsing)

**File:** `core/libs/widgets/graphicsview/dimgpreviewitem.cpp:148–154, 299–310`

When image N finishes loading, digiKam immediately starts loading image N+1 at low priority on a separate thread:

```cpp
void DImgPreviewItem::setPreloadPaths(const QStringList& pathsToPreload) {
    d->pathsToPreload = pathsToPreload;
    preloadNext();  // starts loading N+1 immediately
}

// When N+1 finishes loading → fires preloadNext() again → starts N+2
```

This creates a **rolling prefetch chain**: by the time the user presses next, the next image is already decoded and sitting in the preview cache. Navigation is instant because it's just a cache lookup.

Sharpr prefetches ±1 but only triggers on selection, and only two slots. If decode of N+1 isn't complete when the user presses next, they still wait. The chain approach ensures decode always has a head start.

---

### Complete digiKam Preview Loading Pipeline

```
User clicks image
      ↓
Check LoadingCache (in-memory LRU)     ← ~5ms, INSTANT if hit
      ↓ (miss)
Try EXIF embedded preview               ← ~30–80ms (skip main decode entirely)
      ↓ (no embedded or too small)
JPEG: DCT-scale decode (scale_denom)    ← ~80–300ms depending on scale factor
PNG/WebP: full decode + downsample      ← ~200–800ms (no DCT trick available)
      ↓
Apply EXIF rotation (on small image)    ← ~5ms
Color management                        ← ~10ms
      ↓
Store in LoadingCache                   ← next visit = instant
      ↓
Display in viewer, trigger preload of N+1
```

For a typical camera JPEG in a 1280px viewer: **30–300ms total**, mostly spent on embedded preview extraction or DCT-scaled decode.

---

### Scope Note: JPEG First, RAW Secondary

Sharpr targets lossy image viewing (JPEG, WebP, PNG). The embedded preview and DCT scaling optimizations are the most impactful for this use case. RAW support is secondary — if added later, the half-size RAW decode (`halfSizeColorImage=true`, `sixteenBitsImage=false`) and LibRaw embedded preview extraction apply, but they are not needed for the core use case.

---

## 3. Key Differences & Bottlenecks in Sharpr

| Feature | Sharpr | digiKam | Impact |
| :--- | :--- | :--- | :--- |
| **Storage** | Filesystem (Thousands of files) | Database (BLOBs) | **High:** Filesive I/O is a major bottleneck for smooth scrolling. |
| **Format** | PNG (Lossless, heavy) | PGF/Compressed BLOBs | **Medium:** PNG is slow to decode and large on disk. |
| **Decoding** | Full image decode for every thumb | Extract embedded metadata | **CRITICAL:** Decoding a full image just for a 160px thumb is extremely wasteful. |
| **Memory** | Fixed 4-slot prefetch | Configurable RAM cache | **Medium:** 4 slots may be too few for high-resolution displays/fast navigation. |
| **Scaling** | Lanczos3 (Expensive) | Fast interpolation for previews | **Low:** Lanczos is high quality but adds to the CPU load. |

---

## 4. Recommendations for Sharpr

### Phase 1: Easy Wins (Decoding)
1. **Use Embedded Previews:** Use `rexiv2` or a specialized crate to extract embedded thumbnails from EXIF data. This would make initial folder scans near-instant.
2. **Fast Decoding for Previews:** For JPEGs, use `turbojpeg` (if possible) or a faster decoding path that doesn't require full-resolution bit-perfect accuracy for a 160px thumb.

### Phase 2: Architectural Shifts (Storage)
3. **Thumbnail Database:** Move from individual files to a single SQLite database. Use `rusqlite` with WAL mode. Store thumbnails as BLOBs.
4. **Switch Format:** Consider using high-quality JPEG or WebP (lossy) for thumbnails instead of PNG. The storage savings and decoding speed boost would be significant.

### Phase 3: UI & UX (Responsiveness)
5. **Incremental Loading:** Implement a "placeholder -> thumbnail -> full image" pipeline.
6. **Configurable Cache:** Allow the user to set a memory limit (e.g., 512MB) for pre-decoded images, rather than a hardcoded 4-entry limit.
7. **SSD Optimization:** Ensure the thumbnail database is stored on the fastest local drive (usually `~/.cache`), even if the images are on slow external storage.

---

## 5. Implementation Priority for Sharpr

Ranked by expected performance gain vs. implementation effort. **Preview loading is the most user-visible problem** and should be fixed before thumbnail optimizations.

### Preview Loading (Viewer — Fix First)

| # | Change | Effort | Expected Gain |
| :-- | :--- | :--- | :--- |
| 1 | **JPEG DCT-level scaled decode** via `turbojpeg` or `mozjpeg` (`scale_denom`) | Medium (swap decoder in `viewer.rs:904–917`) | **CRITICAL** — 4–5s → ~200–300ms for typical JPEG |
| 2 | **EXIF embedded preview extraction** via `rexiv2::get_preview_images()` before full decode | Low (rexiv2 already linked; add to viewer slow path) | **CRITICAL** — near-instant for camera JPEGs with embedded previews |
| 3 | **In-memory preview LRU cache** (decoded scaled previews, ~256 MB) | Medium (new cache in `library.rs`) | **HIGH** — revisiting images becomes instant; navigation feels instant |
| 4 | **Rolling prefetch chain** (on image N display, immediately start N+1) | Low (extend `window.rs:33–74`) | **HIGH** — sequential browsing perceived as instant after first load |

### Thumbnail Loading (Filmstrip — Fix Second)

| # | Change | Effort | Expected Gain |
| :-- | :--- | :--- | :--- |
| 5 | **EXIF embedded thumbnail** for filmstrip thumbs via `rexiv2` | Low | **HIGH** — most JPEG/RAW files have a 160px Exif thumbnail; eliminates full decode |
| 6 | **Separate visible/preload worker channels** | Medium (refactor `worker.rs`) | **HIGH** — prevents preload from blocking visible tiles |
| 7 | **Increase stored thumbnail size to 256px** | Low | **MEDIUM** — serves HiDPI and future larger strips from one cache entry |
| 8 | **Per-file dedup set** in pending queue | Low | **MEDIUM** — eliminates redundant queuing during fast scroll |
| 9 | **Switch disk cache format to JPEG/WebP** | Low | **LOW** — faster decode + smaller files vs PNG |
| 10 | **SQLite BLOB storage** with WAL mode | High | **LOW on SSD** — meaningful on HDD; defer unless targeting slow storage |

### Recommended first two PRs

- **PR A — Preview speed** (`viewer.rs`): Add `rexiv2` embedded preview fast-path + swap `image::ImageReader::decode()` for `turbojpeg` with `scale_denom`. Target: 4–5s → <300ms. Scope: `viewer.rs` only.
- **PR B — Navigation feel** (`window.rs`, `library.rs`): Add in-memory preview LRU cache + rolling prefetch chain. Target: perceived-instant sequential browsing. Scope: `window.rs` + `library.rs`.
