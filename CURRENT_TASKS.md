# Current Tasks

## Active: Real detail (D48, item 39) and Replace with upgrade (D49, item 40)

Branch `feature/real-detail-and-replace` (one branch for both, the user's
choice, November 25, 2026). Goal mode: finish a verified candidate, then stop;
no merge, push or deploy. The full plan, including the online review's seven
changes, is summarised here; decisions are D48/D49 in `product.md`.

Priority order (stop only at a committed, verified boundary):
1. Docs (this commit), then 39a: `sharpr/src/detail.rs` estimator plus
   `tools/detail-gate.py` gate on camera originals and synthetic fakes;
   labelled fake-4K candidates found by non-estimator evidence (lower-res
   version elsewhere, upscaler names, Upscaled/Resized folders) for the user
   to confirm.
2. 40a matcher (`upgrade.rs`), 40b `Job::Replace` (preflight; move new
   first, trash original last; temp-name dance for the same path; buckets
   rewritten with an undo record), 40c Compare replace mode and entry points.
3. 39b cache table `image_detail` plus background `detail` thread; 39c Info,
   `upscaled` chip, sort, help.
4. 40d Wallslæd `Relink` (own branch there, with 10-minute tombstones);
   Sharpr calls it with NO_AUTO_START.

Progress:
- [x] Branch and docs (D48, D49, rows 39/40, briefs, Backlog).

Choices for the user at acceptance: `upscaled` word and "Looks upscaled"
chip, Info wording, original's stem with the new extension, R/S/B keys,
cache keyed by fingerprint, pairs across drives skipped, new file keeps its
own modified time, Wallslæd never auto-started.

## Recently accepted and merged

October 20: PNG defaults for crop and export (D47, item 38, `be29be9`).

October 19: Compare layout (`a4ff062`, items 33/D45/D46), close-images Info
count (`fed0396`, item 29), Show Similar (`cdd57bd`, item 28/D44), Compare
zoom floor and AVIF/HEIC Crop Copy (`d275c54`, items 35/34). Their source-format
encoders remain under Original (D47).
