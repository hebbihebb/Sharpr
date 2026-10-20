# Current Tasks

## Implemented: PNG defaults for crop and export (D47, Work Order item 38)

Branch `codex/png-output`, including the D47 plan (`79e35db`).
PNG is the initial crop/export default; explicit PNG / WebP / Original
choices are remembered separately. Legacy destination, quality and fit
settings remain. Both operations share the encoders and safe writers;
Original resolves per image, and WebP-only controls are conditional.
Menus, Ctrl+E, guide/keyboard crop, fullscreen, labels, export icon and help
are updated. File copy/move, Duplicate and clipboard stay separate.

PNG retains decoded 8/16-bit samples, alpha, compatible raster ICC profiles
and PNG color-definition chunks. Incompatible ICC color models and float
samples fail explicitly. JPEG XL/AVIF/HEIC profile metadata is not retained;
AVIF/HEIC decode at 8-bit. These limits are shown beside the format choice.
The direct `png` dependency reuses the already bundled crate; the regenerated
Flatpak source list is unchanged. No data migration or new crate was added.

Verified: `./check.sh` (330 tests, strict clippy, deny, machete),
independent Python PNG decode of exact 16-bit crop pixels/alpha/ICC, and the
all 43 off-screen GTK workflows (32 passed in the full run, then the last
11 passed after updating the stale WebP-icon expectation). One independent
review found ICC mismatch and gamma-loss cases;
both have fixes and regression tests. Screenshots and check log are under
`.tmp/png-output/`. Nothing deployed or accepted yet.

PNG is a preparation policy, not a confirmed fix for the discarded SeedVR
banding examples. Work Order priority is unchanged. The verified candidate is committed on
its branch for user acceptance; no merge/push/deploy,
as specified by the Work Order's standard effort.

## Recently accepted and merged

- Compare layout (item 33, D45/D46): deployed `a4ff062`, accepted October 19;
  `./check.sh` (323 tests) and 42 off-screen GTK tests passed.
- Close-images count in Info (item 29): deployed `fed0396`, accepted October 19,
  including the focus fix that keeps Show Similar results open.
- Show Similar (item 28, D44): deployed `cdd57bd`, accepted October 19;
  ranked stored-vector search with nested Exact / Close / Similar levels.
- Compare zoom floor and AVIF/HEIC Crop Copy (items 35/34): deployed `d275c54`,
  accepted October 19. Source-format encoders remain under Original (D47).
