#!/bin/sh
# Builds libopus VERSION (BSD-3-Clause) from its release source archive as a static library, installed under PREFIX
# (default target/opus). The display links it; point the build at it with OPUS_LIB_DIR=PREFIX/lib.
# Needs curl, cmake, and a C compiler. The checksum is the one Xiph publishes; different bytes fail the build.
set -eu
PREFIX=$(realpath -m "${1:-target/opus}")
NAME=libopus
VERSION=1.5.2
SHA256=65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1
URL="https://downloads.xiph.org/releases/opus/opus-$VERSION.tar.gz"
LIBRARY=libopus.a
DOC=opus
NOTICES="COPYING AUTHORS"
# shellcheck source=scripts/static-library.sh
. "$(dirname "$0")/static-library.sh"
static_library -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DOPUS_BUILD_PROGRAMS=OFF -DOPUS_BUILD_TESTING=OFF \
    -DCMAKE_INSTALL_LIBDIR=lib
