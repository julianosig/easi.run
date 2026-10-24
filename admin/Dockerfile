# syntax=docker/dockerfile:1
# The admin web application and its client. Build from the repository root:
#   docker build -f admin/Dockerfile -t easi-run/admin:dev .
FROM docker.io/library/node:26@sha256:32fa97f3363975684b08bf4e8a68a47c7905175cc20275b50b974bbd02aba731 AS client
WORKDIR /src/client
COPY client/package.json client/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY client/ ./
COPY protocol/vectors /src/protocol/vectors
# The bench profile sets CLIENT_BENCH=1 to add the benchmark page; the runtime image has none.
ARG CLIENT_BENCH=0
# The SDK the build put in dist/sdk/ moves out of the page's files: it is served once, at /sdk/ (not also under the
# page's own path, such as /demo/sdk/).
RUN npm test && if [ "$CLIENT_BENCH" = 1 ]; then npm run build:bench; else npm run build; fi && mv dist/sdk /src/sdk

FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked --mount=type=cache,target=/src/target \
    cargo build --locked --release -p admin && install -m 0555 target/release/admin /admin \
    && install -d -m 0700 /state

FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3
COPY --from=build /admin /usr/local/bin/admin
COPY --from=client /src/client/dist /srv/admin
# The viewer SDK, served at /sdk/ even when ADMIN_STATIC points at a custom client.
COPY --from=client /src/sdk /srv/sdk
# An empty named volume mounted here takes this owner and mode on first use.
COPY --from=build --chown=10004:10014 /state /var/lib/admin
USER 10004:10014
ENTRYPOINT ["/usr/local/bin/admin"]
