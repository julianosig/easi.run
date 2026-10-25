#!/bin/sh
# The checks a change needs before review, chosen by scripts/affected.py from the files it touches:
#   scripts/check.sh            the working tree against HEAD
#   scripts/check.sh BASE       everything since BASE, plus the working tree
#   scripts/check.sh --all      every check (what CI's check job runs)
# Formatting always runs; scripts/affected.py's docstring says what each line of its plan selects. The end-to-end tests
# need the running stack, so they are printed instead; scripts/e2e.sh rebuilds those images and runs them.
#
# Needs what CONTRIBUTING.md's Requirements list (scripts/dev/run.sh runs it in the development image, which has it
# all). SVT-AV1 and libopus come from target/svt-av1 and target/opus, which scripts/svt-av1.sh and scripts/opus.sh
# build, unless SVTAV1_* and OPUS_LIB_DIR are set.
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
    # Only the stackless integration targets the change can affect: the end-to-end ones are ignored without the stack.
    # shellcheck disable=SC2046
    cargo test --locked -p display-tests --lib --bins $(args stackless)
fi
if selected client; then
    (cd client && npm ci --no-audit --no-fund && npm test)
fi
# The README's comparison is written from the landing's, and the docs list every setting the code reads: a second's
# work each, whatever changed.
python3 scripts/compare.py --check
python3 scripts/settings.py
if selected site; then
    python3 scripts/tools.py --site
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
    # Its Compose file and nginx configuration need Docker, which the development image does not have.
    echo "next: on the host, the demo's configuration check in CI's images job (.github/workflows/ci.yml)"
fi
if selected scripts; then
    # Every script, tracked or not yet added (ignored ones aside), with the linters' default rules: few and quick.
    git ls-files -z --cached --others --exclude-standard '*.sh' | xargs -0 shellcheck
    git ls-files -z --cached --others --exclude-standard '*.py' | xargs -0 ruff check --no-cache --quiet
fi
if selected installer; then
    bash scripts/install-test.sh
fi
if selected pins; then
    python3 scripts/pins.py
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
# A change to an end-to-end test or to what one runs selects it without rebuilding any image.
if selected e2e && selected images; then
    echo "next: rebuild $(args images), then run the end-to-end tests: $(args e2e)"
elif selected e2e; then
    echo "next: run the end-to-end tests: $(args e2e)"
elif selected images; then
    echo "next: rebuild $(args images)"
fi
if echo " $(args crates) " | grep -q " controller "; then
    # It needs the host's Docker Engine, which the development image does not have.
    echo "next: on the host (scripts/e2e.sh runs it), cargo test -p controller --test docker -- --ignored"
fi
