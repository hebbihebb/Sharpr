# Current Tasks

## Planned: PNG defaults for crop and export (D47, Work Order item 38)

Branch `codex/png-output-plan`, from a clean `main` at `35f56fe`.
The user approved PNG as the default for Crop Copy and single/batch export,
with explicit WebP and Original format choices in both (October 20).
The roadmap brief covers filmstrip/viewer menus, Ctrl+E, guide-based crop,
Ctrl+Shift+G, mouse/keyboard save and cancel, mixed batches, legacy settings,
format/depth/profile handling, naming, Edits buckets and all help surfaces.
Duplicate, file copy/move, clipboard and internal image preparation keep
separate semantics. Implementation has not started; the running app still
exports WebP and crops to the source format (D37).

Research context: an 8-bit PNG/lossless-WebP comparison had identical pixels;
three controlled SeedVR tests did not conclusively reproduce the discarded
problem outputs. PNG is the approved preparation policy, not a confirmed
fix for SeedVR banding. PNG supports 16-bit samples, but the inspected
ComfyUI workflow loads and saves color images at 8-bit.

The decision and coverage are recorded in the roadmap and product log;
agent instructions distinguish the planned policy from current behavior.
Verified: `git diff --check`, decision/task IDs and referenced paths, and
`cargo build --offline --locked`. Documentation only; nothing deployed.
Work Order priority is unchanged; item 38 follows item 37. Next for this
feature: implement item 38 on its own branch, then run its acceptance checks.

## Recently accepted and merged

- Compare layout (item 33, D45/D46): image-corner captions, mirrored Keep
  controls, centered candidate strip with L/R halves, per-side errors.
  Deployed `a4ff062`, accepted October 19. `./check.sh` (323 tests) and all
  42 off-screen GTK tests passed.
- Close-images count in Info (item 29): counts standout neighbors and versions
  elsewhere; opens Show Similar at Close over the whole library. Includes
  the focus fix that keeps results open. Deployed `fed0396`, accepted
  October 19. `./check.sh` (323) and all 41 GTK tests passed.
- Show Similar (item 28, D44): ranked stored-vector search with nested
  Exact / Close / Similar and explicit library scope. Deployed `cdd57bd`,
  accepted October 19. Verification and behavior are recorded in the roadmap.
- Compare zoom floor and AVIF/HEIC Crop Copy (items 35/34): Fit is the zoom
  floor; glycin reads sizes and writes source-format crops at quality 100,
  8-bit. Deployed `d275c54`, accepted October 19. Format/region/original-byte
  regressions passed. D47 retains these encoders under Original.
