#!/bin/bash
# Rebuilds the images a change affects and runs the end-to-end tests that exercise them, as scripts/affected.py
# selects them (same arguments: nothing, BASE, or --all), and the controller's tests against the real Docker Engine
# when the controller can be affected. The end-to-end tests need the bench stack's admin password in ADMIN_PASSWORD.
#
# The suites run side by side against the bench stack's admin, each in a test process of its own (a test stops only
# the sessions it started), beside the controller's tests; then the tests in a module named `exclusive`, which change
# the admin for everyone (its second step, a restart, its whole session list), run alone. Each suite's output is
# printed whole, with its result and time, as it ends. The run fails if any of them failed, or unless every test of
# the selected suites ran exactly once.
set -euo pipefail
# `wait -n -p`, below, came with Bash 5.1.
if ((BASH_VERSINFO[0] * 100 + BASH_VERSINFO[1] < 501)); then
    echo "scripts/e2e.sh needs Bash 5.1 or later, not $BASH_VERSION" >&2
    exit 1
fi
cd "$(dirname "$0")/.."
mkdir -p target/e2e
# One run at a time on this host, from any checkout: another would recreate the bench stack under this one's tests. An
# interrupted run's processes that outlive it (a build it started) keep the lock until they end.
exec 9>/tmp/easi-run-bench-e2e.lock
flock -n 9 || { echo "another scripts/e2e.sh runs against the bench stack" >&2; exit 1; }
plan=$(python3 scripts/affected.py "$@")
args() { echo "$plan" | sed -n "s/^$1 //p"; }
images=$(args images)
# The selected suites, from affected.py's `--test NAME` pairs.
read -r -a suites <<<"$(args e2e | sed 's/--test //g')"
controller=""
if echo " $(args crates) " | grep -q " controller "; then
    controller=1
fi
# Checked before anything runs: a plan without end-to-end tests needs no password.
if [ ${#suites[@]} -gt 0 ]; then
    : "${ADMIN_PASSWORD:?the admin password of the running bench stack}"
fi
# The controller's session lifecycle (creation, rollback, orphan reclaim, cleanup) against this host's engine, as an
# instance of its own beside the bench stack's.
controller_test=(cargo test --locked -p controller --test docker -- --ignored)
if [ ${#suites[@]} -eq 0 ]; then
    if [ -n "$controller" ]; then
        "${controller_test[@]}"
    fi
    echo "no end-to-end test is affected"
    exit 0
fi
if [ -n "$controller" ]; then
    cargo test --locked --no-run -p controller --test docker
fi
export DOCKER_GID="${DOCKER_GID:-$(stat -c %g /var/run/docker.sock)}"
# The project named outright, so a COMPOSE_PROJECT_NAME in the environment cannot point this at another installation.
compose="docker compose -p easi-run-bench -f compose.yaml -f compose.bench.yaml"
services=""
for image in $images; do
    case $image in
        display | browser | agent | proxy) services="$services $image-image" ;;
        tools) ;; # built below, whenever a test that uses it runs
        *) services="$services $image" ;;
    esac
done
# ffmpeg and perl for the stream and audio tests, which run it with --pull never: a cached build takes a second.
case " ${suites[*]} " in
    *" stream "* | *" audio "*) docker build -q -t easi-run/tools:dev bench/tools >/dev/null ;;
esac
if [ -n "$services" ]; then
    # shellcheck disable=SC2086
    $compose build $services
fi
$compose up -d
# The controller resolves the session image tags when it starts; the admin starts afresh with it, ending the sessions
# an earlier run left (the new controller no longer holds them).
$compose up -d --no-deps --force-recreate controller admin
# The admin the tests talk to: the bench stack's, on the port compose.bench.yaml publishes it at, whatever the
# environment says (the tests start and stop sessions of the admin they reach).
export ADMIN_ORIGIN=https://localhost:9444
selected=()
for suite in "${suites[@]}"; do
    selected+=(--test "$suite")
done
cargo test --locked --no-run -p display-tests "${selected[@]}"
# The probes the security tests build as they start: built now, so those tests do not wait on each other for it.
case " ${suites[*]} " in
    *" security "*) cargo build --locked --release -p display-tests --bins ;;
esac
# Against an admin that never came up, every test would fail on its own, far from the cause: stop here instead.
up=""
for _ in $(seq 60); do
    curl -ksf -o /dev/null "$ADMIN_ORIGIN/api/status" && { up=1; break; }
    sleep 1
done
if [ -z "$up" ]; then
    echo "the admin did not answer $ADMIN_ORIGIN/api/status within 60 s; see: $compose logs admin" >&2
    exit 1
fi

out=target/e2e
rm -f -- "${out:?}"/*.list "${out:?}"/*.log "${out:?}"/*.end "${out:?}"/*.results
# Every test of each suite, which the two phases together must run exactly once.
for suite in "${suites[@]}"; do
    cargo test --locked -q -p display-tests --test "$suite" -- --include-ignored --list --format terse 2>/dev/null |
        sed -n 's/: test$//p' | sort >"$out/$suite.list"
done
# Test threads per suite: three in stream and security, the longest, two in the next longest, one elsewhere; agents
# must keep one (its tests revoke every token an earlier one left). profiles, the shortest, runs after features in the
# same job when both run, so the two hold one session between them. Each test holds one session at a time, so all of
# them together must stay under compose.bench.yaml's ADMIN_MAX_SESSIONS with room to spare: a refusal for the limit
# would hide the one a test expects (a saved session in use).
threads() {
    case $1 in
        security | stream) echo 3 ;;
        control | files) echo 2 ;;
        *) echo 1 ;;
    esac
}
chained=""
[[ " ${suites[*]} " == *" features "* && " ${suites[*]} " == *" profiles "* ]] && chained=1
sessions=0
for suite in "${suites[@]}"; do
    [ -n "$chained" ] && [ "$suite" = profiles ] && continue
    sessions=$((sessions + $(threads "$suite")))
done
limit=$(sed -n 's/^ *ADMIN_MAX_SESSIONS: "\([0-9]*\)"$/\1/p' compose.bench.yaml)
if [ "$sessions" -gt $((limit - 2)) ]; then
    echo "e2e: $sessions sessions at once leave no room under ADMIN_MAX_SESSIONS ($limit): fewer threads" >&2
    exit 1
fi
start=$(date +%s.%N)
seconds() { awk -v from="$1" -v to="$(date +%s.%N)" 'BEGIN { printf "%.1f", to - from }'; }
# Runs a job in the background, in a process group of its own, its output in "$out/$1.log", and its exit status and
# time in "$out/$1.end".
declare -A job_name
# Interrupted, every process of the jobs stops too.
trap 'for pid in "${!job_name[@]}"; do kill -- "-$pid" 2>/dev/null || true; done' EXIT
set -m
job() {
    local name=$1
    shift
    (
        began=$(date +%s.%N)
        set +e
        "$@" >"$out/$name.log" 2>&1
        echo "$? $(seconds "$began")" >"$out/$name.end"
    ) &
    job_name[$!]=$name
}
failed=()
summary=()
# Waits for every job, printing each one's output, result and time as it ends.
finish() {
    local pid name rc took
    while [ ${#job_name[@]} -gt 0 ]; do
        wait -n -p pid "${!job_name[@]}" || true
        name=${job_name[$pid]}
        unset "job_name[$pid]"
        read -r rc took <"$out/$name.end"
        cat "$out/$name.log"
        if [ "$rc" = 0 ]; then
            echo "=== $name: ok in $took s"
        else
            echo "=== $name: FAILED (exit $rc) in $took s"
            failed+=("$name")
        fi
        summary+=("$(printf '%-24s %-6s %7s s' "$name" "$([ "$rc" = 0 ] && echo ok || echo FAILED)" "$took")")
    done
}
if [ -n "$controller" ]; then
    job controller "${controller_test[@]}"
fi
# A suite's tests but the exclusive ones; then, for a chain, each suite in turn, all of them run whatever one gave.
suite() {
    cargo test --locked -p display-tests --test "$1" -- --include-ignored --skip exclusive:: \
        --test-threads="$(threads "$1")" --logfile "$PWD/$out/$1.results"
}
chain() {
    local rc=0 one
    for one in "$@"; do suite "$one" || rc=$?; done
    return "$rc"
}
for name in "${suites[@]}"; do
    if [ -n "$chained" ]; then
        case $name in
            profiles) continue ;;
            features) job features+profiles chain features profiles; continue ;;
        esac
    fi
    job "$name" suite "$name"
done
finish
parallel=$(seconds "$start")
start=$(date +%s.%N)
for suite in "${suites[@]}"; do
    if grep -q '^exclusive::' "$out/$suite.list"; then
        job "$suite-exclusive" cargo test --locked -p display-tests --test "$suite" -- --include-ignored exclusive:: \
            --test-threads=1 --logfile "$PWD/$out/$suite-exclusive.results"
        finish
    fi
done
exclusive=$(seconds "$start")

# Each listed test ran exactly once, in one phase or the other: a filter that skipped one, or a suite that ended
# early, fails the run. libtest's result logs say it apart from what the tests print.
missed=""
for suite in "${suites[@]}"; do
    # A phase that never wrote its log (no exclusive test, or a suite that did not start) adds nothing.
    ran=$(cat "$out/$suite.results" "$out/$suite-exclusive.results" 2>/dev/null | sed -En 's/^(ok|failed) //p' |
        sort || true)
    never=$(comm -23 "$out/$suite.list" <(echo "$ran" | sort -u))
    twice=$(echo "$ran" | uniq -d)
    unknown=$(comm -13 "$out/$suite.list" <(echo "$ran" | sort -u))
    for name in $never; do missed="$missed\n$suite: $name never ran"; done
    for name in $twice; do missed="$missed\n$suite: $name ran twice"; done
    for name in $unknown; do missed="$missed\n$suite: $name ran but was not listed"; done
done

echo
printf '%s\n' "${summary[@]}"
echo "side by side $parallel s, alone $exclusive s"
if [ -n "$missed" ]; then
    printf '%b\n' "not every test ran exactly once:$missed" >&2
    exit 1
fi
if [ ${#failed[@]} -gt 0 ]; then
    echo "failed: ${failed[*]}" >&2
    exit 1
fi
