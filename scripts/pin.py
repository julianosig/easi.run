#!/usr/bin/env python3
"""Pins install.sh to a release: a commit of the public repository and the images CI published from it.

    scripts/pin.py COMMIT

COMMIT is the whole id of the public commit whose CI run passed, which published ghcr.io/julianosig/easi.run-<role>:
COMMIT for every role install.sh pulls. For each, this reads the registry as anyone would, without credentials (so a
package that is not public yet fails here, as it would for every installation): the image's manifest for that tag,
checked against the digest it names, and its configuration, which must be for Linux on x86-64 and carry the labels CI
sets (the repository, the commit, the package). Only then does it write PINNED_COMMIT and PINNED_IMAGES in install.sh,
both at once; nothing is written when any image fails.
"""

import hashlib
import json
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INSTALLER = ROOT / "install.sh"
REGISTRY = "ghcr.io"
NAMESPACE = "julianosig/easi.run"
SOURCE = "https://github.com/julianosig/easi.run"
# The single-image manifests CI pushes (no index: one platform, no attestations).
MANIFESTS = ("application/vnd.oci.image.manifest.v1+json", "application/vnd.docker.distribution.manifest.v2+json")
COMMIT_LINE = re.compile(r"^PINNED_COMMIT=[0-9a-f]*$", re.M)
IMAGES_BLOCK = re.compile(r"^PINNED_IMAGES=\([^)]*\)$", re.M)
# install.sh's roles, in the order its PINNED_IMAGES names them.
ROLES_LINE = re.compile(r"^ROLES=\(([a-z ]+)\)$", re.M)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


# A blob is served by a redirect to storage elsewhere, which must not receive the registry's token.
OPENER = urllib.request.build_opener(NoRedirect)


def fetch(url, token=None, accept=None):
    """The body and headers of `url`, following a redirect once without the token."""
    headers = {"Authorization": f"Bearer {token}"} if token else {}
    if accept:
        headers["Accept"] = accept
    try:
        with OPENER.open(urllib.request.Request(url, headers=headers), timeout=60) as answer:
            return answer.read(), answer.headers
    except urllib.error.HTTPError as error:
        if error.code in (301, 302, 303, 307, 308) and error.headers.get("Location"):
            with urllib.request.urlopen(urllib.parse.urljoin(url, error.headers["Location"]), timeout=300) as answer:
                return answer.read(), answer.headers
        raise


def sha256(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def image(role, commit):
    """`role@sha256:<digest>` of the image tagged `commit`, checked as the docstring says."""
    repository = f"{NAMESPACE}-{role}"
    query = urllib.parse.urlencode({"scope": f"repository:{repository}:pull", "service": REGISTRY})
    try:
        token = json.loads(fetch(f"https://{REGISTRY}/token?{query}")[0])["token"]
        body, headers = fetch(f"https://{REGISTRY}/v2/{repository}/manifests/{commit}", token, ", ".join(MANIFESTS))
    except urllib.error.HTTPError as error:
        sys.exit(f"{REGISTRY}/{repository}:{commit}: {error.code} {error.reason} (did CI publish it? is it public?)")
    kind = headers.get("Content-Type", "").split(";")[0]
    if kind not in MANIFESTS:
        sys.exit(f"{REGISTRY}/{repository}:{commit} is a {kind}, not a single image's manifest")
    digest = sha256(body)
    if headers.get("Docker-Content-Digest") != digest:
        sys.exit(f"{REGISTRY}/{repository}:{commit}: the manifest is not the digest the registry names")
    config_digest = json.loads(body)["config"]["digest"]
    config = fetch(f"https://{REGISTRY}/v2/{repository}/blobs/{config_digest}", token)[0]
    if sha256(config) != config_digest:
        sys.exit(f"{REGISTRY}/{repository}:{commit}: its configuration is not the digest its manifest names")
    config = json.loads(config)
    labels = (config.get("config") or {}).get("Labels") or {}
    expected = {"org.opencontainers.image.source": SOURCE, "org.opencontainers.image.revision": commit,
                "org.opencontainers.image.title": f"easi.run-{role}"}
    wrong = {k: labels.get(k) for k, v in expected.items() if labels.get(k) != v}
    if (config.get("os"), config.get("architecture")) != ("linux", "amd64") or wrong:
        sys.exit(f"{REGISTRY}/{repository}:{commit} is not CI's image of {commit} for Linux on x86-64: "
                 f"{config.get('os')}/{config.get('architecture')}, {wrong}")
    return f"{role}@{digest}"


def main():
    if len(sys.argv) != 2 or not re.fullmatch(r"[0-9a-f]{40}", sys.argv[1]):
        sys.exit("usage: scripts/pin.py COMMIT (the public commit's whole id)")
    commit = sys.argv[1]
    text = INSTALLER.read_text()
    roles = ROLES_LINE.findall(text)
    if len(COMMIT_LINE.findall(text)) != 1 or len(IMAGES_BLOCK.findall(text)) != 1 or len(roles) != 1:
        sys.exit("install.sh has not one PINNED_COMMIT line, one PINNED_IMAGES array and one ROLES array")
    images = [image(role, commit) for role in roles[0].split()]
    text = COMMIT_LINE.sub(f"PINNED_COMMIT={commit}", text)
    text = IMAGES_BLOCK.sub("PINNED_IMAGES=(\n" + "".join(f"  {i}\n" for i in images) + ")", text)
    INSTALLER.write_text(text)
    print(f"install.sh pins {commit}:", *images, sep="\n  ")


if __name__ == "__main__":
    main()
