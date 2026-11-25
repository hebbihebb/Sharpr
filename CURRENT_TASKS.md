# Current Tasks

## Active: Review fixes for Real detail (D48) and Replace with upgrade (D49)

Branch `fix/review-real-detail-and-replace` (from `main` `e95e45b`), paired
with Wallslæd branch `fix/relink-review` (from `main` `8651e09`).

Fixes:
- Compare Replace mode: keep keyboard handling inside `review_key` when the
  candidate thumbnail strip has focus (so `Escape` finishes the review and
  `[`/`]` never dismiss), and clear `review` / `replace_bar` in `clear()`.
- `library_scope_changed`: cancel and re-scope `detail_index` immediately on
  folder disable or root change, matching `visual_index`.
- `after_replace`: refresh sidebar bucket counts, reload an open bucket on
  Replace and Undo, and insert arrived folder files via `show_new_files`
  (including restored `also_trashed` copies).
- Real Detail sort: pass `detail_sides` into `RankedSnapshot::sorted`, and
  carry `detail_short_side` on `IndexedImage`/`ImageEntry` so `compare_entries`
  and `insert_new_files` follow the same sort rule.
- `upgrade.rs`: trash `also_trash` copies before `replace_identity` so a failed
  copy keeps its bucket rows and reports the error in the toast; roll back
  `placed` if `undo` fails to restore `original`; use `symlink_metadata` for
  dangling-symlink collision checks.
- Polish: `wallslaed::pairs_for` skips no-op Undo pairs on same-extension swaps;
  `--upscaled` parses as `Not(Upscaled)`; `Job::UndoReplace` locks `new_from`
  and `also_trashed`; drop unused `detail_index::Service::version`.
- Wallslæd (`fix/relink-review`): `Relink` updates `current.source` and
  `library`, drops stale `preloaded` entries, skips empty/no-op pairs without
  rewriting state, and deduplicates non-favorite partial-scan slots in `order`.

State: **verified candidate**, all committed on `fix/review-real-detail-and-replace`
(and Wallslæd `fix/relink-review`); not merged, pushed or deployed. Checks:
Sharpr `cargo fmt --check`, strict `cargo clippy --all-targets -- -D warnings`,
`cargo build`, `cargo machete` clean; 361/365 unit tests (+7 new; only the 4
pre-existing glycin AVIF/HEIC sandbox tests fail, as on `main`); 6/6 targeted
off-screen GTK tests pass (`replace_with_upgrade_full_flow`,
`replace_review_focus_keys_and_bucket_view_refresh`,
`real_detail_filter_sort_and_info`, and all `compare_*` tests). Wallslæd
`cargo build --workspace` and `cargo test --workspace` clean (49/49 pass, +2 new).

## Recently accepted and merged

November 25: Real detail (D48, item 39) and Replace with upgrade (D49, item 40),
merged to `main` (`e95e45b`), pushed to `origin/main`, and deployed natively;
Wallslæd `Relink` merged (`8651e09`), pushed, and installed. Gate: 5% native
false positives (2/40), 96% fakes flagged (692/720), 95% within one step
(686/720); desktop trial accepted (231 s, 359 images measured while browsing).

October 20: PNG defaults for crop and export (D47, item 38, `be29be9`).

October 19: Compare layout (`a4ff062`, items 33/D45/D46), close-images Info
count (`fed0396`, item 29), Show Similar (`cdd57bd`, item 28/D44), Compare
zoom floor and AVIF/HEIC Crop Copy (`d275c54`, items 35/34). Their source-format
encoders remain under Original (D47).
