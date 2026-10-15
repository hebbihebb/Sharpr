#!/usr/bin/env bash
# Builds and installs the Flatpak with bounded memory. Flatpak moves its
# build sandbox out of any systemd memory cap, so this limits parallel jobs
# and stops the build if the system runs low on memory, instead of letting
# it take the desktop down. Usage: bash build-flatpak.sh [extra flatpak-builder args]
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

jobs="${SHARPR_FLATPAK_JOBS:-4}"
floor_mb="${SHARPR_FLATPAK_FLOOR_MB:-2048}"

available_mb() { awk '/^MemAvailable:/ { print int($2 / 1024) }' /proc/meminfo; }
if (( $(available_mb) < floor_mb * 3 )); then
  echo "Only $(available_mb) MB available; close something before building." >&2
  exit 1
fi

flatpak-builder --force-clean --user --install --repo=repo --jobs="${jobs}" \
  build-dir io.github.hebbihebb.Sharpr.yml "$@" &
builder=$!
while kill -0 "${builder}" 2>/dev/null; do
  if (( $(available_mb) < floor_mb )); then
    echo "Memory below ${floor_mb} MB; stopping the build." >&2
    pkill -TERM -P "${builder}" || true
    kill -TERM "${builder}" || true
    # The compilers run in Flatpak's own scope for this app id.
    systemctl --user kill --signal=KILL 'app-flatpak-io.github.hebbihebb.Sharpr-*.scope' || true
    wait "${builder}" || true
    exit 1
  fi
  sleep 1
done
wait "${builder}"
