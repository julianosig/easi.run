#!/bin/sh
# Builds SVT-AV1 VERSION (BSD-3-Clause-Clear, AOMedia patent license) from its release source archive as a static
# library, installed under PREFIX (default target/svt-av1). The display links it; point the build at it with
#   SVTAV1_INCLUDE_DIR=PREFIX/include SVTAV1_LIB_DIR=PREFIX/lib
# Needs curl, cmake, nasm, and a C compiler. GitLab generates the archive; should its bytes ever change, the checksum
# fails the build rather than accept different source.
set -eu
PREFIX=$(realpath -m "${1:-target/svt-av1}")
VERSION=3.1.2
SHA256=d0d73bfea42fdcc1222272bf2b0e2319e9df5574721298090c3d28315586ecb1
if [ -f "$PREFIX/lib/libSvtAv1Enc.a" ] && [ -f "$PREFIX/share/doc/svt-av1/LICENSE.md" ] &&
    [ "$(cat "$PREFIX/.version" 2>/dev/null)" = "$VERSION" ]; then
    echo "SVT-AV1 $VERSION is already in $PREFIX"
    exit 0
fi
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
curl -fsSL --retry 5 --retry-all-errors -o "$WORK/svt.tar.gz" "https://gitlab.com/AOMediaCodec/SVT-AV1/-/archive/v$VERSION/SVT-AV1-v$VERSION.tar.gz"
echo "$SHA256  $WORK/svt.tar.gz" | sha256sum -c -
mkdir "$WORK/src"
tar -xzf "$WORK/svt.tar.gz" -C "$WORK/src" --strip-components=1
# No LTO: GCC's LTO objects do not link into a Rust binary.
cmake -S "$WORK/src" -B "$WORK/build" -DCMAKE_BUILD_TYPE=Release -DBUILD_APPS=OFF -DSVT_AV1_LTO=OFF \
    -DBUILD_SHARED_LIBS=OFF -DCMAKE_INSTALL_PREFIX="$PREFIX"
cmake --build "$WORK/build" -j "$(nproc)"
cmake --install "$WORK/build"
# Its license, patent and notice texts, which every binary linking it ships (the display image, in
# /usr/share/doc/easi.run/svt-av1/).
for file in LICENSE.md LICENSE-BSD2.md PATENTS.md third_party/fastfeat/LICENSE third_party/safestringlib/LICENSE; do install -D -m 0644 "$WORK/src/$file" "$PREFIX/share/doc/svt-av1/$file"; done
echo "$VERSION" > "$PREFIX/.version"
