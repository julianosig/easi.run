#!/bin/sh
# Rebuilds the images a change affects and runs the end-to-end tests that exercise them, as scripts/affected.py
# selects them (same arguments: nothing, BASE, or --all), and the controller's tests against the real Docker Engine
# when the controller can be affected. The end-to-end tests need the bench stack's admin password in ADMIN_PASSWORD.
set -eu
cd "$(dirname "$0")/.."
plan=$(python3 scripts/affected.py "$@")
images=$(echo "$plan" | sed -n 's/^images //p')
tests=$(echo "$plan" | sed -n 's/^e2e //p')
# Checked before the controller's tests run: a plan without end-to-end tests needs no password.
if [ -n "$tests" ]; then
    : "${ADMIN_PASSWORD:?the admin password of the running bench stack}"
fi
# The controller's session lifecycle (creation, rollback, orphan reclaim, cleanup) against this host's engine, as an
# instance of its own beside the bench stack's.
if echo " $(echo "$plan" | sed -n 's/^crates //p') " | grep -q " controller "; then
    cargo test --locked -p controller --test docker -- --ignored
fi
if [ -z "$tests" ]; then
    echo "no end-to-end test is affected"
    exit 0
fi
export DOCKER_GID="${DOCKER_GID:-$(stat -c %g /var/run/docker.sock)}"
compose="docker compose -f compose.yaml -f compose.bench.yaml"
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
# The controller resolves the session image tags when it starts.
$compose restart controller
# The admin the tests talk to: the bench stack's, on the port Compose publishes it at.
export ADMIN_ORIGIN="${ADMIN_ORIGIN:-https://localhost:${ADMIN_PORT:-9443}}"
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
selection=$(for t in $tests; do printf -- '--test %s ' "$t"; done)
# shellcheck disable=SC2086
cargo test --locked --no-fail-fast -p display-tests $selection -- --ignored --test-threads=1
