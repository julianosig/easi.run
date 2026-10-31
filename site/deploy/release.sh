#!/bin/sh
# Publish the landing: build the committed site/, copy the build into a new release outside the repository, add its
# hashed assets to the pool every kept release shares, and switch easi-run-site to it atomically. Keeps the last five
# releases (and the assets they use) and every link preview card ever published; `release.sh rollback` switches back to the previous one. One publish or rollback
# at a time.
set -eu

releases="${EASI_RUN_RELEASES:-$HOME/.local/share/easi-run/releases}"
site="$(cd "$(dirname "$0")/.." && pwd)"
# The site builds with the languages and messages it shares with the admin's page (i18n/, beside it).
repo="$(cd "$site/.." && pwd)"
mkdir -p "$releases/pool/assets"

exec 9>"$releases/.lock"
flock -n 9 || { echo "another publish or rollback is running" >&2; exit 1; }

switch() {
  ln -sfn "$1" "$releases/current.tmp"
  mv -T "$releases/current.tmp" "$releases/current"
  echo "live: $1"
}
kept() { find "$releases" -mindepth 1 -maxdepth 1 -printf '%f\n' | grep -xE '[0-9]{14}' | sort; }

if [ "${1:-}" = rollback ]; then
  now="$(readlink "$releases/current")"
  previous="$(kept | grep -B1 -x "$now" | head -n1)"
  if [ -z "$previous" ] || [ "$previous" = "$now" ]; then
    echo "no earlier release" >&2
    exit 1
  fi
  switch "$previous"
  exit 0
fi

# A release is exactly one commit: nothing uncommitted under site/, the locked dependencies, and the commit recorded in
# the release as REVISION.
if [ -n "$(git -C "$site" status --porcelain -- . ../i18n)" ]; then
  echo "site/ or i18n/ has uncommitted changes; commit them first" >&2
  exit 1
fi
revision="$(git -C "$site" rev-parse HEAD)"
# The installer it serves names a whole release, which install.sh itself checks: a commit, and an image digest for each
# role (scripts/pin.py writes them).
if ! git -C "$site" show "$revision:install.sh" | bash -c '. /dev/stdin && resolve_target'; then
  echo "install.sh does not pin a whole release: run scripts/pin.py, commit it, then publish" >&2
  exit 1
fi
# Node 26, as every Node workload here runs, whatever the host has, pinned by digest like every base image (bump the
# tag and digest together). HOME=/tmp keeps npm's cache writable for any uid.
docker run --rm -u "$(id -u):$(id -g)" -e HOME=/tmp -e npm_config_update_notifier=false -v "$repo:/w" -w /w/site \
  docker.io/library/node:26-slim@sha256:36793bcbb160a6a16cede1e62b93f84e26b78a972a8a4c0b879c1c12f22761ec \
  sh -c 'npm ci --no-audit --no-fund && npm run build'
version="$(date -u +%Y%m%d%H%M%S)"
mkdir "$releases/$version"
cp -r "$site/dist/." "$releases/$version/"
echo "$revision" >"$releases/$version/REVISION"
# The installer, at https://easi.run/install.sh: the repository's own, as the release's commit holds it.
git -C "$site" show "$revision:install.sh" >"$releases/$version/install.sh"
# A pool file appears only whole: copied under a temporary name, then renamed into place.
for asset in "$releases/$version/assets/"*; do
  target="$releases/pool/assets/$(basename "$asset")"
  [ -e "$target" ] && continue
  cp "$asset" "$target.tmp.$$"
  mv "$target.tmp.$$" "$target"
done
# The link previews' cards join the pool too, and stay there: a post anywhere keeps naming the card it was shared with.
for card in "$releases/$version/og/"*/*.png; do
  target="$releases/pool/og/${card#"$releases/$version/og/"}"
  [ -e "$target" ] && continue
  mkdir -p "$(dirname "$target")"
  cp "$card" "$target.tmp.$$"
  mv "$target.tmp.$$" "$target"
done
chmod -R a+rX "$releases/$version" "$releases/pool"
switch "$version"

# Drop releases beyond the last five, then pool assets no kept release uses.
kept | head -n -5 | while read -r old; do rm -rf "${releases:?}/$old"; done
for asset in "$releases/pool/assets/"*; do
  name="$(basename "$asset")"
  kept | while read -r r; do [ -e "$releases/$r/assets/$name" ] && echo used; done | grep -q used || rm -f "$asset"
done
