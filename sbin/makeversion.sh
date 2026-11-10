#!/usr/bin/env bash
# SPDX-License-Identifier: 0BSD
# Copyright © 2026 Dr. Ing. Marcus "Karo" Pinnecke
# Provided "AS IS", without warranty of any kind.
# Generate mdboard build metadata (BUILD, BUILD_DATE) and print the full version string.
set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
root="$(cd -- "$script_dir/.." && pwd -P)"
version_file="$root/VERSION"
build_file="$root/BUILD"
date_file="$root/BUILD_DATE"
tag_file="$root/TAG"

build_tmp=""
date_tmp=""
cleanup() {
    [ -z "$build_tmp" ] || rm -f -- "$build_tmp"
    [ -z "$date_tmp" ] || rm -f -- "$date_tmp"
}
fail() { echo "ERROR: $*" >&2; exit 1; }
trap cleanup EXIT
trap 'echo "ERROR: version generation interrupted." >&2; exit 130' HUP INT TERM

"$root/sbin/tag.sh" >/dev/null || true

for f in "$version_file" "$build_file" "$tag_file"; do
    [ -f "$f" ] || fail "required file not found: $f"
    [ -r "$f" ] || fail "required file is not readable: $f"
done

version="$(<"$version_file")"
previous_build="$(<"$build_file")"
tag="$(<"$tag_file")"
version="${version//$'\r'/}"
previous_build="${previous_build//$'\r'/}"
tag="${tag//$'\r'/}"
normalized_build="${previous_build#-}"

[ -n "$version" ] || fail "VERSION must not be empty"
[ -n "$normalized_build" ] || fail "BUILD must not be empty"
[[ "$version" =~ ^[0-9]+([.][0-9]+)*$ ]] || fail "invalid VERSION '$version'; expected a numeric dotted version such as 0.3.1"
[[ "$normalized_build" =~ ^([0-9]{8})-([0-9]+)$ ]] || fail "invalid BUILD '$previous_build'; expected YYYYMMDD-N"
previous_date="${BASH_REMATCH[1]}"
previous_counter="${BASH_REMATCH[2]}"
[[ "$tag" =~ ^[A-Za-z0-9._-]*$ ]] || fail "invalid TAG '$tag'"

today="$(date -u '+%Y%m%d')"
if [ "$previous_date" = "$today" ]; then
    counter=$((10#$previous_counter + 1))
else
    counter=0
fi
build="$today-$counter"

build_tmp="$(mktemp "${build_file}.tmp.XXXXXX")"
printf '%s\n' "$build" >"$build_tmp"
chmod 0644 "$build_tmp"
mv -- "$build_tmp" "$build_file"
build_tmp=""

build_date="$(date -u '+%Y-%m-%d %H:%M:%S UTC')"
date_tmp="$(mktemp "${date_file}.tmp.XXXXXX")"
printf '%s\n' "$build_date" >"$date_tmp"
chmod 0644 "$date_tmp"
mv -- "$date_tmp" "$date_file"
date_tmp=""

version_string="$version-$build"
[ -z "$tag" ] || version_string+="#$tag"
commit="$(git -C "$root" rev-parse HEAD 2>/dev/null || echo unknown)"

echo "BUILD: $previous_build -> $build"
echo "VERSION_STRING: $version_string"
echo "COMMIT_SHA: $commit"
echo "BUILD_DATE: $build_date"
