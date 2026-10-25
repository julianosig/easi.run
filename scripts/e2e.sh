#!/bin/sh
# Rebuilds the images a change affects and runs the end-to-end tests that exercise them, as scripts/affected.py
# selects them (same arguments: nothing, BASE, or --all), and the controller's tests against the real Docker Engine
# when the controller can be affected. The end-to-end tests need the bench stack's admin password in ADMIN_PASSWORD.
set -eu
cd "$(dirname "$0")/.."
plan=$(python3 scripts/affected.py "$@")
args() { echo "$plan" | sed -n "s/^$1 //p"; }
images=$(args images)
tests=$(args e2e)
# Checked before the controller's tests run: a plan without end-to-end tests needs no password.
if [ -n "$tests" ]; then
    : "${ADMIN_PASSWORD:?the admin password of the running bench stack}"
fi
# The controller's session lifecycle (creation, rollback, orphan reclaim, cleanup) against this host's engine, as an
# instance of its own beside the bench stack's.
if echo " $(args crates) " | grep -q " controller "; then
    cargo test --locked -p controller --test docker -- --ignored
fi
if [ -z "$tests" ]; then
    echo "no end-to-end test is affected"
    exit 0
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
case " $tests " in
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
# environment says (the tests stop every session of the admin they reach).
export ADMIN_ORIGIN=https://localhost:9444
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
# shellcheck disable=SC2086
cargo test --locked --no-fail-fast -p display-tests $tests -- --ignored --test-threads=1
