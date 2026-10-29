#!/bin/sh
# Measures the line coverage of the tests CI's check job runs, in the development image:
#   scripts/dev/run.sh scripts/coverage.sh
# The Rust ones with cargo-llvm-cov (every crate's unit and integration tests, the display's again with its H.264
# encoder, and display-tests' stackless targets), and the client's with c8 (its unit tests, over every TypeScript module
# in client/src). The end-to-end tests need the stack and are not run, so the code only they reach counts as uncovered.
# Writes target/coverage/rust.lcov and client.lcov, and scripts/coverage.py's total of both into coverage.json, the
# shields.io endpoint the README's badge reads (CI's coverage job publishes it to the badges branch).
set -eu
cd "$(dirname "$0")/.."
out=target/coverage
rm -rf "$out"
mkdir -p "$out"
. scripts/encoders.sh
# The Rust tests as check.sh runs them, built in a target directory of their own with cargo-llvm-cov's instrumenting
# environment, which the display binary the hostile tests build for themselves inherits too. The counters are kept in
# their files as they count (`%c`, which needs the relocation flag on Linux): a test kills the compositor it ran. A test
# binary with no test in it has no counters to keep, and LLVM's runtime says so ("Neither __llvm_profile_counter_bias").
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}/llvm-cov-target"
export RUSTFLAGS="-Cllvm-args=-runtime-counter-relocation"
eval "$(cargo llvm-cov show-env --sh)"
export LLVM_PROFILE_FILE="${LLVM_PROFILE_FILE%.profraw}%c.profraw"
cargo llvm-cov clean --workspace
cargo test --locked --workspace --exclude display-tests
cargo test --locked -p display --features h264
cargo test --locked -p display-tests --lib --bins --test hostile --test seccomp
# Test code is no code under test: the test crate (tests/), the crates' tests/ directories and tests.rs modules, the
# admin's test directory helper (a test-only module of its own), and the benchmark crate; nor is the vendored crate, or
# code generated into the target directory.
cargo llvm-cov report --lcov --output-path "$out/rust.lcov" \
    --ignore-filename-regex "^$PWD/(tests|bench|third_party)/|^$PWD/admin/src/private/test_dir\.rs\$|^$CARGO_TARGET_DIR/|/tests/|/tests\.rs\$"
# The test bundles keep each module whole and map back to it (client/scripts/build.mjs); a module no test imports
# counts as wholly uncovered (`--all`).
(cd client && npm ci --no-audit --no-fund && node scripts/build.mjs --tests \
    && npx --no-install c8 --reporter=lcovonly --report-dir=../target/coverage/c8 --exclude-after-remap --all \
        --src=src --extension=.ts --exclude='test/**' --exclude='**/node_modules/**' node --test .test-out/)
mv "$out/c8/lcov.info" "$out/client.lcov"
rm -rf "$out/c8"
python3 scripts/coverage.py "$out"
