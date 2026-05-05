#!/usr/bin/env bash
# Quick project health check. Run from sharpr/.
set -euo pipefail

cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run
cargo deny check
cargo machete
