#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
dest="${script_dir}/.flatpak-source"

mkdir -p "${dest}"

rsync -a --delete \
  --exclude '/target/' \
  --exclude '/packaging/.flatpak-source/' \
  --exclude '/packaging/.flatpak-builder/' \
  --exclude '/packaging/build-dir/' \
  --exclude '/.git/' \
  "${repo_root}/" "${dest}/"
