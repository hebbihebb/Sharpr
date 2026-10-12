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
cargo build --features visual-search-trial    # experimental visual-search build
```

Debug builds are unoptimized, so image decoding and filmstrip speed look much slower than in a release build. Judge performance only with `cargo build --release`.

## Tests

```bash
cargo nextest run                     # whole suite (cargo test works if nextest is unavailable)
cargo nextest run <name-filter>       # targeted tests
./check.sh                            # fmt, strict clippy, nextest, deny, machete
```

`check.sh` runs clippy with `--all-features`, so the experimental feature must compile too.

Most tests live in `#[cfg(test)]` modules next to the code. The main ones cover:
- `library_index`
- `tags/db`
- `export`
- `thumbnails/worker`
- `duplicates/phash`
- `model/library`
- `model/browse`

The GTK end-to-end test is `#[ignore]`d by default. It needs a graphical session and isolated directories, and it refuses to run against real data:

```bash
glib-compile-schemas data/
GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=data \
XDG_DATA_HOME="$PWD/../.tmp/gtk/data" XDG_CACHE_HOME="$PWD/../.tmp/gtk/cache" \
SHARPR_GTK_FIXTURE="$PWD/../.tmp/gtk/fixtures" \
cargo nextest run --ignored gtk_tests
```

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

`tools/install-native-launcher.py` installs:
- `~/.local/share/applications/sharpr-dev.desktop`, which runs `sharpr/target/debug/sharpr` from the main checkout with `GSETTINGS_SCHEMA_DIR=sharpr/data`
- the app icon

This has two consequences:
- **Any `cargo build` in the main checkout replaces what the user launches.** That includes uncommitted work. When you rebuild there, tell the user.
- **The user experiences debug-build performance.**

`ROADMAP.md` Phase 0 changes the launcher to an installed release build of an accepted commit. After that, this section describes the release path.

## Flatpak

From `sharpr/packaging/`:

```bash
bash sync-flatpak-source.sh                                   # rsync the working tree into .flatpak-source
flatpak-cargo-generator ../Cargo.lock -o cargo-sources.json   # whenever Cargo.lock changes
flatpak-builder --force-clean --user --install build-dir io.github.hebbihebb.Sharpr.yml
flatpak run io.github.hebbihebb.Sharpr
```

- **Runtime:** GNOME 50 plus the rust-stable SDK extension.
- **Memory:** the manifest caps cargo at `--jobs 4` to avoid running out of memory while libjxl compiles. Keep the cap.
- **No version pinning:** the build uses whatever is in the working tree, uncommitted changes included, so record the commit you built.
- **Stale target data:** `.flatpak-source` keeps an old `target/` from earlier syncs because the script does not delete excluded files (`ROADMAP.md` Phase 6).

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

To check filmstrip performance, compare logs from the same folder before and after a change, using release builds.

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

The method and gate are in `ROADMAP.md` Phase 3.
