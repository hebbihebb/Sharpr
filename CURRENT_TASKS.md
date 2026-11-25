# Current Tasks

## Recently accepted and merged

November 25 (review fixes): Real detail and Replace with upgrade fixes merged
to `main` (Sharpr `1e92746`..`e1aa973`, Wallslæd `702ba6d`), deployed natively
and installed; not pushed yet. Covered: Compare Replace focus/Escape/`[`/`]`
and `clear()`, `detail_index` scope cancellation on folder disable/root change,
open-bucket and sidebar refresh on Replace/Undo, `RealDetail` sort in visual
search (`RankedSnapshot`) and `insert_new_files`, `also_trash` ordering/toast
errors and `undo` rollback on `restore` failure, dangling-symlink target check,
`--upscaled` parsing, same-path Wallslæd Undo suppression, and Wallslæd
`Relink` updating `current.source`/`library`/stale `preloaded` and guarding
empty/no-op/partial-scan cases.

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
