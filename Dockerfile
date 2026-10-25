# syntax=docker/dockerfile:1
# The admin, agent, browser, controller and proxy images, and the benchmark baseline: one target each, built from the
# repository root, e.g. docker build --target browser -t easi-run/browser:dev .
# They share two stages: `build`, one cargo invocation for every Rust program they ship, so one build compiles each
# shared crate once and each image copies its own programs from /out; and `chrome`, so the browser and the baseline
# install one Google Chrome. (The display and the fixture host have Dockerfiles of their own.)

# The runtime images' Rust programs.
FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked --mount=type=cache,target=/src/target \
    cargo build --locked --release -p admin -p agent -p browser-launch -p controller -p proxy \
    && install -D -m 0555 -t /out target/release/admin target/release/agent-mcp target/release/agent-idle \
        target/release/browser-launch target/release/dpkg-prune target/release/controller target/release/proxy \
    && install -d -m 0700 /out/admin-state

# Google Chrome's current stable package, from Google's signed apt repository. Its key is trusted only as the one
# certificate with the fingerprint EB4C1BFD4F042F6DDDCCEC917721F63BD38B4796 (Google's Linux package signing key), and
# apt checks the repository's signed index against it, then the package's SHA-256 against the index. The index is
# fetched on every build, so a new Chrome stable runs this stage again, while an unchanged package keeps the stages
# that copy it cached.
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

# The admin web application and its client.
FROM docker.io/library/node:26@sha256:32fa97f3363975684b08bf4e8a68a47c7905175cc20275b50b974bbd02aba731 AS client
WORKDIR /src/client
COPY client/package.json client/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY client/ ./
COPY protocol/vectors /src/protocol/vectors
# The bench profile sets CLIENT_BENCH=1 to add the benchmark page; the runtime image has none.
ARG CLIENT_BENCH=0
# The SDK the build put in dist/sdk/ moves out of the page's files: it is served once, at /sdk/ (not also under the
# page's own path, such as /demo/sdk/). The client's tests run in scripts/check.sh and CI, which gate every change, and
# not in each installation's build.
RUN if [ "$CLIENT_BENCH" = 1 ]; then npm run build:bench; else npm run build; fi && mv dist/sdk /src/sdk

FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3 AS admin
COPY --from=build /out/admin /usr/local/bin/admin
COPY --from=client /src/client/dist /srv/admin
# The viewer SDK, served at /sdk/ even when ADMIN_STATIC points at a custom client.
COPY --from=client /src/sdk /srv/sdk
# An empty named volume mounted here takes this owner and mode on first use.
COPY --from=build --chown=10004:10014 /out/admin-state /var/lib/admin
USER 10004:10014
ENTRYPOINT ["/usr/local/bin/admin"]

# The automation agent: the pinned Cua Driver, started only by the controller's exec of agent-mcp.
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS cua
# The exact Cua Driver release, checked against its SHA-256 and never fetched at run time.
ADD --checksum=sha256:05dae2160c86bdf4855c279d30898213d7a150edb1992b7470afdd69f0df4064 \
    https://github.com/trycua/cua/releases/download/cua-driver-rs-v0.36.0/cua-driver-rs-0.36.0-linux-x86_64-binary.tar.gz \
    /tmp/cua.tar.gz
RUN mkdir /cua && tar -xzf /tmp/cua.tar.gz -C /cua cua-driver

# The driver's X11 and keymap libraries (all MIT/X11 or BSD), with their dpkg records and copyright files.
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS agent-files
ARG PACKAGES="libx11-6 libxi6 libxext6 libxcb1 libxau6 libxdmcp6 libxkbcommon0 xkb-data libx11-data"
# Every file and symbolic link each package installs, without manual pages and lintian overrides; any failed step
# fails the build, so the image never lacks a library the driver loads only at run time.
RUN set -eu; apt-get update; DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends $PACKAGES; \
    mkdir -p /out/var/lib/dpkg/status.d; \
    for p in $PACKAGES; do \
      dpkg -L $p > /tmp/files; \
      while read -r f; do \
        if [ -L "$f" ] || { [ -f "$f" ] && ! echo "$f" | grep -q '^/usr/share/\(man\|lintian\)'; }; then \
          mkdir -p "/out$(dirname "$f")" && cp -a "$f" "/out$f" || exit 1; \
        fi; \
      done < /tmp/files; \
      dpkg-query -s $p > /out/var/lib/dpkg/status.d/$p; \
    done

# Distroless: glibc and libgcc, no shell, no package manager.
FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3 AS agent
COPY --from=agent-files /out/ /
COPY --from=cua --chmod=0111 /cua/cua-driver /usr/local/bin/cua-driver
# Both of the agent's own programs are execute-only too, so the kernel marks them non-dumpable from their exec: in
# DevTools mode Chrome shares their uid and PID namespace, and could otherwise reach the agent's mount namespace through
# /proc/<pid>/root, a handle that a later exec of the driver would not revoke.
COPY --from=build --chmod=0111 /out/agent-mcp /usr/local/bin/agent-mcp
COPY --from=build --chmod=0111 /out/agent-idle /usr/local/bin/agent-idle
USER 10005:10005
ENTRYPOINT ["/usr/local/bin/agent-idle"]

# Google Chrome (stable, the owner-approved proprietary exception in docs/licenses.md) on the easi.run compositor, with
# its session and accessibility buses and the PulseAudio daemon that carries its sound to the display.
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS browser-rootfs
COPY --from=chrome /chrome/google-chrome-stable.deb /tmp/google-chrome-stable.deb
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        /tmp/google-chrome-stable.deb dbus-daemon at-spi2-core pulseaudio libegl1 libgles2 \
        fonts-dejavu-core fonts-liberation2 fonts-noto-core fonts-noto-color-emoji fonts-noto-cjk \
    && rm -rf /var/lib/apt/lists/* /tmp/google-chrome-stable.deb /etc/apt/sources.list.d/google-chrome.list \
        /etc/cron.daily/google-chrome \
    && groupadd -g 10001 browser && groupadd -g 10011 wayland && groupadd -g 10012 bus \
    && groupadd -g 10005 agent && groupadd -g 10016 proxyclients \
    && useradd -u 10001 -g 10001 -G 10011,10012,10016 -M -d /home/browser -s /usr/sbin/nologin browser \
    && useradd -u 10005 -g 10005 -G 10012 -M -d /tmp -s /usr/sbin/nologin agent \
    && sed -i 's#<allow user="root"/>#<allow user="browser"/><allow user="agent"/>#' \
        /usr/share/defaults/at-spi2/accessibility.conf \
    && mkdir -p /home/browser /downloads /run/browser/bus /run/browser/wayland /run/browser/proxy \
        /etc/opt/chrome/policies/managed \
    && chown 10001:10001 /home/browser /downloads
COPY browser/policy.json /etc/opt/chrome/policies/managed/display.json
COPY browser/session.conf /etc/display-browser/session.conf
COPY browser/pulse-client.conf /etc/display-browser/pulse-client.conf
# PulseAudio keeps only the modules browser-launch loads: the native protocol on a private socket, the pipe sink that
# writes to the display's FIFO, and suspend on idle. Its tools and the dependencies only other modules use (the ALSA
# plugins and their codecs, JACK, GStreamer, FFTW, echo cancellation) are pruned below.
RUN cd /usr/lib/pulse-17.0+dfsg1/modules \
    && find . -type f ! -name module-native-protocol-unix.so ! -name libprotocol-native.so \
        ! -name module-pipe-sink.so ! -name module-suspend-on-idle.so -delete \
    && rm -rf /etc/pulse /usr/share/pulseaudio
COPY --from=build /out/browser-launch /usr/local/bin/browser-launch
# Nothing at run time executes a shell, core utilities, a package manager, or login tooling: remove those programs
# (GPL-licensed) and the libraries and data only removed packages use, without a shell, and keep the libraries
# Chromium, D-Bus, and AT-SPI link.
COPY --from=build /out/dpkg-prune /dpkg-prune
RUN ["/dpkg-prune", "apt", "libapt-pkg7.0", "bash", "coreutils", "diffutils", "findutils", "grep", "gzip", \
     "hostname", "login", "login.defs", "passwd", "mount", "util-linux", "sysvinit-utils", "init-system-helpers", \
     "bsdutils", "debianutils", "perl-base", "sed", "tar", "libsemanage2", "libsemanage-common", "libpam-modules", \
     "libpam-modules-bin", "libpam-runtime", "libdebconfclient0", "base-passwd", "dconf-service", \
     "gtk-update-icon-cache", "libdb5.3t64", "ncurses-bin", "systemd", "systemd-sysv", "libsystemd-shared", \
     "libpam-systemd", "dbus-user-session", "procps", "libproc2-0", "libsensors-config", "openssl", \
     "libtiff6", "libjbig0", \
     "pulseaudio-utils", "libpulsedsp", "libcap2-bin", "adduser", \
     "libfftw3-single3", "libgstreamer1.0-0", "libgstreamer-plugins-base1.0-0", "libwebrtc-audio-processing-1-3", \
     "libabsl20240722", "libasound2-plugins", "libjack-jackd2-0", "libsamplerate0", "libspeex1", \
     "libavcodec61", "libavutil59", "libswresample5", "libaom3", "libcodec2-1.2", "libgsm1", "libhwy1t64", \
     "libjxl0.11", "libnuma1", "librav1e0.7", "librsvg2-2", "libshine3", "libsnappy1v5", "libsvtav1enc2", \
     "libtheoradec1", "libtheoraenc1", "libtwolame0", "libva-drm2", "libva-x11-2", "libva2", "libvdpau1", \
     "libvpl2", "libvpx9", "libwebpmux3", "libx264-164", "libx265-215", "libxvidcore4", "libzvbi-common", \
     "libzvbi0t64", "ocl-icd-libopencl1", "iso-codes", \
     "mawk", "dash", "dpkg", \
     "wget", "xdg-utils", "libcurl3t64-gnutls", "libldap2", "libsasl2-2", "libsasl2-modules-db", "libnghttp2-14", \
     "libnghttp3-9", "libngtcp2-16", "libngtcp2-crypto-gnutls8", \
     "libacl1", "libattr1", "libcrypt1", "libpam0g", "liblastlog2-2", "libsmartcols1", "libuuid1", "libncursesw6", \
     "libseccomp2", "libsepol2", "libice6", "libsm6", "libpsl5t64", "librtmp1", "libssh2-1t64", "libdav1d7", \
     "libopenjp2-7", "libdeflate0", "liblerc4", "libwebp7", "libsharpyuv0", "libdconf1", "debconf", \
     "debian-archive-keyring", "sqv", "dbus", "dbus-system-bus-common"]

# One flat layer: the removed programs exist in no layer at all.
FROM scratch AS browser
COPY --from=browser-rootfs / /
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/browser-launch"]

# No shell, no package manager: glibc and libgcc only. The browser seccomp profile is read at run time from
# CONTROLLER_BROWSER_SECCOMP (default /etc/controller/browser-seccomp.json).
FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3 AS controller
COPY --from=build /out/controller /controller
COPY browser/seccomp.json /etc/controller/browser-seccomp.json
# The Docker socket's group and the controller volume's group (10015) come from the service's group_add.
USER 10006:10006
ENTRYPOINT ["/controller"]

# No shell, no package manager: glibc and libgcc only.
FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3 AS proxy
COPY --from=build /out/proxy /proxy
USER 10003:10003
EXPOSE 3128
ENTRYPOINT ["/proxy"]

# The benchmark baseline (bench profile only, never a runtime image): the same Google Chrome as easi.run's browser
# (the same package, from the same stage, and its fonts and Mesa for WebGL) on KasmVNC 1.5.0's X server, with
# KasmVNC's own web client (its noVNC fork) at /vnc.html, over HTTPS with the image's snakeoil certificate (its user is
# in ssl-cert, as KasmVNC's setup asks). KasmVNC is GPL-2.0 and Google Chrome proprietary; this image is built locally
# for measurement and not distributed (see the README's license section).
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS baseline
ADD --checksum=sha256:80b241de7dfe53bba2b7e1cc5ac8c5246d72271efa16be2d4f76607f30fab1c4 \
    https://github.com/kasmtech/KasmVNC/releases/download/v1.5.0/kasmvncserver_trixie_1.5.0_amd64.deb /tmp/kasmvnc.deb
COPY --from=chrome /chrome/google-chrome-stable.deb /tmp/google-chrome-stable.deb
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        /tmp/google-chrome-stable.deb libegl1 libgles2 \
        fonts-dejavu-core fonts-liberation2 fonts-noto-core fonts-noto-color-emoji fonts-noto-cjk /tmp/kasmvnc.deb \
    && rm -rf /var/lib/apt/lists/* /tmp/*.deb /etc/apt/sources.list.d/google-chrome.list /etc/cron.daily/google-chrome \
    && groupadd -g 10001 browser && useradd -u 10001 -g 10001 -G ssl-cert -M -d /home/browser -s /usr/sbin/nologin browser \
    && mkdir -p /home/browser /etc/opt/chrome/policies/managed && chown 10001:10001 /home/browser
COPY browser/policy.json /etc/opt/chrome/policies/managed/display.json
COPY --chmod=0755 bench/kasmvnc/launch.sh /usr/local/bin/baseline-launch
# By name, so Docker applies its supplementary ssl-cert group (a numeric USER gets none).
USER browser
EXPOSE 6901
ENTRYPOINT ["/usr/local/bin/baseline-launch"]
