# syntax=docker/dockerfile:1
# Build from the repository root: docker build -f proxy/Dockerfile .

FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p proxy \
    && install -D -m 0555 target/release/proxy /out/proxy

# No shell, no package manager: glibc and libgcc only.
FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3
COPY --from=build /out/proxy /proxy
USER 10003:10003
EXPOSE 3128
ENTRYPOINT ["/proxy"]
