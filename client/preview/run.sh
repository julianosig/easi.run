#!/bin/sh
# Runs the admin page's design preview (client/preview: the page on example data, with hot reload) for a checkout, in
# the container easi-run-preview, published on the host's loopback only (127.0.0.1:5181):
#   client/preview/run.sh                    this checkout
#   client/preview/run.sh /path/to/worktree  another checkout (a worktree), in place of the one running now
# The checkout's client/ is mounted, its preview/ with it, so its edits reload at once. Its node_modules are installed
# on first use.
# Reach it over SSH with `ssh -N -L 5181:127.0.0.1:5181 <server>`, then open http://localhost:5181/__preview/.
set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
root=$(cd "${1:-$here}" && pwd)
[ -f "$root/client/preview/vite.config.ts" ] || { echo "preview: $root has no client/preview/" >&2; exit 1; }
# The same Node as the image builds (Dockerfile, client stage).
image=docker.io/library/node:26@sha256:32fa97f3363975684b08bf4e8a68a47c7905175cc20275b50b974bbd02aba731
docker rm -f easi-run-preview >/dev/null 2>&1 || true
docker run -d --name easi-run-preview --restart unless-stopped --user "$(id -u):$(id -g)" -e HOME=/tmp \
    -v "$root/client:/client" -w /client -p 127.0.0.1:5181:5181 "$image" \
    sh -c '[ -d node_modules ] || npm ci --no-audit --no-fund; exec node_modules/.bin/vite --configLoader runner --config preview/vite.config.ts --host 0.0.0.0' \
    >/dev/null
echo "preview: $root/client on http://127.0.0.1:5181/__preview/ (docker logs -f easi-run-preview)"
