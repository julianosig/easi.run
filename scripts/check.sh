#!/bin/sh
# The checks a change needs before review, chosen by scripts/affected.py from the files it touches:
#   scripts/check.sh            the working tree against HEAD
#   scripts/check.sh BASE       everything since BASE, plus the working tree
#   scripts/check.sh --all      every check (what CI runs)
# Formatting always runs. Lints and tests run for the affected crates and their dependents; the stackless integration
# tests (the hostile clients, the browser seccomp profile's rules) when the display, that profile, or the test harness
# can be affected; the client tests, the static pages' check and build, the license report, the fuzz targets' lints,
# the public demo's host lists, the benchmark baseline's Chrome pin, the vendored crate against its release, and the
# egress Compose overlay when their inputs change. The end-to-end tests need the running stack, so they are printed
# instead; scripts/e2e.sh rebuilds those images and runs them.
#
# Needs the toolchain in rust-toolchain.toml, Node 26, Python 3, libxkbcommon headers, SVT-AV1 (scripts/svt-av1.sh
# builds it into target/svt-av1, which this script uses when SVTAV1_* are unset), and libopus (scripts/opus.sh, into
# target/opus when OPUS_LIB_DIR is unset).
set -eu
cd "$(dirname "$0")/.."
plan=$(python3 scripts/affected.py "$@")
echo "$plan" | sed 's/^/plan: /'
selected() { echo "$plan" | grep -q "^$1\( \|$\)"; }
args() { echo "$plan" | sed -n "s/^$1 //p"; }

cargo fmt --all --check
# The fuzz targets are a workspace of their own, which `--all` does not reach.
cargo fmt --manifest-path tests/fuzz/Cargo.toml --check
if selected crates || selected stackless; then
    if [ -z "${SVTAV1_LIB_DIR:-}" ]; then
        scripts/svt-av1.sh target/svt-av1
        export SVTAV1_INCLUDE_DIR="$PWD/target/svt-av1/include" SVTAV1_LIB_DIR="$PWD/target/svt-av1/lib"
    fi
    if [ -z "${OPUS_LIB_DIR:-}" ]; then
        scripts/opus.sh target/opus
        export OPUS_LIB_DIR="$PWD/target/opus/lib"
    fi
fi
if selected crates; then
    packages=$(for c in $(args crates); do printf -- '-p %s ' "$c"; done)
    # shellcheck disable=SC2086
    cargo clippy --locked --all-targets $packages -- -D warnings
    # shellcheck disable=SC2086
    cargo test --locked $packages
    # The display's optional H.264 encoder builds and passes too.
    if echo " $(args crates) " | grep -q " display "; then
        cargo clippy --locked --all-targets -p display --features h264 -- -D warnings
        cargo test --locked -p display --features h264
    fi
fi
if selected stackless; then
    cargo clippy --locked --all-targets -p display-tests -- -D warnings
    # Only the stackless integration targets the change can affect: the end-to-end ones would skip themselves.
    targets=$(for t in $(args stackless); do printf -- '--test %s ' "$t"; done)
    # shellcheck disable=SC2086
    cargo test --locked -p display-tests --lib --bins $targets
fi
if selected client; then
    (cd client && npm ci --no-audit --no-fund && npm test)
fi
if selected site; then
    (cd site && npm ci --no-audit --no-fund && npm run build)
fi
if selected licenses; then
    python3 scripts/licenses.py
    git diff --exit-code -- docs/licenses.md
fi
if selected fuzz; then
    # The fuzz targets use the protocol crate's API; they build on stable (only running them needs nightly).
    cargo clippy --locked --manifest-path tests/fuzz/Cargo.toml --bins -- -D warnings
fi
if selected demo; then
    python3 scripts/demo.py
fi
if selected chrome; then
    python3 scripts/chrome.py
fi
if selected vendored; then
    scripts/third_party.sh
fi
if selected compose; then
    # The egress overlay on the base file; the development image has no Docker, and CI's images job runs it.
    if command -v docker >/dev/null; then
        docker compose -f compose.yaml -f compose.egress.yaml config -q
    else
        echo "next: on the host, docker compose -f compose.yaml -f compose.egress.yaml config -q"
    fi
fi
if selected images; then
    echo "next: rebuild $(args images), then run the end-to-end tests: $(args e2e)"
fi
if echo " $(args crates) " | grep -q " controller "; then
    # It needs the host's Docker Engine, which the development image does not have.
    echo "next: on the host (scripts/e2e.sh runs it), CONTROLLER_DOCKER_TESTS=1 cargo test -p controller --test docker"
fi
