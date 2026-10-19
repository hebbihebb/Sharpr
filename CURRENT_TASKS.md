# Current Tasks

## Accepted: close-images count in Info (Work Order item 29)

Branch `feature/close-count`. While the Info card is open, the viewer counts
the pictures close to the image over the whole library, off the GTK thread,
once per image: what Show Similar's Close lists less its nearest 12
(`similar::counts_as_close`, sharing `in_close` with Show Similar), so its
versions elsewhere and pictures at or above `close_cutoff`. Versions in the
image's own folder aren't counted (Close doesn't list them). "N close
images" opens Show Similar at Close with Whole library ticked. Nothing
shows when there are none or the image isn't indexed yet; the vectors load
only when Info first opens; a cancelled or stale count never shows.
Tests: `close_count` unit test (versions elsewhere, standing out, nearest
below the cutoff, same-folder copy, cancel); Show Similar's GTK test opens
Info on the shot ("1 close image") and clicks it. `./check.sh` (323 tests)
and all 41 GTK tests pass off screen.
Fix (user, October 19): clicking the count ended the search half a second
later. The focused button vanished as Info collapsed, so GTK moved the
focus into the sidebar's folder list, which opened its folder (item 26's
trap). The button no longer takes the focus on click, and from the keyboard
the focus goes to the filmstrip; the GTK test focuses it, clicks, collapses
Info and checks the results stay.
Deployed as `fed0396`; accepted by the user October 19 and merged to `main`.

## Accepted: Show Similar (Work Order item 28, D44)

Branch `feature/show-similar`. First version (`4a9e574`) cut at fixed scores
and always searched the whole library; the user found it listed only copies
(October 19). Revised per D44 and the approved plan:
- Ranked closest first by stored vectors (`visual_search::similar::neighbours`).
  Second revision (user found Close empty): levels nest. Close = versions +
  12 nearest distinct pictures + any ≥ `close_cutoff` (mean + 3 sd of the
  image's similarity to the library); Similar = all; Exact = versions
  (`copies::version_of`: same file, or ≥ 0.95 with 32×32 versions within 20,
  which takes crops and edits; Find Copies' D14 rule unchanged).
- Scope follows the Whole library box; versions in the image's own folder are
  shown only under Exact; Show More pages Similar.
- Trial on a copy of the user's data: the "(Edited)" lava wallpaper's Exact
  finds its 4 versions across the library and Close 18 images (versions +
  12 nearest pictures and their copies); in Wallpapers Exact says to look
  further and Close lists 12 lava and landscape wallpapers. 2–40 ms.
- Tests: neighbours order; `version_of` (crop and edit in, other renders
  out); GTK test of folder scope, the three levels, the Whole library box,
  chip removal and unchanged files. Show More isn't covered by a test (needs
  more than 200 images).
- Deployed as `cdd57bd`; accepted by the user October 19 and merged to `main`.

## Accepted: Compare zoom floor and AVIF/HEIC Crop Copy (Work Order items 35, 34)

Branch `fix/compare-zoom-avif-crop`.
- Item 35: Compare's zoom never goes below Fit, as in the viewer and crop.
  `compare_view::zoomed` snaps to Fit when zooming out past it, and
  `placement` never draws an image smaller than its fitted size (100% on a
  small image, the other side of a linked zoom). Test covers both.
- Item 34: `oriented_source_dimensions` reads AVIF/HEIC sizes through glycin
  (`prepare::heif_dimensions`, upright), and Crop Copy writes AVIF/HEIC in
  their own format through glycin at quality 100, 8-bit (`prepare::encode_heif`),
  as D37 does for JPEG. Test: AVIF and HEIC crops keep the format, size and
  region; originals unchanged. A real 1920×1337 AVIF cropped in 0.25 s.

Deployed as `d275c54`; accepted by the user October 19 and merged to `main`.

## Accepted: AVIF and HEIC in Compare (Work Order item 32)

Branch `fix/compare-avif`. `export::decode_oriented` decodes AVIF and
HEIC/HEIF through glycin (`prepare::decode_with_glycin`), so Compare shows
them; WebP export and Copy Image of these formats work through the same
path. Compare's summary takes the decoded size when the header can't be
read. Test: glycin-written AVIF and HEIC decode at full size and the right
colour, with "1600 × 900" in the summary (fails without the fix). Item 33
(Compare layout redesign) and a Backlog note on AVIF/HEIC sizes added.
Deployed as `e24032b`; accepted by the user October 19 and merged to `main`.
