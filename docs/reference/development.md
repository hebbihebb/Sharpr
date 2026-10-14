# Sharpr Development Reference

Commands for building, running, testing, and packaging Sharpr. Run them from `sharpr/` unless a block says otherwise.

## Setup

- **Fedora packages:** `gtk4-devel`, `libadwaita-devel`, `gexiv2-devel`, `pkg-config`, `gcc`.
- **Rust:** stable, minimum version 1.94 (pinned in `rust-toolchain.toml`).
- **Cargo tools:** `cargo-nextest`, `cargo-deny`, `cargo-machete`.
- **Flatpak builds:** `flatpak-builder` and `flatpak-cargo-generator` (in `~/.local/bin`).

## Build And Run

```bash
cargo build                                   # always run this before handing work back
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR=data cargo run           # native run; the variable is needed only for run
```

Debug builds are unoptimized, so image decoding and filmstrip speed look much slower than in a release build. Judge performance only with `cargo build --release`.

## Tests

```bash
cargo nextest run                     # whole suite (cargo test works if nextest is unavailable)
cargo nextest run <name-filter>       # targeted tests
./check.sh                            # fmt, strict clippy, nextest, deny, machete
```

`check.sh` runs clippy with `--all-features` for the features present in the current checkout.

Visual-search tests that need the real model are ignored by default. Point them at the files with `SHARPR_TEST_MODEL_DIR` (the folder holding both GGUF files) and `SHARPR_LLAMA_SERVER`:
- `visual_search_ranks_by_content` (GTK): indexes four synthetic images in an isolated library and checks the ranking. Without the variables it checks only the file-name fallback.
- `service_indexes_a_library_with_the_installed_model`: run with `XDG_DATA_HOME` pointing at a folder that has the model installed under `sharpr-models/embeddinggemma-2/`.
- `app_pipeline_benchmark_report`: runs the app's own pipeline over the Phase 3 sample (`SHARPR_BENCH_MANIFEST`, `SHARPR_BENCH_OUTPUT`) and writes a report for `tools/benchmark-parity.py` and `tools/benchmark-score.py`. It reads private images; use it for development only.

Most tests live in `#[cfg(test)]` modules next to the code. The main ones cover:
- `library_index`
- `tags/db`
- `export`
- `thumbnails/worker`
- `duplicates/phash`
- `model/library`
- `model/browse`

The GTK end-to-end test is `#[ignore]`d by default. It needs a graphical session and isolated directories, and it refuses to run against real data. `sharpr/.config/nextest.toml` runs GTK tests one at a time. In test builds the fingerprint service skips its low priority and its 30 s spacing, so its passes finish predictably:

```bash
glib-compile-schemas data/
GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=data \
XDG_DATA_HOME="$PWD/../.tmp/gtk/data" XDG_CONFIG_HOME="$PWD/../.tmp/gtk/config" \
XDG_CACHE_HOME="$PWD/../.tmp/gtk/cache" \
SHARPR_GTK_FIXTURE="$PWD/../.tmp/gtk/fixtures" \
cargo nextest run --run-ignored ignored-only -E 'test(gtk_tests)'
```

## Checkpoint Without Replacing The Launcher

When deployment is not authorized, build with a separate target directory:

```bash
CARGO_TARGET_DIR="$PWD/../.tmp/phase0/target" cargo build --offline --locked
```

Run checks with `GSETTINGS_BACKEND=memory`, an isolated compiled schema, and isolated `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, and `XDG_CACHE_HOME`. The Phase 0 verification used these variables for unit tests as well as GTK. Do not run `cargo run` against the live library or assume an idle-data snapshot while another Sharpr process is running.

## Protecting User Data

The user's real data lives in:
- `~/.local/share/sharpr/`: `library-index.sqlite` and `tags.sqlite3`
- the GSettings schema `io.github.hebbihebb.Sharpr`

Never point tests, scripts, or experiments at these. Before the first run of a build that changes persistence code, close Sharpr and back up:

```bash
stamp=$(date +%Y%m%d-%H%M%S)
cp -a ~/.local/share/sharpr ~/.local/share/sharpr.backup-$stamp
dconf dump /io/github/hebbihebb/Sharpr/ > ~/.local/share/sharpr.backup-$stamp/gsettings.ini
```

To inspect the live databases, open them read-only. Python example: `sqlite3.connect('file:...?mode=ro', uri=True)`.

## Native Launcher (What The User Runs)

The user launches an installed release build, not the checkout. To deploy, with the user's approval only:

```bash
python3 tools/install-native-launcher.py     # from the repository root
```

The script:
1. Refuses to run on an uncommitted tree.
2. Runs `cargo build --release --locked`.
3. Copies the binary and a freshly compiled GSettings schema to `~/.local/opt/sharpr/<commit>/`, together with a `COMMIT` file.
4. Switches the link `~/.local/opt/sharpr/current` to the new install.
5. Rewrites `~/.local/share/applications/sharpr-dev.desktop` to run `current/sharpr` with `GSETTINGS_SCHEMA_DIR=current/schemas`.

Earlier installs are kept. To roll back, repoint the link and the launcher follows it:

```bash
ln -sfn ~/.local/opt/sharpr/<older-commit> ~/.local/opt/sharpr/current
```

To see what is deployed, run `cat ~/.local/opt/sharpr/current/COMMIT`. Development builds in `sharpr/target/` never affect the launcher.

## Visual Search Model

Sharpr never downloads the model (D23). Install it once, with the user's approval when it goes into their real home folder:

```bash
python3 tools/install-visual-search-model.py --models DIR --runtime DIR [--gpu-runtime DIR]
```

- `--models` holds `embeddinggemma-2-Q8_0.gguf` and `mmproj-embeddinggemma-2-Q8_0.gguf`. The Phase 3 copies are in `.tmp/native-benchmark/models/`.
- `--runtime` holds a `llama-server` build with its `lib*.so*` files: `.tmp/native-benchmark/runtime/llama-b11457/` (the prebuilt Ubuntu x64 release).
- `--gpu-runtime` holds the Vulkan build, `.tmp/native-benchmark/runtime/vulkan/llama-b11457/` (official release, checksum verified). It goes to `runtime-gpu/` and indexing then runs on the graphics card. For tests, `SHARPR_LLAMA_SERVER_GPU` points at one directly.
- Files go to `~/.local/share/sharpr-models/embeddinggemma-2/` (and `runtime/`). Each is verified against its source; identical files are skipped.
- Preferences → Library → Visual Search shows what Sharpr found and the indexing progress. `~/.cache/sharpr/logs/llama-server-*.log` holds the server's own messages.
- Deleting `~/.local/share/sharpr/visual-search.sqlite` only forces a reindex.

## Flatpak

From `sharpr/packaging/`:

```bash
bash sync-flatpak-source.sh                                   # rsync the working tree into .flatpak-source
flatpak-cargo-generator ../Cargo.lock -o cargo-sources.json   # whenever Cargo.lock changes
flatpak-builder --force-clean --user --install build-dir io.github.hebbihebb.Sharpr.yml
flatpak run io.github.hebbihebb.Sharpr
```

- **Runtime:** GNOME 50 plus the rust-stable SDK extension.
- **Visual search:** the `llama-cpp` module builds `llama-server` from the pinned source tarball into `/app/libexec/sharpr-llama/`, without network features. The model is not bundled: the Flatpak reads `~/.var/app/io.github.hebbihebb.Sharpr/data/sharpr-models/embeddinggemma-2/`, or else the native install's folder through `home:ro`.
- **Memory:** the manifest caps cargo at `--jobs 4` to avoid running out of memory while libjxl compiles. Keep the cap.
- **No version pinning:** the build uses whatever is in the working tree, uncommitted changes included, so record the commit you built.
- **Stale target data:** `.flatpak-source` keeps an old `target/` from earlier syncs because the script does not delete excluded files (`ROADMAP.md` Phase 10).

## Structured Logging

`src/bench.rs` writes JSONL events to `~/.cache/sharpr/logs/run-<timestamp>-<pid>.jsonl`. Logging is on by default.

| Variable | Effect |
| --- | --- |
| `SHARPR_BENCH=0` | Disable logging |
| `SHARPR_BENCH_LOG=<path>` | Choose the output file |
| `SHARPR_BENCH_LOG_LIMIT=<n>` | Number of log files to keep |

Use `bench_event!`, `bench_warn!`, and `bench_error!`, never `println!`.

These events are useful for checking responsiveness:
- `folder.open.request` and `folder.open.ready`
- `thumbnail.apply` (includes `worker_ms`)
- `filmstrip.thumbnail_schedule`

`mainloop.stall` is logged whenever the GTK main loop was blocked for 50 ms or more. A 100 ms probe timer ticked late; `late_ms` is the delay. `mainloop.probe` marks that the probe was running.

To summarize a log, run `python3 tools/bench-summary.py [LOG.jsonl]`; without an argument it reads the newest log. The summary covers:
- startup time
- per-folder open time, flagging scans that waited in the queue
- stalls during startup and while browsing, reported separately
- thumbnail time
- memory and CPU

To check filmstrip performance, do the same steps in the same folders before and after a change, using release builds, and compare the two summaries. The Phase 0 baseline is in `CURRENT_TASKS.md`.

## Rust Conventions

- Prefer simple, idiomatic code: exhaustive `match`, and enums that make invalid states unrepresentable. Add traits only at real boundaries.
- Errors: use `Result<T, String>` for simple operations and `Box<dyn Error>` for export and serialization. Introduce `thiserror` or `anyhow` only with a reason.
- No `unwrap()` or `expect()` in production paths unless the invariant is obvious. When you use `expect()`, its message states the invariant.
- Threads: `std::thread::spawn` or `rayon::spawn`, with `async_channel`. No Tokio. Check for pool oversubscription before adding parallelism.
- Avoid unnecessary `clone()` or `collect()`, but prefer clarity in GTK and GObject code.

## Semantic-Search Benchmark Tools

`tools/benchmark-image-search.py` (reference models) and `tools/benchmark-native-image-search.py` (llama.cpp) are development tools, not app dependencies. Keep these under `.tmp/`, never in Git:
- the Python environment (`.tmp/search-benchmark`)
- model files (`.tmp/search-models`, `.tmp/native-benchmark`)
- the private manifest and reports (`.tmp/*.json`)

Phase 3 judging and analysis, run with the benchmark environment `.tmp/search-benchmark/bin/python`:

```bash
# blind judging sheet: pools each query's top 20 across reports, then shuffles them
tools/benchmark-judge.py --manifest .tmp/search-manifest.json \
    --report .tmp/gemma-native.json --report .tmp/siglip-reference.json --output .tmp/search-judge.html
# after saving search-judgments.json from the sheet (queries missing from the
# manifest, the report or the judgments fail; every category needs queries):
tools/benchmark-score.py --manifest .tmp/search-manifest.json --judgments ~/Downloads/search-judgments.json \
    --report .tmp/gemma-native.json --report .tmp/siglip-reference.json --detail
# native versus reference agreement of the same model
tools/benchmark-parity.py --manifest .tmp/search-manifest.json \
    --reference .tmp/gemma-reference.json --native .tmp/gemma-native.json
```

Both benchmark scripts record provenance in every new report: model and runtime hashes, versions, threads, preprocessing, and the query prompt.

The method and gate are in `ROADMAP.md` Phase 3.
