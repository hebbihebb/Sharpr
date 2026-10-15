#!/usr/bin/env bash
# Exports the committed sharpr/ tree at HEAD into .flatpak-source, so a
# Flatpak build is pinned to one commit and carries no stale or uncommitted
# files. Refuses a dirty working tree.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
dest="${script_dir}/.flatpak-source"
repo_root="$(git -C "${script_dir}" rev-parse --show-toplevel)"

if [[ -n "$(git -C "${repo_root}" status --porcelain --untracked-files=no)" ]]; then
  echo "Commit or stash your changes first; the Flatpak builds HEAD only." >&2
  exit 1
fi

commit="$(git -C "${repo_root}" rev-parse HEAD)"
rm -rf "${dest}"
mkdir -p "${dest}"
git -C "${repo_root}" archive --format=tar "${commit}:sharpr" | tar -x -C "${dest}"
echo "${commit}" > "${dest}/COMMIT"
echo "Flatpak source: ${commit:0:7}"
