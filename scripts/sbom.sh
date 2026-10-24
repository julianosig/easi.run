#!/bin/sh
# Regenerates docs/sbom/*.spdx.json from the locally built images, then docs/licenses.md from them. Build the images
# first, with the bench profile for the baseline and the fixture host:
#   DOCKER_GID=$(stat -c %g /var/run/docker.sock) docker compose -f compose.yaml -f compose.bench.yaml build
set -eu
cd "$(dirname "$0")/.."
SYFT=docker.io/anchore/syft@sha256:500e2d872ac019436926e8322b4fc1f39441d94d21f6f4046c6ff29b30e8cb02 # 1.52.0
mkdir -p docs/sbom
# syft reads each image from a saved archive, with no network and without the Docker socket (root on this host); the
# source name and version keep each SBOM's root package named as the image.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for image in display browser agent proxy admin controller baseline fixtures; do
    docker image save "easi-run/$image:dev" -o "$tmp/$image.tar"
    docker run --rm --network none -v "$tmp:/in:ro" "$SYFT" -q --source-name "easi-run/$image" --source-version dev \
        "docker-archive:/in/$image.tar" -o spdx-json > "docs/sbom/$image.spdx.json.tmp"
    rm "$tmp/$image.tar"
    mv "docs/sbom/$image.spdx.json.tmp" "docs/sbom/$image.spdx.json"
    echo "docs/sbom/$image.spdx.json"
done
python3 scripts/licenses.py
