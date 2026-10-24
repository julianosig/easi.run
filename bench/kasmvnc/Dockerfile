# syntax=docker/dockerfile:1
# The benchmark baseline (bench profile only, never a runtime image): the same Google Chrome as easi.run's browser
# (the same package, by the same checksum, and its fonts and Mesa for WebGL) on KasmVNC 1.5.0's X server, with
# KasmVNC's own web client (its noVNC fork) at /vnc.html, over HTTPS with the image's snakeoil certificate (its user is
# in ssl-cert, as KasmVNC's setup asks). KasmVNC is GPL-2.0 and Google Chrome proprietary; this image is built locally
# for measurement and not distributed (see the README's license section).
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
ADD --checksum=sha256:80b241de7dfe53bba2b7e1cc5ac8c5246d72271efa16be2d4f76607f30fab1c4 \
    https://github.com/kasmtech/KasmVNC/releases/download/v1.5.0/kasmvncserver_trixie_1.5.0_amd64.deb /tmp/kasmvnc.deb
ADD --checksum=sha256:6c92cd22e90c6e3e255d782991543fa2091db8614e8536a971a7433a5c4d7ac8 \
    https://dl.google.com/linux/chrome/deb/pool/main/g/google-chrome-stable/google-chrome-stable_156.0.8078.47-1_amd64.deb \
    /tmp/google-chrome-stable.deb
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        /tmp/google-chrome-stable.deb libegl1 libegl-mesa0 libgles2 \
        fonts-dejavu-core fonts-liberation2 fonts-noto-core fonts-noto-color-emoji fonts-noto-cjk \
        ca-certificates /tmp/kasmvnc.deb \
    && rm -rf /var/lib/apt/lists/* /tmp/*.deb /etc/apt/sources.list.d/google-chrome.list /etc/cron.daily/google-chrome \
    && groupadd -g 10001 browser && useradd -u 10001 -g 10001 -G ssl-cert -M -d /home/browser -s /usr/sbin/nologin browser \
    && mkdir -p /home/browser /etc/opt/chrome/policies/managed && chown 10001:10001 /home/browser
COPY browser/policy.json /etc/opt/chrome/policies/managed/display.json
COPY --chmod=0755 bench/kasmvnc/launch.sh /usr/local/bin/baseline-launch
# By name, so Docker applies its supplementary ssl-cert group (a numeric USER gets none).
USER browser
EXPOSE 6901
ENTRYPOINT ["/usr/local/bin/baseline-launch"]
