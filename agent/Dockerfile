# syntax=docker/dockerfile:1
# The automation agent: the pinned Cua Driver, started only by the controller's exec of agent-mcp.
# Build from the repository root: docker build -f agent/Dockerfile -t easi-run/agent:dev .
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS cua
# The exact Cua Driver release, checked against its SHA-256 and never fetched at run time.
ADD --checksum=sha256:05dae2160c86bdf4855c279d30898213d7a150edb1992b7470afdd69f0df4064 \
    https://github.com/trycua/cua/releases/download/cua-driver-rs-v0.36.0/cua-driver-rs-0.36.0-linux-x86_64-binary.tar.gz \
    /tmp/cua.tar.gz
RUN mkdir /cua && tar -xzf /tmp/cua.tar.gz -C /cua cua-driver

FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target \
    cargo build --locked --release -p agent \
    && install -m 0555 target/release/agent-mcp target/release/agent-idle /

# The driver's X11 and keymap libraries (all MIT/X11 or BSD), with their dpkg records and copyright files.
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS runtime-files
ARG PACKAGES="libx11-6 libxi6 libxext6 libxcb1 libxau6 libxdmcp6 libbsd0 libmd0 libxkbcommon0 xkb-data libx11-data"
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends $PACKAGES \
    && mkdir -p /out/var/lib/dpkg/status.d \
    && for p in $PACKAGES; do \
         dpkg -L $p | while read f; do [ -f "$f" ] && ! echo "$f" | grep -q '^/usr/share/\(man\|lintian\)' \
           && mkdir -p "/out$(dirname "$f")" && cp -a "$f" "/out$f"; done; \
         dpkg -L $p | while read f; do [ -L "$f" ] && mkdir -p "/out$(dirname "$f")" && cp -a "$f" "/out$f"; done; \
         dpkg-query -s $p > /out/var/lib/dpkg/status.d/$p; done

# Distroless: glibc and libgcc, no shell, no package manager.
FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3
COPY --from=runtime-files /out/ /
COPY --from=cua --chmod=0111 /cua/cua-driver /usr/local/bin/cua-driver
COPY --from=build /agent-mcp /usr/local/bin/
# The main process is execute-only too, so the kernel marks it non-dumpable: in DevTools mode Chrome shares its uid and
# PID namespace, and could otherwise reach the agent's mount namespace through /proc/<agent-idle>/root.
COPY --from=build --chmod=0111 /agent-idle /usr/local/bin/agent-idle
USER 10005:10005
ENTRYPOINT ["/usr/local/bin/agent-idle"]
