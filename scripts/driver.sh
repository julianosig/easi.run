#!/bin/sh
# Builds the agent's driver alone (the root Dockerfile's `driver` target: the Cua Driver from its source, with
# the patches in scripts/patches) as the image the agent image takes it from, so that it is compiled once for each
# recipe (its version, its commit, the patches), not by every build of the agent image:
#   scripts/driver.sh           builds it and says its name
#   scripts/driver.sh --push    also pushes it, and pins it in the Dockerfile (CUA_DRIVER) by its digest
# A push needs `docker login ghcr.io` as an account that may write the package. Its tag is the driver's version, its
# commit's first twelve hex digits and the patch's; the digest pinned is the one that push wrote. Commit the
# Dockerfile's change once the image reads without credentials (a new package on GHCR is private: CONTRIBUTING.md,
# Releases).
set -eu
# Every cd below is to this checkout's own directories, whatever CDPATH says.
unset CDPATH
cd "$(dirname "$0")/.."
REGISTRY=${REGISTRY:-ghcr.io/julianosig/easi.run}
case "${1:-}" in
    "" | --push) ;;
    *) echo "usage: scripts/driver.sh [--push]" >&2; exit 2 ;;
esac
version=$(sed -n 's/^ARG CUA_DRIVER_VERSION=//p' Dockerfile)
commit=$(sed -n 's/^ARG CUA_DRIVER_COMMIT=//p' Dockerfile)
# The patches' names and digests, as the Dockerfile's recipe records them (a patch that cannot be read stops this).
manifest=$(cd ./scripts/patches && LC_ALL=C sha256sum cua-driver-*.patch)
digest=$(printf '%s\n' "$manifest" | sha256sum)
patch=$(printf %s "$digest" | cut -c1-12)
if [ -z "$version" ] || [ -z "$commit" ]; then
    echo "Dockerfile names no driver version and commit" >&2
    exit 1
fi
image="$REGISTRY-driver:$version-$(printf %s "$commit" | cut -c1-12)-$patch"
# The engine's own builder, so the image is in its store to push and to build the agent image from.
docker buildx build --builder default --target driver --build-arg CUA_DRIVER=driver --tag "$image" \
    --label "org.opencontainers.image.source=https://github.com/julianosig/easi.run" \
    --label "org.opencontainers.image.title=easi.run-driver" \
    --label "org.opencontainers.image.licenses=MIT" \
    --label "org.opencontainers.image.description=Cua Driver $version ($commit) with easi.run's patches" .
if [ "${1:-}" != --push ]; then
    echo "built $image; try the agent image on it: --build-arg CUA_DRIVER=$image"
    exit 0
fi
# The digest this push wrote, as the registry acknowledged it (not what the tag names later, which another push could
# have moved).
digest=$(docker push "$image" | sed -n 's/^.*digest: \(sha256:[0-9a-f]\{64\}\).*$/\1/p' | tail -n 1)
if ! printf '%s\n' "$digest" | grep -Eqx 'sha256:[0-9a-f]{64}'; then
    echo "the registry names no digest for $image" >&2
    exit 1
fi
pinned="$image@$digest"
# Written whole beside the file and moved over it: an interrupted run leaves the Dockerfile as it was.
sed "s|^ARG CUA_DRIVER=.*$|ARG CUA_DRIVER=$pinned|" Dockerfile > Dockerfile.pinning
grep -Fqx "ARG CUA_DRIVER=$pinned" Dockerfile.pinning || { rm Dockerfile.pinning; echo "Dockerfile has no CUA_DRIVER" >&2; exit 1; }
mv Dockerfile.pinning Dockerfile
echo "pinned CUA_DRIVER=$pinned"
echo "next: check that it reads without credentials (docker logout ghcr.io; docker buildx imagetools inspect $pinned),"
echo "      then build the agent image and commit the Dockerfile"
