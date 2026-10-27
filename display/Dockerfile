# syntax=docker/dockerfile:1
# The easi.run compositor and encoder. Build from the repository root:
#   docker build -f display/Dockerfile -t easi-run/display:dev .
FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS toolchain
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends nasm cmake libxkbcommon-dev \
    && rm -rf /var/lib/apt/lists/*
# SVT-AV1, built from its checksummed source archive into a static library: its real-time mode encodes a
# 1280x800 frame in about 13 ms on one core, where Debian's 2.3.0 needs about 21.
COPY scripts/svt-av1.sh /usr/local/bin/svt-av1.sh
RUN svt-av1.sh /opt/svt-av1
ENV SVTAV1_INCLUDE_DIR=/opt/svt-av1/include SVTAV1_LIB_DIR=/opt/svt-av1/lib
# libopus for the browser's sound, likewise from its checksummed release archive.
COPY scripts/opus.sh /usr/local/bin/opus.sh
RUN opus.sh /opt/opus
ENV OPUS_LIB_DIR=/opt/opus/lib
WORKDIR /src

# The workspace as cargo reads it, as the root Dockerfile's `workspace` stage copies it, so a change to the client, a
# test, or a fixture leaves the stages below cached.
FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS workspace
WORKDIR /src
COPY --parents Cargo.toml Cargo.lock admin/Cargo.toml admin/src affinity/Cargo.toml affinity/src agent/Cargo.toml \
    agent/src bench/Cargo.toml bench/src browser/launch/Cargo.toml browser/launch/build.rs browser/launch/src \
    controller/Cargo.toml controller/src display/Cargo.toml display/build.rs display/src egress/Cargo.toml egress/src \
    line/Cargo.toml line/src locale/Cargo.toml locale/src protocol/Cargo.toml protocol/src protocol/wayland \
    proxy/Cargo.toml proxy/src tests/Cargo.toml tests/src third_party/wayland-backend \
    third_party/wtransport ./

# Its dependency skeleton (scripts/skeleton.sh), which only a manifest or the lock file changes.
FROM workspace AS skeleton
COPY scripts/skeleton.sh /skeleton.sh
RUN /skeleton.sh /src

# The display's dependencies, compiled from the skeleton into this stage's own layer, as the root Dockerfile's `deps`
# stage does for the other programs.
FROM toolchain AS deps
COPY --from=skeleton /src ./
# Cargo features: empty for the published image; `h264` adds the patent-encumbered H.264 encoder (see display/Cargo.toml).
ARG DISPLAY_FEATURES=
RUN --mount=type=cache,id=cargo-registry-display,target=/usr/local/cargo/registry,sharing=locked \
    cargo build --locked --release -p display --features "$DISPLAY_FEATURES" \
    && cargo clean --release $(cat .members) && mv target /deps

# The display. Its target directory is a cache mount, so a local rebuild compiles only what changed; an empty one (a CI
# runner's) starts from the dependencies above.
FROM deps AS build
COPY --from=workspace /src ./
ARG DISPLAY_FEATURES=
RUN --mount=type=cache,id=cargo-registry-display,target=/usr/local/cargo/registry,sharing=locked --mount=type=cache,id=cargo-target-display,target=/src/target \
    { [ -e target/release ] || cp -a /deps/. target/; } \
    && cargo build --locked --release -p display --features "$DISPLAY_FEATURES" \
    && install -m 0555 target/release/display /display
# What the image owes for what it ships, in its /usr/share/doc/easi.run, as the root Dockerfile's build stage writes it
# for the other images (scripts/notices.py), and the encoders' license, patent and notice texts.
COPY scripts/notices.py scripts/notices.py
COPY scripts/notices scripts/notices
COPY LICENSE LICENSE
ARG SOURCE_REVISION=
RUN --mount=type=cache,id=cargo-registry-display,target=/usr/local/cargo/registry,sharing=locked \
    cargo fetch --locked --target x86_64-unknown-linux-gnu \
    && python3 scripts/notices.py source /notices "$SOURCE_REVISION" \
    && python3 scripts/notices.py cargo /notices/rust-crates.md display --features "$DISPLAY_FEATURES" \
    && cp -R /opt/svt-av1/share/doc/svt-av1 /opt/opus/share/doc/opus /notices/

# The two runtime packages, with their dpkg records and copyright files, so the image's SBOM stays exact.
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS runtime-files
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends libxkbcommon0 xkb-data \
    && mkdir -p /out/usr/lib/x86_64-linux-gnu /out/usr/share/X11 /out/usr/share/doc /out/var/lib/dpkg/status.d \
    && cp -a /usr/lib/x86_64-linux-gnu/libxkbcommon.so.0* /out/usr/lib/x86_64-linux-gnu/ \
    && cp -a /usr/share/X11/xkb /out/usr/share/X11/ \
    && for p in libxkbcommon0 xkb-data; do \
         mkdir -p /out/usr/share/doc/$p && cp /usr/share/doc/$p/copyright /out/usr/share/doc/$p/ \
         && dpkg-query -s $p > /out/var/lib/dpkg/status.d/$p; done

# Distroless: glibc and libgcc, no shell, no package manager.
FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3
COPY --from=runtime-files /out/ /
COPY --from=build /display /usr/local/bin/display
COPY --from=build /notices/ /usr/share/doc/easi.run/
# Two heap arenas instead of one per thread, and buffers of 256 KiB or more always mapped on their own (glibc otherwise
# raises that threshold after the first freed frame and serves later 4 MiB frames from a fragmenting heap): measured
# 13-18 MiB less in full-screen motion, at the same CPU.
ENV MALLOC_ARENA_MAX=2 MALLOC_MMAP_THRESHOLD_=262144
USER 10002:10002
ENTRYPOINT ["/usr/local/bin/display"]
