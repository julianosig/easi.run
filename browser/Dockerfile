# syntax=docker/dockerfile:1
# Google Chrome (stable, the owner-approved proprietary exception in docs/licenses.md) on the easi.run
# compositor, with its session and accessibility buses and the PulseAudio daemon that carries its sound to the display.
# Build from the repository root: docker build -f browser/Dockerfile -t easi-run/browser:dev .

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

FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS rootfs
COPY --from=chrome /chrome/google-chrome-stable.deb /tmp/google-chrome-stable.deb
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        /tmp/google-chrome-stable.deb dbus-daemon dbus-bin at-spi2-core pulseaudio libegl1 libegl-mesa0 libgles2 \
        fonts-dejavu-core fonts-liberation2 fonts-noto-core fonts-noto-color-emoji fonts-noto-cjk ca-certificates \
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
# (GPL-licensed), without a shell, and keep the libraries Chromium, D-Bus, and AT-SPI link.
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
     "libnghttp3-9", "libngtcp2-16", "libngtcp2-crypto-gnutls8"]

# One flat layer: the removed programs exist in no layer at all.
FROM scratch
COPY --from=rootfs / /
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/browser-launch"]
