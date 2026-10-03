#!/usr/bin/env bash
set -euo pipefail

# Official upstream binary, pinned independently of the runner's apt packages.
destination="${RUNNER_TEMP:?}/arcforge-minisign"
mkdir -p "$destination"
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
  https://github.com/jedisct1/minisign/releases/download/0.12/minisign-0.12-linux.tar.gz \
  --output "$destination/minisign.tar.gz"
printf '%s  %s\n' \
  '9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73' \
  "$destination/minisign.tar.gz" | sha256sum --check
tar -xzf "$destination/minisign.tar.gz" -C "$destination"
chmod +x "$destination/minisign-linux/x86_64/minisign"
printf '%s\n' "$destination/minisign-linux/x86_64" >> "${GITHUB_PATH:?}"
