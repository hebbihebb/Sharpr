#!/usr/bin/env bash
set -e

cd "$(dirname "$0")"

echo "Compiling GSettings schemas..."
glib-compile-schemas data/

echo "Running tests..."
GSETTINGS_SCHEMA_DIR="$PWD/data" GIO_USE_VFS=local cargo test "$@"
