#!/bin/sh
# Checks that each crate under third_party/ is its crates.io release with third_party/NAME.patch applied, and nothing
# else: Cargo.lock holds no checksum for a [patch.crates-io] path, so nothing else would notice another edit to a
# library that parses hostile input (every Wayland client's bytes; every peer of the admin's UDP port). Needs the
# network (static.crates.io).
set -eu
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# Each crate, its version, and the release's checksum as the crates.io index lists it.
while read -r name version sha256; do
    crate=$name-$version
    curl -fsS --retry 5 --retry-all-errors --proto '=https' -o "$tmp/$crate.crate" "https://static.crates.io/crates/$name/$crate.crate"
    echo "$sha256  $tmp/$crate.crate" | sha256sum -c --quiet
    tar -xzf "$tmp/$crate.crate" -C "$tmp"
    patch -s -p1 -d "$tmp/$crate" <"third_party/$name.patch"
    diff -r "$tmp/$crate" "third_party/$name"
done <<'CRATES'
wayland-backend 0.3.17 38a91b4eaddff87b1cd1074985e3713da4af2c49742d1b356b2c01670a67a078
wtransport 0.7.2 b4273ce3157a3262a68665f8d3f20a0ac0c5b8a69ffd67f05ae986832ebec036
CRATES
