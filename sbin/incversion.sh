#!/usr/bin/env bash
# SPDX-License-Identifier: 0BSD
# Copyright © 2026 Dr. Ing. Marcus "Karo" Pinnecke
# Provided "AS IS", without warranty of any kind.
# Increment VERSION (major|minor|patch; default minor), preserving each field's width.
set -Eeuo pipefail

usage() {
    cat <<'USAGE'
Usage: sbin/incversion.sh [major|minor|patch]

Increments VERSION. The default is minor. Lower fields reset to zero.
USAGE
}

[ $# -le 1 ] || { usage >&2; exit 2; }
part="${1:-minor}"
case "$part" in
    major|minor|patch) ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown version part '$part'" >&2; usage >&2; exit 2 ;;
esac

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
file="$root/VERSION"
tmp=""
cleanup() { [ -z "$tmp" ] || rm -f -- "$tmp"; }
trap cleanup EXIT
trap 'echo "ERROR: version increment interrupted." >&2; exit 130' HUP INT TERM

[ -f "$file" ] && [ -r "$file" ] && [ -w "$file" ] || { echo "ERROR: VERSION is missing or not writable: $file" >&2; exit 1; }
current="$(<"$file")"
current="${current//$'\r'/}"
[[ "$current" =~ ^([0-9]+)[.]([0-9]+)[.]([0-9]+)$ ]] || { echo "ERROR: invalid VERSION '$current'; expected MAJOR.MINOR.PATCH" >&2; exit 1; }
major_text="${BASH_REMATCH[1]}"
minor_text="${BASH_REMATCH[2]}"
patch_text="${BASH_REMATCH[3]}"
mw=${#major_text}; nw=${#minor_text}; pw=${#patch_text}
major=$((10#$major_text)); minor=$((10#$minor_text)); patch=$((10#$patch_text))
case "$part" in
    major) major=$((major + 1)); minor=0; patch=0 ;;
    minor) minor=$((minor + 1)); patch=0 ;;
    patch) patch=$((patch + 1)) ;;
esac
printf -v major_text '%0*d' "$mw" "$major"
printf -v minor_text '%0*d' "$nw" "$minor"
printf -v patch_text '%0*d' "$pw" "$patch"
new="$major_text.$minor_text.$patch_text"
tmp="$(mktemp "${file}.tmp.XXXXXX")"
printf '%s\n' "$new" >"$tmp"
chmod 0644 "$tmp"
mv -- "$tmp" "$file"
tmp=""
echo "VERSION: $current -> $new ($part)"
