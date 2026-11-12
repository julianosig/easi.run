# syntax=docker/dockerfile:1
# The environment the checks run in, locally (scripts/dev/run.sh) and in CI: the Rust toolchain the images build with,
# Node 26, Python 3, the display's build dependencies, SVT-AV1 and libopus built once into /opt, and the scripts' linters
# (ShellCheck and Ruff).
FROM docker.io/library/node:26@sha256:e18c3ccdf7d71619990b0e85639f506e2028a95410999cab24cc373cbd10e0ac AS node

FROM docker.io/library/rust:1.99.1-trixie@sha256:8a4e90c7946b74626488b1446f712c0fe0ed1a596cbd3d871684bc9c5433ae07
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends nasm cmake \
        libxkbcommon-dev python3 shellcheck \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add rustfmt clippy
# Ruff, from its release archive by checksum (Debian does not package it).
ARG RUFF_VERSION=0.17.4
ARG RUFF_SHA256=3ab2b34aeacdbc1ea5c529cdbda3e05792633d587228123fd09f3b5a0facb27a
RUN curl -fsSL --retry 5 --retry-all-errors -o /tmp/ruff.tar.gz \
        "https://github.com/astral-sh/ruff/releases/download/${RUFF_VERSION}/ruff-x86_64-unknown-linux-gnu.tar.gz" \
    && echo "${RUFF_SHA256}  /tmp/ruff.tar.gz" | sha256sum -c - \
    && tar -xzf /tmp/ruff.tar.gz -C /usr/local/bin --strip-components=1 ruff-x86_64-unknown-linux-gnu/ruff \
    && rm /tmp/ruff.tar.gz
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx
COPY scripts/static-library.sh scripts/svt-av1.sh /usr/local/bin/
RUN svt-av1.sh /opt/svt-av1
ENV SVTAV1_INCLUDE_DIR=/opt/svt-av1/include SVTAV1_LIB_DIR=/opt/svt-av1/lib
COPY scripts/opus.sh /usr/local/bin/opus.sh
RUN opus.sh /opt/opus
ENV OPUS_LIB_DIR=/opt/opus/lib
# Caches for scripts/dev/run.sh, writable by any user: a new named volume mounted here starts as this directory.
RUN mkdir -p /cache/cargo /cache/npm /cache/home && chmod -R 1777 /cache
