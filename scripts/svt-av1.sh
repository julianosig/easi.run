#!/bin/sh
# Builds SVT-AV1 VERSION (BSD-3-Clause-Clear, AOMedia patent license) from its release source archive as a static
# library, installed under PREFIX (default target/svt-av1). The display links it; point the build at it with
#   SVTAV1_INCLUDE_DIR=PREFIX/include SVTAV1_LIB_DIR=PREFIX/lib
# Needs curl, cmake, nasm, and a C compiler. GitLab generates the archive; should its bytes ever change, the checksum
# fails the build rather than accept different source.
set -eu
PREFIX=$(realpath -m "${1:-target/svt-av1}")
NAME=SVT-AV1
VERSION=3.1.2
SHA256=d0d73bfea42fdcc1222272bf2b0e2319e9df5574721298090c3d28315586ecb1
URL="https://gitlab.com/AOMediaCodec/SVT-AV1/-/archive/v$VERSION/SVT-AV1-v$VERSION.tar.gz"
LIBRARY=libSvtAv1Enc.a
DOC=svt-av1
NOTICES="LICENSE.md LICENSE-BSD2.md PATENTS.md third_party/fastfeat/LICENSE third_party/safestringlib/LICENSE"
# shellcheck source=scripts/static-library.sh
. "$(dirname "$0")/static-library.sh"
# No LTO: GCC's LTO objects do not link into a Rust binary.
static_library -DBUILD_APPS=OFF -DSVT_AV1_LTO=OFF
