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
- [x] 40a/40b committed: matcher and swap with Undo (`ab39612`, `85d21d0`).
- [x] 40c committed: Compare replace mode, entry points, GTK workflow
  `replace_with_upgrade_full_flow` (`00f1145`).
- [x] 40d committed: Sharpr calls Wallslæd `Relink` (`e2c46cf`); Wallslæd
  branch `feature/relink` `ddb162c` (own repo, not merged). Tombstones were
  dropped: Wallslæd `main` already never drops favourites or bans on a scan
  (its DS-02 fix), and a test shows the rescan-first race is harmless.
- [x] 39a gate **passed** (run 2, probe 1.4, near-lossless 0.25): 40 camera
  images with ≥2900 px short side (Photo, Photo/Export, ARW) scaled to 4K;
  natives falsely flagged 5% (2/40, limit 10%); fakes flagged 96% (692/720,
  needed 90%); within one ladder step 95% (686/720). Weakest: 1440p
  Triangle + sharpened 58%; 1080p sources 100%. Run 1 (camera grids with
  too little headroom) flagged 19% of natives. Report
  `.tmp/detail-gate/report.md`. Under about 1120 px short side: "too small".
- [x] 39b/39c committed (`1fea477`). Sample: Wallpapers 1527 files, 0
  undecodable, 540 look upscaled, 238 not measurable; 2.4 s per image at
  full speed, about 4 hours for the 5910-file library at idle priority. All
  10 labelled candidates look upscaled. Sheets: `.tmp/detail-gate/
  contact-sheet.html`, `labelled-candidates.html`.

State: **verified candidate**, all committed on this branch; not merged,
pushed or deployed. Checks: fmt, strict clippy, machete clean; 354/358
unit tests (4 glycin AVIF/HEIC sandbox tests fail on `main` too); deny
fails only on RUSTSEC-2026-0459 (SQLite, pre-existing); GTK 44/45 then
the one failure passed alone. Final build re-measured the gate directly
(no probe arithmetic): identical, 5% / 96% / 95%. Off-screen stand-in for
the rapid-scroll check (`filmstrip_stays_responsive_while_measuring_detail`,
debug build): longest main-loop iteration 57 ms with the pass held, 70 ms
while it measured 4K images, giving way to thumbnails. Left for the user:
the same check by hand on the desktop, and confirming the 10 labelled
candidates in `.tmp/detail-gate/labelled-candidates.html` (all measured
"looks upscaled").

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
