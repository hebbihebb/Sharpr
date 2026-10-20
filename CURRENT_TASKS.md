# Current Tasks

## Accepted: PNG defaults for crop and export (D47, Work Order item 38)

Accepted by the user October 20, 2026; merge, native deployment and push
authorized. Implementation `be29be9`, fresh verification `7e26d73`, D47
plan `79e35db`. No implementation remains; the installed commit is recorded
in `~/.local/opt/sharpr/current/COMMIT`.
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
independent Python PNG decode of exact 16-bit crop pixels/alpha/ICC, and
all 43 off-screen GTK workflows (32 passed in the full run, then the last
11 passed after updating the stale WebP-icon expectation). One independent
review found ICC mismatch and gamma-loss cases;
both have fixes and regression tests. Screenshots and check log are under
`.tmp/png-output/`. Fresh user-requested retest of `be29be9`: all 31
file/settings regressions and 5 targeted GTK workflows passed. Independent
checks of actual GTK outputs confirmed format/size and identical PNG/WebP
crop pixels; the 16-bit RGBA/ICC crop is exact. JPEG comparison differed by
at most 2/255 between independent decoders, not an added PNG loss. Screenshots
checked; evidence is in `.tmp/png-output-retest/` and its adjacent test logs.
User acceptance covers this verified candidate.

PNG is a preparation policy, not a confirmed fix for the discarded SeedVR
banding examples. Work Order priority is unchanged; follow its first open
row for the next task. This user authorization supersedes the Work Order's
standard stop before merge/push/deploy.

## Recently accepted and merged

October 19: Compare layout (`a4ff062`, items 33/D45/D46), close-images Info
count (`fed0396`, item 29), Show Similar (`cdd57bd`, item 28/D44), Compare
zoom floor and AVIF/HEIC Crop Copy (`d275c54`, items 35/34). Their source-format
encoders remain under Original (D47).
