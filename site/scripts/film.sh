#!/bin/bash
# Encodes one language's film from its frames (scripts/film.mjs, <out>/frames/) as the landing plays it: AV1 WebM, which
# Chrome, Edge and Firefox play, then HEVC MP4 for Safari; a poster, the frame of the mark; and the language's README
# film: an animated AVIF, an image GitHub shows looping, where a video gets controls. deploy/film.sh publishes them.
#   site/scripts/film.sh <out> <locale>     e.g. site/scripts/film.sh /tmp/film-en en
set -euo pipefail
out=$1 locale=$2
ff() { docker run --rm --user "$(id -u):$(id -g)" -v "$out:$out" -w "$out" --entrypoint ffmpeg dp-ffmpeg:latest -hide_banner -loglevel error -y "$@"; }
ff -framerate 30 -i frames/%05d.jpg -c:v libsvtav1 -preset 5 -crf 38 -g 300 -pix_fmt yuv420p "$locale.webm"
ff -framerate 30 -i frames/%05d.jpg -c:v libx265 -preset slow -crf 28 -tag:v hvc1 -pix_fmt yuv420p -movflags +faststart -x265-params log-level=error "$locale.mp4"
ff -i frames/00322.jpg -vf scale=1280:-2 -q:v 3 "$locale.jpg"
ff -framerate 30 -i frames/%05d.jpg -vf "fps=24,scale=1280:-2:flags=lanczos" -c:v libsvtav1 -preset 6 -crf 40 -pix_fmt yuv420p -loop 0 "film.$locale.avif"
mv "$out/film.$locale.avif" "$out/$locale.avif"
ls -la "$out/$locale".*
