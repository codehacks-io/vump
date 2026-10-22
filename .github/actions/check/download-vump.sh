#!/usr/bin/env bash
# download-vump.sh — Download the vump binary for the current platform.
#
# Downloads to $RUNNER_TEMP (writable on both GitHub-hosted and self-hosted
# runners, cleaned up after the run, never touches system paths).
# Exports VUMP_BIN to $GITHUB_ENV so subsequent steps reference the exact
# binary path without relying on PATH.
#
# Requires env vars:
#   VUMP_VERSION_INPUT — "latest" or a specific tag like "v0.2.0"
#   RUNNER_TEMP        — set by GitHub Actions runtime (always defined)
#   GITHUB_ENV         — set by GitHub Actions runtime (always defined)

set -euo pipefail

OS=$(uname -s | tr '[:upper:]' '[:lower:]')

case "$(uname -m)" in
  x86_64)        ARCH="amd64" ;;
  aarch64|arm64) ARCH="arm64" ;;
  *) echo "::error::Unsupported architecture: $(uname -m)"; exit 1 ;;
esac

# Darwin releases only the universal binary (contains both arm64 + amd64).
if [[ "$OS" == "darwin" ]]; then
  BINARY="vump-darwin-universal"
else
  BINARY="vump-${OS}-${ARCH}"
fi

if [[ "$VUMP_VERSION_INPUT" == "latest" ]]; then
  TAG=$(curl -fsSL https://api.github.com/repos/codehacks-io/vump/releases/latest \
        | grep '"tag_name"' | head -1 | cut -d'"' -f4)
else
  TAG="$VUMP_VERSION_INPUT"
fi

DEST="${RUNNER_TEMP}/vump-${TAG}"
SUMS="${RUNNER_TEMP}/vump-${TAG}.SHA256SUMS"
BASE="https://github.com/codehacks-io/vump/releases/download/${TAG}"

echo "Downloading vump ${TAG} (${BINARY}) → ${DEST}"
curl -fsSLo "$DEST" "${BASE}/${BINARY}"

# This binary is about to be executed inside a job that can hold signing
# secrets, so what arrived is checked against what was published before it is
# allowed to run. A release with no published digests is refused rather than
# trusted: warning and continuing would not be a safeguard.
echo "Verifying against published checksums"
# Errors are silenced here (-S dropped) because a missing SHA256SUMS is an
# expected outcome with its own message below; curl's raw 404 would only bury
# it in the log.
if ! curl -fsLo "$SUMS" "${BASE}/SHA256SUMS" 2>/dev/null; then
  echo "::error::vump ${TAG} publishes no SHA256SUMS; refusing to run an unverified binary."
  echo "::error::Pin vump-version to a release that publishes checksums."
  exit 1
fi

EXPECTED=$(grep -E "[[:space:]][*]?${BINARY}\$" "$SUMS" | awk '{print $1}' | head -1)
if [[ -z "$EXPECTED" ]]; then
  echo "::error::SHA256SUMS for ${TAG} lists no digest for ${BINARY}; refusing to run it."
  exit 1
fi

# Prefer the coreutils tool, falling back to the BSD one on macOS runners.
if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL=$(sha256sum "$DEST" | awk '{print $1}')
else
  ACTUAL=$(shasum -a 256 "$DEST" | awk '{print $1}')
fi

if [[ "$ACTUAL" != "$EXPECTED" ]]; then
  echo "::error::${BINARY} does not match its published checksum."
  echo "::error::expected ${EXPECTED}"
  echo "::error::received ${ACTUAL}"
  rm -f "$DEST"
  exit 1
fi
echo "  ✓ checksum verified"

chmod +x "$DEST"

# Export the full path — next steps use $VUMP_BIN directly, never assume PATH.
echo "VUMP_BIN=${DEST}" >> "$GITHUB_ENV"
"$DEST" --version
