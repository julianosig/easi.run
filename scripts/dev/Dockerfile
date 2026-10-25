# syntax=docker/dockerfile:1
# The environment the checks run in, locally (scripts/dev/run.sh) and in CI: the Rust toolchain the images build with,
# Node 26, Python 3, the display's build dependencies, SVT-AV1 and libopus built once into /opt, the scripts' linters
# (ShellCheck and Ruff), and cargo-llvm-cov, which scripts/coverage.sh measures the Rust tests with.
FROM docker.io/library/node:26@sha256:32fa97f3363975684b08bf4e8a68a47c7905175cc20275b50b974bbd02aba731 AS node

FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends nasm cmake \
        libxkbcommon-dev python3 shellcheck \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add rustfmt clippy
# Ruff, from its release archive by checksum (Debian does not package it).
ARG RUFF_VERSION=0.15.20
ARG RUFF_SHA256=df8e74862d4cd4fdac11faf3048789896ff9898a0cacb98497df20d0a1cc7bb4
RUN curl -fsSLo /tmp/ruff.tar.gz \
        "https://github.com/astral-sh/ruff/releases/download/${RUFF_VERSION}/ruff-x86_64-unknown-linux-gnu.tar.gz" \
    && echo "${RUFF_SHA256}  /tmp/ruff.tar.gz" | sha256sum -c - \
    && tar -xzf /tmp/ruff.tar.gz -C /usr/local/bin --strip-components=1 ruff-x86_64-unknown-linux-gnu/ruff \
    && rm /tmp/ruff.tar.gz
# cargo-llvm-cov the same way (its release lists each archive's SHA-256 digest), and the LLVM tools it runs.
ARG CARGO_LLVM_COV_VERSION=0.9.1
ARG CARGO_LLVM_COV_SHA256=b3f68e625481fed9b16444174f3fa5ebcdbde4a1878803a35eabe2dcefcdc41a
RUN curl -fsSLo /tmp/cargo-llvm-cov.tar.gz \
        "https://github.com/taiki-e/cargo-llvm-cov/releases/download/v${CARGO_LLVM_COV_VERSION}/cargo-llvm-cov-x86_64-unknown-linux-gnu.tar.gz" \
    && echo "${CARGO_LLVM_COV_SHA256}  /tmp/cargo-llvm-cov.tar.gz" | sha256sum -c - \
    && tar -xzf /tmp/cargo-llvm-cov.tar.gz -C /usr/local/bin cargo-llvm-cov \
    && rm /tmp/cargo-llvm-cov.tar.gz \
    && rustup component add llvm-tools-preview
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx
COPY scripts/svt-av1.sh /usr/local/bin/svt-av1.sh
RUN svt-av1.sh /opt/svt-av1
ENV SVTAV1_INCLUDE_DIR=/opt/svt-av1/include SVTAV1_LIB_DIR=/opt/svt-av1/lib
COPY scripts/opus.sh /usr/local/bin/opus.sh
RUN opus.sh /opt/opus
ENV OPUS_LIB_DIR=/opt/opus/lib
# Caches for scripts/dev/run.sh, writable by any user: a new named volume mounted here starts as this directory.
RUN mkdir -p /cache/cargo /cache/npm /cache/home && chmod -R 1777 /cache
