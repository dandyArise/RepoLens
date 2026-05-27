#!/usr/bin/env sh
set -eu

REPO="${REPOLENS_REPO:-dandyArise/RepoLens}"
VERSION="${REPOLENS_VERSION:-latest}"
INSTALL_DIR="${REPOLENS_INSTALL_DIR:-$HOME/.local/bin}"
INIT="${REPOLENS_INIT:-0}"
INIT_TARGET="${REPOLENS_INIT_TARGET:-all}"
INIT_ROOT="${REPOLENS_INIT_ROOT:-$(pwd)}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

need curl
need tar

case "$(uname -s)" in
  Linux) os="linux" ;;
  Darwin) os="darwin" ;;
  *) echo "unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64) arch="x86_64" ;;
  arm64|aarch64) arch="arm64" ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

asset="repolens-${os}-${arch}.tar.gz"
api_base="https://api.github.com/repos/${REPO}/releases"

if [ "$VERSION" = "latest" ]; then
  release_json="$(curl -fsSL -H 'User-Agent: repolens-installer' "${api_base}/latest")"
else
  case "$VERSION" in
    v*) tag="$VERSION" ;;
    *) tag="v$VERSION" ;;
  esac
  release_json="$(curl -fsSL -H 'User-Agent: repolens-installer' "${api_base}/tags/${tag}")"
fi

asset_url="$(printf '%s' "$release_json" | sed -n 's/.*"browser_download_url": "\([^"]*'"$asset"'\)".*/\1/p' | head -n 1)"
checksum_url="$(printf '%s' "$release_json" | sed -n 's/.*"browser_download_url": "\([^"]*'"$asset"'.sha256\)".*/\1/p' | head -n 1)"

if [ -z "$asset_url" ] || [ -z "$checksum_url" ]; then
  echo "release asset not found: $asset" >&2
  exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL -o "$tmp/$asset" "$asset_url"
curl -fsSL -o "$tmp/$asset.sha256" "$checksum_url"

expected="$(awk '{print $1}' "$tmp/$asset.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/$asset" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')"
else
  echo "missing sha256sum or shasum" >&2
  exit 1
fi

if [ "$expected" != "$actual" ]; then
  echo "checksum mismatch" >&2
  exit 1
fi

tar -xzf "$tmp/$asset" -C "$tmp"
bin="$(find "$tmp" -type f -name repolens | head -n 1)"
if [ -z "$bin" ]; then
  echo "repolens binary not found in archive" >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR"
cp "$bin" "$INSTALL_DIR/repolens"
chmod +x "$INSTALL_DIR/repolens"

echo "Installed repolens to $INSTALL_DIR"
echo "Add this directory to PATH if needed:"
echo "  $INSTALL_DIR"

if [ "$INIT" = "1" ] || [ "$INIT" = "true" ]; then
  case "$INIT_TARGET" in
    all|codex|claude|cursor) ;;
    *) echo "invalid REPOLENS_INIT_TARGET: $INIT_TARGET" >&2; exit 1 ;;
  esac
  echo "Configuring MCP target '$INIT_TARGET' for repo root:"
  echo "  $INIT_ROOT"
  "$INSTALL_DIR/repolens" init --target "$INIT_TARGET" "$INIT_ROOT"
fi
