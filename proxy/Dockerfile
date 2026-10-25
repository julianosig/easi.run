# syntax=docker/dockerfile:1
# Build from the repository root: docker build -f proxy/Dockerfile .

# The runtime images' Rust programs, built by one cargo invocation: this stage is the same, byte for byte, in the five
# Dockerfiles that ship them (admin, agent, browser, controller, proxy; scripts/pins.py checks it), so one build runs it
# once, compiling each shared crate once, and each image copies its own programs from /out.
FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked --mount=type=cache,target=/src/target \
    cargo build --locked --release -p admin -p agent -p browser-launch -p controller -p proxy \
    && install -D -m 0555 -t /out target/release/admin target/release/agent-mcp target/release/agent-idle \
        target/release/browser-launch target/release/dpkg-prune target/release/controller target/release/proxy \
    && install -d -m 0700 /out/admin-state

# No shell, no package manager: glibc and libgcc only.
FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3
COPY --from=build /out/proxy /proxy
USER 10003:10003
EXPOSE 3128
ENTRYPOINT ["/proxy"]
