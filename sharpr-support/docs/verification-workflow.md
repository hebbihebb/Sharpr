# Verification Workflow

## Goal

Keep verification lightweight enough for day-to-day Rust GTK4 development while establishing a path to stronger checks later.

## Baseline Local Workflow

Run these commands from the repository root:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build
```

Use `cargo run` for manual GTK smoke testing after the build succeeds.

## Recommended Manual Smoke Checks

For now, keep manual checks short and repeatable:

1. Launch the app.
2. Open a folder with a small set of images.
3. Change selection in the filmstrip.
4. Verify the viewer updates and the app remains responsive.
5. Verify metadata overlay toggle and basic zoom interactions.

## When Tests Exist

Add this to the local loop:

```bash
cargo test
```

Recommended command order:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
```

## Suggested CI Shape

Start with a single lightweight verification job:

1. Install Rust stable.
2. Install GTK4, Libadwaita, and GExiv2 development packages.
3. Cache Cargo registry and target artifacts conservatively.
4. Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
```

Do not add GUI automation yet. The current codebase will get more value from stable compile, lint, and unit-test coverage first.

## Near-Term Test Strategy

Focus on pure or near-pure logic:

- formatting helpers in `metadata/exif.rs`
- extension and image-file filtering logic in `model/library.rs`
- config serialization and config path behavior in `config/settings.rs`

## Later-Phase Additions

- headless integration tests for extracted non-UI services
- packaging verification for Flatpak manifest consistency
- UI smoke automation only after core selection/loading flows stabilize
