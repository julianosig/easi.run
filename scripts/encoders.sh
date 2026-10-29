# shellcheck shell=sh
# Sourced from the repository root by scripts/check.sh and scripts/coverage.sh: SVT-AV1 and libopus, which the display
# links, built into target/svt-av1 and target/opus by scripts/svt-av1.sh and scripts/opus.sh and exported for its
# build, unless SVTAV1_* and OPUS_LIB_DIR are set.
if [ -z "${SVTAV1_LIB_DIR:-}" ]; then
    scripts/svt-av1.sh target/svt-av1
    export SVTAV1_INCLUDE_DIR="$PWD/target/svt-av1/include" SVTAV1_LIB_DIR="$PWD/target/svt-av1/lib"
fi
if [ -z "${OPUS_LIB_DIR:-}" ]; then
    scripts/opus.sh target/opus
    export OPUS_LIB_DIR="$PWD/target/opus/lib"
fi
