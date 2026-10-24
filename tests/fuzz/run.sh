#!/bin/sh
# Runs every fuzz target for SECONDS (default 3600) in parallel, prints each one's final line, and fails when any
# target fails.
# Needs cargo-fuzz and a nightly toolchain (FUZZ_TOOLCHAIN, default `nightly`), and for `microphone_packet` libopus
# as the display links it (OPUS_LIB_DIR; scripts/opus.sh builds it into the repository's target/opus when unset).
set -eu
cd "$(dirname "$0")"
if [ -z "${OPUS_LIB_DIR:-}" ]; then
    ../../scripts/opus.sh ../../target/opus
    OPUS_LIB_DIR=$(cd ../../target/opus/lib && pwd)
    export OPUS_LIB_DIR
fi
SECONDS_EACH=${1:-3600}
TOOLCHAIN=${FUZZ_TOOLCHAIN:-nightly}
# Every [[bin]] in Cargo.toml.
TARGETS=$(cargo "+$TOOLCHAIN" fuzz list)
mkdir -p results
for t in $TARGETS; do
    mkdir -p "corpus/$t"
    cargo "+$TOOLCHAIN" fuzz run "$t" "corpus/$t" -- -max_total_time="$SECONDS_EACH" -rss_limit_mb=2048 -timeout=10 \
        >"results/$t.log" 2>&1 &
    pids="${pids:-} $!"
done
failed=0
for pid in $pids; do
    wait "$pid" || failed=1
done
for t in $TARGETS; do
    line=$(grep -E "Done [0-9]+ runs|ERROR|crash-|deadly signal" "results/$t.log" | tail -1)
    echo "$t: ${line:-no result}"
done
exit "$failed"
