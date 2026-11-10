#!/usr/bin/env bash
# Install the mdBoard CLI into ~/.mdboard/bin.
# Usage: curl -fsSL https://mdboard.io/install.sh | sh
set -euo pipefail

REPO="mdboard-io/mdboard"
INSTALL_DIR="${MDBOARD_INSTALL:-$HOME/.mdboard}"
BIN_DIR="$INSTALL_DIR/bin"
EXE="$BIN_DIR/mdboard"
VERSION="${MDBOARD_VERSION:-}"
FROM_SOURCE="${MDBOARD_FROM_SOURCE:-0}"
CHANNEL="${MDB_CHANNEL:-stable}"

color_off=""
green=""
dim=""
if [ -t 1 ]; then
  color_off=$'\033[0m'
  green=$'\033[0;32m'
  dim=$'\033[0;2m'
fi

info() { printf '%s%s%s\n' "$dim" "$*" "$color_off"; }
success() { printf '%s%s%s\n' "$green" "$*" "$color_off"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar >/dev/null 2>&1 || die "tar is required"
case "$CHANNEL" in
  stable|nightly) ;;
  *) die "Unknown channel: $CHANNEL (use stable or nightly)" ;;
esac

case "$(uname -s)-$(uname -m)" in
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  Linux-x86_64) target="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64|Linux-arm64) target="aarch64-unknown-linux-gnu" ;;
  *)
    if [ "${OS:-}" = "Windows_NT" ]; then
      die "Use install.ps1 on Windows"
    fi
    die "Unsupported platform: $(uname -s)-$(uname -m)"
    ;;
esac

resolve_tag() {
  if [ -n "$VERSION" ]; then
    case "$VERSION" in v*) printf '%s' "$VERSION" ;; *) printf 'v%s' "$VERSION" ;; esac
    return
  fi
  curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null |
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1
}

install_from_release() {
  tag="$1"
  asset="mdboard-${tag}-${target}.tar.gz"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  info "Downloading $asset"
  curl -fsSL --retry 3 -o "$tmp/$asset" "https://github.com/$REPO/releases/download/$tag/$asset"
  if curl -fsSL --retry 3 -o "$tmp/SHA256SUMS" "https://github.com/$REPO/releases/download/$tag/SHA256SUMS"; then
    line="$(grep -F "  $asset" "$tmp/SHA256SUMS" || true)"
    [ -n "$line" ] || die "No checksum found for $asset"
    if command -v sha256sum >/dev/null 2>&1; then
      (cd "$tmp" && printf '%s\n' "$line" | sha256sum -c -) || die "Checksum failed for $asset"
    elif command -v shasum >/dev/null 2>&1; then
      (cd "$tmp" && printf '%s\n' "$line" | shasum -a 256 -c -) || die "Checksum failed for $asset"
    else
      die "No SHA-256 tool available"
    fi
  else
    die "Could not download SHA256SUMS for $tag"
  fi
  tar -xzf "$tmp/$asset" -C "$tmp"
  src="$tmp/mdboard-${tag}-${target}/mdboard"
  [ -f "$src" ] || die "Archive did not contain the mdboard binary"
  install -m 755 "$src" "$EXE"
}

install_from_source() {
  command -v cargo >/dev/null 2>&1 || die "No published release found and cargo is not installed"
  info "No usable release found; building the $CHANNEL branch from source"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  curl -fsSL --retry 3 -o "$tmp/mdboard.tar.gz" "https://github.com/$REPO/archive/refs/heads/$CHANNEL.tar.gz"
  tar -xzf "$tmp/mdboard.tar.gz" -C "$tmp"
  src_root="$tmp/mdboard-$CHANNEL"
  [ -d "$src_root" ] || die "Could not unpack the $CHANNEL source archive"
  (cd "$src_root" && cargo build --locked --release -p mdboard-cli)
  install -m 755 "$src_root/target/release/mdboard" "$EXE"
}

mkdir -p "$BIN_DIR"

tag=""
if [ "$FROM_SOURCE" != "1" ] && [ "$CHANNEL" = "stable" ]; then
  tag="$(resolve_tag || true)"
fi
if [ -n "$tag" ]; then
  install_from_release "$tag"
else
  install_from_source
fi

success "mdboard installed to $EXE"

if command -v mdboard >/dev/null 2>&1; then
  printf '\nRun %smdboard --help%s to get started.\n' "$green" "$color_off"
  exit 0
fi

quoted_install="$INSTALL_DIR"
case "$quoted_install" in
  "$HOME"/*) quoted_install="\$HOME/${quoted_install#"$HOME"/}" ;;
esac
path_line="export PATH=\"$quoted_install/bin:\$PATH\""
marker="# mdboard"

add_to_file() {
  file="$1"
  [ -w "$file" ] || return 1
  if ! grep -Fq "$marker" "$file" 2>/dev/null; then
    {
      printf '\n%s\n' "$marker"
      printf '%s\n' "$path_line"
    } >>"$file"
  fi
}

added=""
case "$(basename "${SHELL:-}")" in
  fish)
    fish_config="$HOME/.config/fish/config.fish"
    mkdir -p "$(dirname "$fish_config")"
    if [ -w "$fish_config" ] || [ ! -e "$fish_config" ]; then
      if ! grep -Fq "$marker" "$fish_config" 2>/dev/null; then
        {
          printf '\n%s\n' "$marker"
          printf 'set --export PATH %s/bin $PATH\n' "$quoted_install"
        } >>"$fish_config"
      fi
      added="$fish_config"
    fi
    ;;
  zsh)
    if add_to_file "$HOME/.zshrc"; then added="$HOME/.zshrc"; fi
    ;;
  bash)
    for file in "$HOME/.bash_profile" "$HOME/.bashrc"; do
      if add_to_file "$file"; then added="$file"; break; fi
    done
    ;;
esac

echo
if [ -n "$added" ]; then
  info "Added $BIN_DIR to PATH in $added"
  printf 'Restart your shell or run: %s\n' "source \"$added\""
else
  printf 'Add this to your shell configuration:\n\n  %s\n\n' "$path_line"
fi
printf 'Then run %smdboard --help%s.\n' "$green" "$color_off"
