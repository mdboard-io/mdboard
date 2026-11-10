#!/usr/bin/env bash
# SPDX-License-Identifier: 0BSD
# Copyright © 2026 Dr. Ing. Marcus "Karo" Pinnecke
# Provided "AS IS", without warranty of any kind.
# Write TAG from the current Git branch (nightly, rc, release, lts, dev).
set -Eeuo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
branch="$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
[ -n "$branch" ] && [ "$branch" != "HEAD" ] || exit 0
if [ -n "${MDBOARD_TAG:-}" ]; then
    tag="$MDBOARD_TAG"
else
    case "$branch" in
        nightly) tag=nightly ;;
        candiate|candidate|rc) tag=rc ;;
        stable) tag= ;;
        stable-lts) tag=lts ;;
        *) tag=dev ;;
    esac
fi
tmp="$(mktemp "$root/TAG.tmp.XXXXXX")"
printf '%s\n' "$tag" >"$tmp"
chmod 0644 "$tmp"
mv -- "$tmp" "$root/TAG"
printf '%s\n' "$tag"
