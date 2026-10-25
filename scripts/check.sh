#!/usr/bin/env bash
# The checks a change needs before review, chosen by scripts/affected.py from the files it touches:
#   scripts/check.sh            the working tree against HEAD
#   scripts/check.sh BASE       everything since BASE, plus the working tree
#   scripts/check.sh --all      every check (what CI's check job runs)
# Formatting always runs; scripts/affected.py's docstring says what each line of its plan selects. The end-to-end tests
# need the running stack, so they are printed instead; scripts/e2e.sh rebuilds those images and runs them.
#
# The checks run in lanes at the same time. A lane stops at its first failure; its output is printed whole once it
# ends, in the order below, with whether it passed; and the run fails if any lane failed.
#
# Needs what CONTRIBUTING.md's Requirements list (scripts/dev/run.sh runs it in the development image, which has it
# all). SVT-AV1 and libopus come from target/svt-av1 and target/opus, which scripts/svt-av1.sh and scripts/opus.sh
# build, unless SVTAV1_* and OPUS_LIB_DIR are set.
set -eu
cd "$(dirname "$0")/.."
plan=$(python3 scripts/affected.py "$@")
while IFS= read -r line; do echo "plan: $line"; done <<<"$plan"
selected() { echo "$plan" | grep -q "^$1\( \|$\)"; }
args() { echo "$plan" | sed -n "s/^$1 //p"; }
crate() { echo " $(args crates) " | grep -q " $1 "; }

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

logs=$(mktemp -d)
trap 'rm -rf "$logs"' EXIT
lanes=()
declare -A pid
# Stopped (a CI job's timeout, say), it stops every lane (SIGTERM, then SIGKILL for one still there 10 s later) and
# prints what each one not printed yet had printed.
stop() {
    trap - INT TERM
    local name running=" " deadline=$((SECONDS + 10))
    for name in "${lanes[@]}"; do
        [ -f "$logs/$name.end" ] || running="$running$name "
        kill -TERM -- "-${pid[$name]}" 2>/dev/null || true
    done
    while [ -n "$(jobs -pr)" ] && [ "$SECONDS" -lt "$deadline" ]; do sleep 0.2; done
    for name in "${lanes[@]}"; do
        kill -KILL -- "-${pid[$name]}" 2>/dev/null || true
    done
    wait
    for name in "${lanes[@]}"; do
        [ ! -f "$logs/$name.shown" ] || continue
        state=ended
        [[ "$running" != *" $name "* ]] || state="still running"
        echo "== $name: $state when check.sh was stopped"
        cat "$logs/$name"
    done
    exit 1
}
trap stop INT TERM
# Each lane is a process group of its own, which a stop ends whole.
set -m
# Starts lane $1: the command after it, in a shell of its own with its output in $logs/$1, and the times it started and
# ended in $logs/$1.start and $logs/$1.end.
lane() {
    local name=$1
    shift
    lanes+=("$name")
    date +%s >"$logs/$name.start"
    (
        trap 'date +%s >"$logs/$name.end"' EXIT
        # A stop's SIGTERM is for the lane's commands, which may print as they stop: the lane ends once they do.
        trap : TERM
        "$@"
    ) </dev/null >"$logs/$name" 2>&1 &
    pid[$name]=$!
}

formatting() {
    cargo fmt --all --check
    # The fuzz targets are a workspace of their own, which `--all` does not reach.
    cargo fmt --manifest-path tests/fuzz/Cargo.toml --check
}
lane formatting formatting

rust() {
    # Each plan's crates by name, as scripts/cargo-test.py builds their tests: the features they unify to are theirs
    # alone, so a crate that needs a dependency's feature it does not ask for fails here.
    if selected crates; then
        packages=$(for c in $(args crates); do printf -- '-p %s ' "$c"; done)
        # shellcheck disable=SC2086
        cargo clippy --locked --all-targets $packages -- -D warnings
    fi
    if selected stackless; then
        cargo clippy --locked --all-targets -p display-tests -- -D warnings
    fi
    # The selected crates' tests, and display-tests' stackless ones (only the integration targets the change can
    # affect: the end-to-end ones are ignored without the stack), every test binary at the same time.
    echo "$plan" | python3 scripts/cargo-test.py
    if selected fuzz; then
        # The fuzz targets use the protocol crate's API; they build on stable (only running them needs nightly).
        cargo clippy --locked --manifest-path tests/fuzz/Cargo.toml --bins -- -D warnings
    fi
}
if selected crates || selected stackless || selected fuzz; then
    lane rust rust
fi

# The display's optional H.264 encoder builds and passes too.
h264() {
    cargo clippy --locked --all-targets -p display --features h264 -- -D warnings
    cargo test --locked -p display --features h264
}
if crate display; then
    lane h264 h264
fi

if selected client; then
    lane client sh -c 'cd client && npm ci --no-audit --no-fund && npm test'
fi
if selected site; then
    lane site sh -c 'python3 scripts/tools.py --site && cd site && npm ci --no-audit --no-fund && npm run build'
fi
if selected installer; then
    lane installer bash scripts/install-test.sh
fi
if selected scripts; then
    # Every script, tracked or not yet added (ignored ones aside), with the linters' default rules: few and quick.
    lane scripts sh -c "git ls-files -z --cached --others --exclude-standard '*.sh' | xargs -0 shellcheck &&
        git ls-files -z --cached --others --exclude-standard '*.py' | xargs -0 ruff check --no-cache --quiet"
fi

# The rest, a second's work or less each.
others() {
    # The README's comparison is written from the landing's, and the docs list every setting the code reads, whatever
    # changed.
    python3 scripts/compare.py --check
    python3 scripts/settings.py
    if selected licenses; then
        python3 scripts/licenses.py
        git diff --exit-code -- docs/licenses.md
    fi
    if selected demo; then
        python3 scripts/demo.py
        # Its Compose file and nginx configuration need Docker, which the development image does not have.
        echo "next: on the host, the demo's configuration check in CI's images job (.github/workflows/ci.yml)"
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
}
lane others others

failed=""
for name in "${lanes[@]}"; do
    status=0
    wait "${pid[$name]}" || status=$?
    verdict=passed
    if [ "$status" != 0 ]; then
        verdict="FAILED (exit status $status)"
        failed="$failed $name"
    fi
    start=$(cat "$logs/$name.start")
    end=$(cat "$logs/$name.end" 2>/dev/null || date +%s)
    echo "== $name: $verdict in $((end - start)) s"
    cat "$logs/$name"
    : >"$logs/$name.shown"
done
if [ -n "$failed" ]; then
    echo "check.sh: failed:$failed" >&2
    exit 1
fi

# A change to an end-to-end test or to what one runs selects it without rebuilding any image.
if selected e2e && selected images; then
    echo "next: rebuild $(args images), then run the end-to-end tests: $(args e2e)"
elif selected e2e; then
    echo "next: run the end-to-end tests: $(args e2e)"
elif selected images; then
    echo "next: rebuild $(args images)"
fi
if crate controller; then
    # It needs the host's Docker Engine, which the development image does not have.
    echo "next: on the host (scripts/e2e.sh runs it), cargo test -p controller --test docker -- --ignored"
fi
