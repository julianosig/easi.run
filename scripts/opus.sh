#!/bin/sh
# Builds libopus VERSION (BSD-3-Clause) from its release source archive as a static library, installed under PREFIX
# (default target/opus). The display links it; point the build at it with OPUS_LIB_DIR=PREFIX/lib.
# Needs curl, cmake, and a C compiler. The checksum is the one Xiph publishes; different bytes fail the build.
set -eu
PREFIX=$(realpath -m "${1:-target/opus}")
VERSION=1.5.2
SHA256=65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1
if [ -f "$PREFIX/lib/libopus.a" ] && [ -f "$PREFIX/share/doc/opus/COPYING" ] && [ "$(cat "$PREFIX/.version" 2>/dev/null)" = "$VERSION" ]; then
    echo "libopus $VERSION is already in $PREFIX"
    exit 0
fi
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
curl -fsSL --retry 5 --retry-all-errors -o "$WORK/opus.tar.gz" "https://downloads.xiph.org/releases/opus/opus-$VERSION.tar.gz"
echo "$SHA256  $WORK/opus.tar.gz" | sha256sum -c -
mkdir "$WORK/src"
tar -xzf "$WORK/opus.tar.gz" -C "$WORK/src" --strip-components=1
cmake -S "$WORK/src" -B "$WORK/build" -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DOPUS_BUILD_PROGRAMS=OFF -DOPUS_BUILD_TESTING=OFF \
    -DCMAKE_INSTALL_PREFIX="$PREFIX" -DCMAKE_INSTALL_LIBDIR=lib
cmake --build "$WORK/build" -j "$(nproc)"
cmake --install "$WORK/build"
# Its license, patent and notice texts, which every binary linking it ships (the display image, in
# /usr/share/doc/easi.run/opus/).
for file in COPYING AUTHORS; do install -D -m 0644 "$WORK/src/$file" "$PREFIX/share/doc/opus/$file"; done
echo "$VERSION" > "$PREFIX/.version"
