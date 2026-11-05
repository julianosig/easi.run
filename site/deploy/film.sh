#!/bin/sh
# Publish the films: copy each language's (scripts/film.sh: <dir>/<locale>/<locale>.webm, .mp4, .jpg and .avif) into
# the releases' pool under a name carrying a hash of its bytes, write src/lib/films.json, which names them for the
# landing, and point each README at its AVIF. The films are rendered, not kept in the repository: commit those, then
# publish with release.sh, which
# refuses a films.json naming a file the pool lacks. Every film ever published stays in the pool.
#   site/deploy/film.sh <dir>
set -eu

releases="${EASI_RUN_RELEASES:-$HOME/.local/share/easi-run/releases}"
site="$(cd "$(dirname "$0")/.." && pwd)"
from="${1:?usage: film.sh <dir>}"
mkdir -p "$releases/pool/film"

# Every language the site speaks, English first: each must have its film.
locales="$(sed -n "s/^  \(['a-zA-Z-]*\): '.*',$/\1/p" "$site/../i18n/locales.ts" | tr -d "'")"
manifest="$site/src/lib/films.json"
{
  printf '{\n'
  first=1
  for locale in $locales; do
    [ "$first" = 1 ] || printf ',\n'
    first=0
    printf '  "%s": {' "$locale"
    sep=''
    for kind in webm mp4 jpg avif; do
      file="$from/$locale/$locale.$kind"
      [ -s "$file" ] || { echo "no $file" >&2; exit 1; }
      name="$locale-$(sha256sum "$file" | cut -c1-16).$kind"
      target="$releases/pool/film/$name"
      # A pool file appears only whole: copied under a temporary name, then renamed into place.
      if [ ! -e "$target" ]; then
        cp "$file" "$target.tmp.$$"
        mv "$target.tmp.$$" "$target"
      fi
      printf '%s "%s": "/film/%s"' "$sep" "$kind" "$name"
      if [ "$kind" = avif ]; then
        readme="$site/../docs/readme/$locale.md"
        [ "$locale" != en ] || readme="$site/../README.md"
        sed -i "s#https://easi.run/film/$locale-[0-9a-f]*\.avif#https://easi.run/film/$name#" "$readme"
      fi
      sep=','
    done
    printf ' }'
  done
  printf '\n}\n'
} >"$manifest.tmp"
mv "$manifest.tmp" "$manifest"
chmod -R a+rX "$releases/pool/film"
echo "wrote $manifest: commit it, then publish"
