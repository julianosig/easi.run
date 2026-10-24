#!/bin/sh
# Runs a command in the development image, with this checkout mounted at the same path and run as you, e.g.
#   scripts/dev/run.sh scripts/check.sh
#   scripts/dev/run.sh cargo test -p display
# Its build output goes to target/dev (the host's own builds keep target/), and the Cargo registry and npm cache
# persist in the easi-run-dev-cache volume.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
image=easi-run/dev:local
docker build -q -t "$image" -f "$root/scripts/dev/Dockerfile" "$root" >/dev/null
tty=""
if [ -t 0 ]; then tty=-t; fi
# $tty stays unquoted: it is one flag or nothing, and an empty quoted word would reach docker as an argument.
# shellcheck disable=SC2086
exec docker run --rm -i $tty --user "$(id -u):$(id -g)" \
    -v "$root:$root" -w "$root" \
    -v easi-run-dev-cache:/cache -e CARGO_HOME=/cache/cargo -e npm_config_cache=/cache/npm -e HOME=/cache/home \
    -e CARGO_TARGET_DIR="$root/target/dev" -e RUSTUP_HOME=/usr/local/rustup \
    "$image" "$@"
