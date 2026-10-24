# syntax=docker/dockerfile:1
# Build from the repository root: docker build -f controller/Dockerfile .
# The browser seccomp profile is read at run time from CONTROLLER_BROWSER_SECCOMP
# (default /etc/controller/browser-seccomp.json).

FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p controller \
    && install -D -m 0555 target/release/controller /out/controller

# No shell, no package manager: glibc and libgcc only.
FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3
COPY --from=build /out/controller /controller
COPY browser/seccomp.json /etc/controller/browser-seccomp.json
# The Docker socket's group and the controller volume's group (10015) come from the service's group_add.
USER 10006:10006
ENTRYPOINT ["/controller"]
