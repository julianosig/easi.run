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
# A worktree's .git is a file naming the repository's own git directory: mount that too, at the same path, so git works
# inside. (In the repository itself it is the mounted checkout's own .git.)
common=$(git -C "$root" rev-parse --path-format=absolute --git-common-dir)
case "$common" in "$root"/*) common="" ;; esac
# $tty stays unquoted: it is one flag or nothing, and an empty quoted word would reach docker as an argument.
# shellcheck disable=SC2086
exec docker run --rm -i $tty --user "$(id -u):$(id -g)" \
    -v "$root:$root" ${common:+-v} ${common:+"$common:$common"} -w "$root" \
    -v easi-run-dev-cache:/cache -e CARGO_HOME=/cache/cargo -e npm_config_cache=/cache/npm -e HOME=/cache/home \
    -e CARGO_TARGET_DIR="$root/target/dev" -e RUSTUP_HOME=/usr/local/rustup \
    "$image" "$@"
