# syntax=docker/dockerfile:1
# Measurement tools for tests and the benchmark (ffmpeg: decode checks, SSIM). Never part of a session.
#   docker build -t easi-run/tools:dev bench/tools
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ffmpeg \
    && rm -rf /var/lib/apt/lists/*
USER 65534:65534
