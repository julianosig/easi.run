#!/bin/sh
# Runs a command in the development image, with this checkout mounted at the same path and run as you, e.g.
#   scripts/dev/run.sh scripts/check.sh
#   scripts/dev/run.sh cargo test -p display
# Its build output goes to target/dev (the host's own builds keep target/), and the Cargo registry and npm cache
# persist in the easi-run-dev-cache volume.
set -eu
# The tests these variables enable drive the host's Docker, which the image has neither the CLI nor the socket of, and
# the variables are not passed in: the tests would skip themselves and pass. Refuse instead.
for gate in CONTROLLER_DOCKER_TESTS DISPLAY_E2E; do
    if env | grep -q "^$gate="; then
        echo "$gate needs the host's Docker: run that command on the host, not in the development image" >&2
        exit 2
    fi
done
root=$(cd "$(dirname "$0")/../.." && pwd)
image=easi-run/dev:local
docker build -q -t "$image" -f "$root/scripts/dev/Dockerfile" "$root" >/dev/null
tty=""
if [ -t 0 ]; then tty=-t; fi
# The hostile tests' clients keep up to a thousand descriptors in flight on Unix sockets, which the kernel caps per user
# by the sender's open-file limit; Docker's default soft limit of 1024 would fail them at random. The display under test
# sets its own limit, the one it runs with in a session, whatever this one is. $tty stays unquoted: it is one flag or
# nothing, and an empty quoted word would reach docker as an argument.
# shellcheck disable=SC2086
exec docker run --rm -i $tty --user "$(id -u):$(id -g)" --ulimit nofile=65536:65536 \
    -v "$root:$root" -w "$root" \
    -v easi-run-dev-cache:/cache -e CARGO_HOME=/cache/cargo -e npm_config_cache=/cache/npm -e HOME=/cache/home \
    -e CARGO_TARGET_DIR="$root/target/dev" -e RUSTUP_HOME=/usr/local/rustup \
    "$image" "$@"
