#!/bin/bash
# What an installation holds outlasts an update: starts the release install.sh pins (its published images, its Compose
# file, Google Chrome added to its browser image as install.sh adds it) as an installation of its own, lets
# tests/tests/update.rs make there what an installation holds, replaces it by this checkout's Compose file and images
# with install.sh --update's own commands, in its order (the admin stopped with its two minutes, everything brought up,
# then the controller and the admin started anew, as when a session image changed), and lets the test find it all
# again. It needs the bench stack's fixture host running and its images built (scripts/e2e.sh does both, and runs this
# beside its exclusive tests): the images under test are the bench stack's, easi-run-bench/*:dev.
#
# What it proves is that the checkout reads and keeps what the release before it wrote. What install.sh itself decides
# (whether to stop the admin first, which images to tag, which containers to start anew) is not run here:
# scripts/install-test.sh has that.
#
# The installation is the Compose project easi-run-update, with a controller instance, volumes and a port
# (127.0.0.1:9445) of its own, on the bench stack's fixture network: it stops, recreates and removes nothing of
# another installation, and is removed whole when the test ends.
set -euo pipefail
cd "$(dirname "$0")/.."
# One run at a time on this host (scripts/e2e.sh's lock, which it already holds when it runs this).
if [ "${E2E_LOCK:-}" != held ]; then
    lock=/tmp/easi-run-bench-e2e.lock
    [ -e "$lock" ] || (umask 022 && : >>"$lock")
    exec 9<"$lock"
    flock -n 9 || { echo "scripts/e2e.sh or another scripts/update-test.sh runs against the bench stack" >&2; exit 1; }
fi
PORT=9445
work=target/update
mkdir -p "$work"

# The release before this one, as install.sh names it: its commit, and each image by its digest.
commit=$(sed -n 's/^PINNED_COMMIT=\([0-9a-f]\{40\}\)$/\1/p' install.sh)
registry=$(sed -n 's/^REGISTRY=//p' install.sh)
mapfile -t pinned < <(sed -n '/^PINNED_IMAGES=(/,/^)/s/^ *\([a-z]*@sha256:[0-9a-f]\{64\}\)$/\1/p' install.sh)
if [ -z "$commit" ] || [ "${#pinned[@]}" != 6 ]; then
    echo "install.sh pins no release to update from" >&2
    exit 1
fi
# Its Compose file and Dockerfile, from the public repository when this clone does not have the commit.
git cat-file -e "$commit^{commit}" 2>/dev/null || git fetch --quiet https://github.com/julianosig/easi.run.git "$commit"
git show "$commit:compose.yaml" >"$work/compose.old.yaml"
git show "$commit:Dockerfile" >"$work/Dockerfile.old"
for image in "${pinned[@]}"; do
    role=${image%%@*}
    # A digest names one content: what this host already has of it is asked of no registry again.
    docker image inspect "$registry-$image" >/dev/null 2>&1 || docker pull --quiet "$registry-$image" >/dev/null
    docker tag "$registry-$image" "easi-run-update/$role:old"
done
# Google Chrome on the published browser image, as install.sh's `chrome` adds it, once for each release: an
# installation keeps the Chrome it was installed with, and the release's Dockerfile refuses a later Chrome whose
# package asks for what its browser image does not have.
# UPDATE_BROWSER_IMAGE names the browser image of an installation of that release instead (its easi-run/browser:dev,
# brought here with docker save and load): what to use once Google's Chrome no longer fits the release.
browser=${UPDATE_BROWSER_IMAGE:-easi-run-update/browser:chrome-${commit:0:12}}
if ! docker image inspect "$browser" >/dev/null 2>&1; then
    if [ -n "${UPDATE_BROWSER_IMAGE:-}" ]; then
        echo "update-test: UPDATE_BROWSER_IMAGE names $browser, which this host does not have" >&2
        exit 1
    fi
    if ! BUILDX_NO_DEFAULT_ATTESTATIONS=1 docker buildx build --quiet --builder default --target browser \
        --build-arg "BROWSER_BASE=easi-run-update/browser:old" --tag "$browser" - <"$work/Dockerfile.old" >/dev/null; then
        # Google serves only its current Chrome. Once that one asks for more than the release's browser image has,
        # the release no longer installs anywhere, and only an installation of it still has the browser it runs.
        echo "update-test: Google Chrome could not be added to release ${commit:0:12}'s browser image (above). If" \
            "the download failed, run this again. If today's Chrome no longer fits that release, name the browser" \
            "image of an installation of it in UPDATE_BROWSER_IMAGE: nothing else here can stand in for it" >&2
        exit 3
    fi
fi
# The image chats run in, built from the release's own runner/ as install.sh's `chat_image` builds it, once for each
# release.
runner=easi-run-update/runner:${commit:0:12}
docker image inspect "$runner" >/dev/null 2>&1 ||
    git archive "$commit:runner" | BUILDX_NO_DEFAULT_ATTESTATIONS=1 docker buildx build --quiet --builder default \
        --tag "$runner" - >/dev/null
for role in admin controller display browser agent proxy runner; do
    docker image inspect "easi-run-bench/$role:dev" >/dev/null 2>&1 ||
        { echo "easi-run-bench/$role:dev is not built: scripts/e2e.sh builds the bench stack's images" >&2; exit 1; }
done
# The fixture host its sessions browse, and the upstream the test's egress leaves through.
docker compose -p easi-run-bench -f compose.yaml -f compose.bench.yaml up -d --no-build --wait fixtures upstream \
    >/dev/null
# A viewer of this checkout's speaks one version of the display's protocol: the release before it must speak it too
# for the part of the test where a person types before the update, which is left out, and said so, otherwise.
wire() { sed -n 's/^pub const VERSION: u16 = \([0-9][0-9]*\);.*$/\1/p'; }
before=$(git show "$commit:protocol/src/lib.rs" 2>/dev/null | wire || true)
now=$(wire <protocol/src/lib.rs)
viewer=1
# One version read of each, and the same: anything else (a declaration this cannot read, too) leaves the part out.
if [ -z "$before" ] || [ -z "$now" ] || [ "$before" != "$now" ]; then
    viewer=0
    echo "update-test: release ${commit:0:12} speaks version '${before:-unread}' of the display's protocol and this" \
        "checkout '${now:-unread}': the part where a person types before the update is left out"
fi

# What makes it an installation of its own, over either release's compose.yaml: $1 and $2 name the admin's and the
# controller's images, the rest each session image, the chats' last.
override() {
    cat <<EOF
name: easi-run-update
services:
  admin: { image: "$1" }
  init: { image: "$1" }
  controller:
    image: "$2"
    environment:
      CONTROLLER_INSTANCE: easirun-update
      IMAGE_DISPLAY: "$3"
      IMAGE_BROWSER: "$4"
      IMAGE_AGENT: "$5"
      IMAGE_PROXY: "$6"
      IMAGE_RUNNER: "$7"
      STREAMS_VOLUME: easi-run-update-streams
      CONTROLLER_BENCH_NETWORK: easi-run-bench
      CONTROLLER_BENCH_FIXTURE: fixture.bench=10.250.250.10
      CONTROLLER_BENCH_SPKI_FILE: /run/bench/spki
    volumes:
      - bench:/run/bench:ro
  display-image: { image: "$3" }
  browser-image: { image: "$4" }
  agent-image: { image: "$5" }
  proxy-image: { image: "$6" }
  runner-image: { image: "$7" }
volumes:
  streams: { name: easi-run-update-streams }
  bench: { external: true, name: easi-run-bench_bench }
EOF
}
old=easi-run-update
override "$old/admin:old" "$old/controller:old" "$old/display:old" "$browser" "$old/agent:old" \
    "$old/proxy:old" "$runner" >"$work/override.old.yaml"
new=easi-run-bench
override "$new/admin:dev" "$new/controller:dev" "$new/display:dev" "$new/browser:dev" "$new/agent:dev" \
    "$new/proxy:dev" "$new/runner:dev" >"$work/override.new.yaml"
# What Compose is given, and nothing else of the caller's environment (scripts/e2e.sh's ADMIN_ORIGIN is the bench
# stack's) nor of an .env here: where Docker is, and this installation's group, port and session limit.
settings=("PATH=$PATH" "HOME=$HOME" "DOCKER_GID=$(stat -c %g /var/run/docker.sock)" "ADMIN_PORT=$PORT"
    ADMIN_MAX_SESSIONS=6)
for name in DOCKER_HOST DOCKER_CONFIG DOCKER_CONTEXT; do
    [ -z "${!name:-}" ] || settings+=("$name=${!name}")
done
compose() {
    local release=$1
    shift
    local file=compose.yaml
    [ "$release" = new ] || file="$work/compose.old.yaml"
    env -i "${settings[@]}" docker compose --project-directory . --env-file /dev/null -p easi-run-update \
        -f "$file" -f "$work/override.$release.yaml" "$@"
}
# Its containers and volumes, an earlier run's that was interrupted too, and whatever its controller made for a
# session and an admin that died did not take down (they carry the controller's instance, not the project): no other
# installation's.
remove() {
    compose new down --volumes --timeout 130 >/dev/null 2>&1 || true
    local kind listed all
    for kind in container network volume; do
        all=()
        [ "$kind" != container ] || all=(--all)
        listed=$(docker "$kind" ls "${all[@]}" --quiet --filter label=run.easi.instance=easirun-update 2>/dev/null || true)
        # shellcheck disable=SC2086 # One word for each.
        [ -z "$listed" ] || docker "$kind" rm --force $listed >/dev/null 2>&1 || true
    done
    docker volume rm easi-run-update-streams >/dev/null 2>&1 || true
}
remove
trap remove EXIT

# A password of its own, which no other installation has, set before the admin starts.
password=$(head -c 18 /dev/urandom | base64 | tr -d '/+=')
printf '%s\n' "$password" | compose old run --rm -T --no-deps init >/dev/null
compose old up -d --no-build --pull never >/dev/null
answers() { curl --silent --insecure --fail --max-time 5 --output /dev/null "https://localhost:$PORT/api/status"; }
for _ in $(seq 120); do
    answers && break
    sleep 1
done
answers || { echo "the release before this one never answered on port $PORT" >&2; compose old logs --tail 40 admin >&2; exit 1; }

# The update, which the test runs once it has made what must outlast it: install.sh's `save_sessions`, its `up`, and
# the start anew it makes of the controller and the admin when a session image changed, as every one did here. The
# admin so starts twice, the first time stopped before anyone gave it the password: what it keeps for the sessions that
# ran must outlast that too.
{
    echo '#!/bin/bash'
    echo 'set -euo pipefail'
    printf 'cd %q\n' "$PWD"
    printf 'compose() { env -i'
    printf ' %q' "${settings[@]}"
    echo ' docker compose --project-directory . --env-file /dev/null -p easi-run-update "$@"; }'
    printf 'compose -f %q -f %q stop --timeout 120 admin\n' "$work/compose.old.yaml" "$work/override.old.yaml"
    printf 'compose -f compose.yaml -f %q up -d --no-build --pull never\n' "$work/override.new.yaml"
    printf 'compose -f compose.yaml -f %q up -d --no-build --pull never --no-deps --force-recreate controller admin\n' \
        "$work/override.new.yaml"
} >"$work/update.sh"
chmod +x "$work/update.sh"
echo "update-test: from release ${commit:0:12} to this checkout"
ADMIN_ORIGIN="https://localhost:$PORT" ADMIN_PASSWORD="$password" UPDATE_COMMAND="$PWD/$work/update.sh" \
    UPDATE_VIEWER=$viewer UPDATE_INSTANCE=easirun-update cargo test --locked -p display-tests --test update -- --include-ignored --test-threads=1 --nocapture
