# syntax=docker/dockerfile:1
# The benchmark fixture host (bench profile only). Build from the repository root:
#   docker build -f bench/Dockerfile -t easi-run/fixtures:dev .
# The workspace as cargo reads it, as the root Dockerfile's `workspace` stage copies it, so a change to the client, a
# test, or a fixture leaves the stages below cached.
FROM docker.io/library/rust:1.99.1-trixie@sha256:8a4e90c7946b74626488b1446f712c0fe0ed1a596cbd3d871684bc9c5433ae07 AS workspace
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

# The fixture host's dependencies, compiled from the skeleton into this stage's own layer, as the root Dockerfile's
# `deps` stage does for the other programs.
FROM docker.io/library/rust:1.99.1-trixie@sha256:8a4e90c7946b74626488b1446f712c0fe0ed1a596cbd3d871684bc9c5433ae07 AS deps
WORKDIR /src
COPY --from=skeleton /src ./
RUN --mount=type=cache,id=cargo-registry-fixtures,target=/usr/local/cargo/registry,sharing=locked \
    cargo build --locked --release -p bench && cargo clean --release $(cat .members) && mv target /deps

# The fixture host. Its target directory is a cache mount, so a local rebuild compiles only what changed; an empty one
# (a CI runner's) starts from the dependencies above.
FROM deps AS build
COPY --from=workspace /src ./
RUN --mount=type=cache,id=cargo-registry-fixtures,target=/usr/local/cargo/registry,sharing=locked --mount=type=cache,id=cargo-target-fixtures,target=/src/target \
    { [ -e target/release ] || cp -a /deps/. target/; } \
    && cargo build --locked --release -p bench && install -m 0555 target/release/fixture-server /fixture-server

FROM docker.io/library/debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2 AS video
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ffmpeg \
    && ffmpeg -v error -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 20 -c:v libvpx-vp9 -b:v 4M \
        -deadline realtime -cpu-used 8 -row-mt 1 /video-1080p.webm

FROM gcr.io/distroless/cc-debian13@sha256:e1f5543d45a2cd7c02a5b43dda5f0ff30729b4832113759d01ed39b0b152cddb
COPY --from=build /fixture-server /usr/local/bin/fixture-server
COPY bench/fixtures /srv/fixtures
COPY --from=video /video-1080p.webm /srv/fixtures/video-1080p.webm
ENTRYPOINT ["/usr/local/bin/fixture-server"]
