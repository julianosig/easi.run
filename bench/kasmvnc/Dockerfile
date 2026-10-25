# syntax=docker/dockerfile:1
# The benchmark baseline (bench profile only, never a runtime image): the same Google Chrome as easi.run's browser
# (the same package, from the same stage, and its fonts and Mesa for WebGL) on KasmVNC 1.5.0's X server, with
# KasmVNC's own web client (its noVNC fork) at /vnc.html, over HTTPS with the image's snakeoil certificate (its user is
# in ssl-cert, as KasmVNC's setup asks). KasmVNC is GPL-2.0 and Google Chrome proprietary; this image is built locally
# for measurement and not distributed (see the README's license section).

# Google Chrome's current stable package, from Google's signed apt repository. Its key is trusted only as the one
# certificate with the fingerprint EB4C1BFD4F042F6DDDCCEC917721F63BD38B4796 (Google's Linux package signing key), and
# apt checks the repository's signed index against it, then the package's SHA-256 against the index. The index is
# fetched on every build, so a new Chrome stable runs this stage again, while an unchanged package keeps the stages
# that copy it cached. This stage is the same, byte for byte, in browser/Dockerfile and bench/kasmvnc/Dockerfile
# (scripts/pins.py checks it), so one build installs one Chrome in both.
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS chrome
ADD https://dl.google.com/linux/linux_signing_key.pub /tmp/google.pub
ADD https://dl.google.com/linux/chrome/deb/dists/stable/InRelease /tmp/InRelease
RUN set -eu; apt-get update; DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates gpg; \
    export GNUPGHOME=/tmp/gnupg; mkdir -m 0700 "$GNUPGHOME"; \
    keys="$(gpg --batch --show-keys --with-colons /tmp/google.pub | awk -F: '$1 == "pub" {p = 1} $1 == "fpr" && p {print $10; p = 0}')"; \
    [ "$keys" = EB4C1BFD4F042F6DDDCCEC917721F63BD38B4796 ]; \
    gpg --batch --dearmor </tmp/google.pub >/usr/share/keyrings/google-chrome.gpg; \
    echo "deb [arch=amd64 signed-by=/usr/share/keyrings/google-chrome.gpg] https://dl.google.com/linux/chrome/deb/ stable main" \
        >/etc/apt/sources.list.d/google-chrome.list; \
    apt-get update; mkdir /chrome; chown _apt /chrome; cd /chrome; apt-get download google-chrome-stable; \
    mv google-chrome-stable_*_amd64.deb google-chrome-stable.deb

FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
ADD --checksum=sha256:80b241de7dfe53bba2b7e1cc5ac8c5246d72271efa16be2d4f76607f30fab1c4 \
    https://github.com/kasmtech/KasmVNC/releases/download/v1.5.0/kasmvncserver_trixie_1.5.0_amd64.deb /tmp/kasmvnc.deb
COPY --from=chrome /chrome/google-chrome-stable.deb /tmp/google-chrome-stable.deb
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
