#!/usr/bin/env bash
# Sync hypr-kblayoutd-bin's PKGBUILD to a published release: rewrite pkgver,
# pull the per-arch sha256sums from the release's sha256sums.txt, and
# regenerate .SRCINFO. Needs curl and makepkg (no compiler).
#
# Usage: contrib/update-bin-pkgbuild.sh <version> [pkgbuild-dir]
#   e.g. contrib/update-bin-pkgbuild.sh 0.2.1 ~/Documents/aur/hypr-kblayoutd-bin

set -euo pipefail

readonly REPO="kengzzzz/hypr-kblayoutd"
readonly PKGNAME="hypr-kblayoutd"

ver="${1:-}"
dir="${2:-$HOME/Documents/aur/hypr-kblayoutd-bin}"

if [ -z "$ver" ]; then
  echo "usage: ${0##*/} <version> [pkgbuild-dir]" >&2
  exit 1
fi

ver="${ver#v}"
pkgbuild="$dir/PKGBUILD"

[ -f "$pkgbuild" ] || { echo "no PKGBUILD at $pkgbuild" >&2; exit 1; }

sums="$(curl -fsSL "https://github.com/$REPO/releases/download/v$ver/sha256sums.txt")"

get_sum() {
  local arch="$1" sum
  sum="$(printf '%s\n' "$sums" \
    | awk -v f="$PKGNAME-$ver-$arch.tar.gz" '$2 == f || $2 == "*" f { print $1 }')"
  if [ -z "$sum" ]; then
    echo "no checksum for $arch in release v$ver" >&2
    exit 1
  fi
  printf '%s' "$sum"
}

sum_x86_64="$(get_sum x86_64)"
sum_aarch64="$(get_sum aarch64)"

sed -i \
  -e "s/^pkgver=.*/pkgver=$ver/" \
  -e "s/^pkgrel=.*/pkgrel=1/" \
  -e "s/^sha256sums_x86_64=.*/sha256sums_x86_64=('$sum_x86_64')/" \
  -e "s/^sha256sums_aarch64=.*/sha256sums_aarch64=('$sum_aarch64')/" \
  "$pkgbuild"

( cd "$dir" && makepkg --printsrcinfo > .SRCINFO )

echo "updated $pkgbuild to $ver"
echo "  x86_64  $sum_x86_64"
echo "  aarch64 $sum_aarch64"
echo
echo "review, then: cd $dir && git commit -am 'upgpkg: $ver' && git push"
