# syntax=docker/dockerfile:1
# Measurement tools for tests and the benchmark (ffmpeg: decode checks, SSIM). Never part of a session.
#   docker build -t easi-run/tools:dev bench/tools
FROM docker.io/library/debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ffmpeg \
    && rm -rf /var/lib/apt/lists/*
USER 65534:65534
