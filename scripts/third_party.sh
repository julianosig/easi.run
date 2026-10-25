#!/bin/sh
# Checks that third_party/wayland-backend is the crates.io release with third_party/wayland-backend.patch applied, and
# nothing else: Cargo.lock holds no checksum for a [patch.crates-io] path, so nothing else would notice another edit
# to the library that parses every hostile Wayland client's bytes. Needs the network (static.crates.io).
set -eu
cd "$(dirname "$0")/.."
crate=wayland-backend-0.3.17
# The release's checksum as the crates.io index lists it.
sha256=38a91b4eaddff87b1cd1074985e3713da4af2c49742d1b356b2c01670a67a078
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsS --retry 5 --retry-all-errors --proto '=https' -o "$tmp/$crate.crate" "https://static.crates.io/crates/wayland-backend/$crate.crate"
echo "$sha256  $tmp/$crate.crate" | sha256sum -c --quiet
tar -xzf "$tmp/$crate.crate" -C "$tmp"
patch -s -p1 -d "$tmp/$crate" <third_party/wayland-backend.patch
diff -r "$tmp/$crate" third_party/wayland-backend
