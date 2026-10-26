# syntax=docker/dockerfile:1
# The benchmark fixture host (bench profile only). Build from the repository root:
#   docker build -f bench/Dockerfile -t easi-run/fixtures:dev .
FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS build
WORKDIR /src
# Only what cargo reads, as the root Dockerfile's build stage copies it, so a change to the client, a test, or a
# fixture leaves this stage cached.
COPY --parents Cargo.toml Cargo.lock admin/Cargo.toml admin/src affinity/Cargo.toml affinity/src agent/Cargo.toml \
    agent/src bench/Cargo.toml bench/src browser/launch/Cargo.toml browser/launch/build.rs browser/launch/src \
    controller/Cargo.toml controller/src display/Cargo.toml display/build.rs display/src egress/Cargo.toml egress/src \
    line/Cargo.toml line/src locale/Cargo.toml locale/src protocol/Cargo.toml protocol/src protocol/wayland \
    proxy/Cargo.toml proxy/src tests/Cargo.toml tests/src third_party/wayland-backend \
    third_party/wtransport ./
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked --mount=type=cache,target=/src/target \
    cargo build --locked --release -p bench && install -m 0555 target/release/fixture-server /fixture-server

FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS video
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ffmpeg \
    && ffmpeg -v error -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 20 -c:v libvpx-vp9 -b:v 4M \
        -deadline realtime -cpu-used 8 -row-mt 1 /video-1080p.webm

FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3
COPY --from=build /fixture-server /usr/local/bin/fixture-server
COPY bench/fixtures /srv/fixtures
COPY --from=video /video-1080p.webm /srv/fixtures/video-1080p.webm
ENTRYPOINT ["/usr/local/bin/fixture-server"]
