# syntax=docker/dockerfile:1
# The admin, agent, browser, controller and proxy images, and the benchmark baseline: one target each, built from the
# repository root, e.g. docker build --target browser -t easi-run/browser:dev .
# They share two stages: `build`, one cargo invocation for every Rust program they ship, so one build compiles each
# shared crate once and each image copies its own programs from /out; and `chrome`, so the browser and the baseline
# install one Google Chrome. (The display and the fixture host have Dockerfiles of their own.) The admin, agent,
# controller and proxy images are distroless: glibc and libgcc, no shell, no package manager.
#
# The browser image is two targets: `browser-base`, everything but Chrome, which CI publishes; and `browser`, which adds
# Google Chrome to it, since Google's terms let no one else ship Chrome. BROWSER_BASE names the image it adds Chrome
# to: the `browser-base` stage by default, or, from install.sh, the published image by digest (`--build-arg
# BROWSER_BASE=ghcr.io/julianosig/easi.run-browser@sha256:...`), and then nothing else here is built.
ARG BROWSER_BASE=browser-base

# The workspace as cargo reads it: its manifest and lock file, every member's manifest, sources and build script (cargo
# loads each member, built or not), the patched dependencies, and the Wayland protocol the crates generate code from;
# so a change to the client, a test, or a fixture leaves the stages below cached. display/Dockerfile and
# bench/Dockerfile copy the same.
FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS workspace
WORKDIR /src
COPY --parents Cargo.toml Cargo.lock admin/Cargo.toml admin/src affinity/Cargo.toml affinity/src agent/Cargo.toml \
    agent/src bench/Cargo.toml bench/src browser/launch/Cargo.toml browser/launch/build.rs browser/launch/src \
    controller/Cargo.toml controller/src display/Cargo.toml display/build.rs display/src egress/Cargo.toml egress/src \
    line/Cargo.toml line/src locale/Cargo.toml locale/src protocol/Cargo.toml protocol/src protocol/wayland \
    proxy/Cargo.toml proxy/src tests/Cargo.toml tests/src third_party/wayland-backend \
    third_party/wtransport ./

# Its dependency skeleton (scripts/skeleton.sh), which only a manifest or the lock file changes.
FROM workspace AS skeleton
COPY scripts/skeleton.sh /skeleton.sh
RUN /skeleton.sh /src

# The runtime programs' dependencies, compiled from the skeleton into this stage's own layer (no cache mount): a CI
# runner keeps image layers between runs but no cargo target directory, so a source change then compiles the
# workspace's crates alone. The skeleton's own crates are cleaned away, so none of them stands in for the real one.
FROM docker.io/library/rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS deps
WORKDIR /src
COPY --from=skeleton /src ./
RUN --mount=type=cache,id=cargo-registry-main,target=/usr/local/cargo/registry,sharing=locked \
    cargo build --locked --release -p admin -p agent -p browser-launch -p controller -p proxy \
    && cargo clean --release $(cat .members) && mv target /deps

# The runtime images' Rust programs. The target directory is a cache mount, so a local rebuild compiles only what
# changed; an empty one (a CI runner's) starts from the dependencies above.
FROM deps AS build
COPY --from=workspace /src ./
RUN --mount=type=cache,id=cargo-registry-main,target=/usr/local/cargo/registry,sharing=locked --mount=type=cache,id=cargo-target-main,target=/src/target \
    { [ -e target/release ] || cp -a /deps/. target/; } \
    && cargo build --locked --release -p admin -p agent -p browser-launch -p controller -p proxy \
    && install -D -m 0555 -t /out target/release/admin target/release/agent-mcp target/release/agent-idle \
        target/release/browser-launch target/release/dpkg-prune target/release/controller target/release/proxy \
    && install -d -m 0700 /out/admin-state
# What each image owes for what it ships, in its /usr/share/doc/easi.run (scripts/notices.py): the repository's
# LICENSE, SOURCE (where its source is: the commit CI builds it from, SOURCE_REVISION), and the license and notice texts
# of the crates its programs link (with the features this stage's one build unifies), from the sources cargo fetched; a
# crate with none fails the build. Every crate of the lock file is fetched, as cargo metadata reads them all.
COPY scripts/notices.py scripts/notices.py
COPY scripts/notices scripts/notices
COPY LICENSE LICENSE
ARG SOURCE_REVISION=
RUN --mount=type=cache,id=cargo-registry-main,target=/usr/local/cargo/registry,sharing=locked \
    cargo fetch --locked --target x86_64-unknown-linux-gnu \
    && for image in admin:admin controller:controller proxy:proxy agent:agent browser:browser-launch; do \
        python3 scripts/notices.py source "/out/notices/${image%%:*}" "$SOURCE_REVISION" \
        && python3 scripts/notices.py cargo "/out/notices/${image%%:*}/rust-crates.md" "${image#*:}" \
            --built-with admin,agent,browser-launch,controller,proxy || exit 1; \
    done

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
# The languages and message formatting the client shares with the site (i18n/), beside it as in the repository.
COPY i18n/ /src/i18n/
COPY client/ ./
# The bench profile sets CLIENT_BENCH=1 to add the benchmark page; the runtime image has none.
ARG CLIENT_BENCH=0
# The SDK the build put in dist/sdk/ moves out of the page's files: it is served once, at /sdk/ (not also under the
# page's own path, such as /demo/sdk/). The client's tests run in scripts/check.sh and CI, which gate every change, and
# not in each installation's build.
RUN if [ "$CLIENT_BENCH" = 1 ]; then npm run build:bench; else npm run build; fi && mv dist/sdk /src/sdk
# The license and notice texts of the npm packages the page bundles (its fonts' included), for the admin image.
COPY scripts/notices.py /src/scripts/notices.py
COPY scripts/notices /src/scripts/notices
RUN python3 /src/scripts/notices.py npm /src/notices/npm-packages.md /src/client

# The base of every Rust server's image: the C runtime its binaries link, no shell, no package manager.
FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3 AS runtime

FROM runtime AS admin
COPY --from=build /out/admin /usr/local/bin/admin
COPY --from=client /src/client/dist /srv/admin
# The viewer SDK, served at /sdk/ even when ADMIN_STATIC points at a custom client.
COPY --from=client /src/sdk /srv/sdk
COPY --from=build /out/notices/admin/ /usr/share/doc/easi.run/
COPY --from=client /src/notices/ /usr/share/doc/easi.run/
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

FROM runtime AS agent
COPY --from=agent-files /out/ /
COPY --from=cua --chmod=0111 /cua/cua-driver /usr/local/bin/cua-driver
# Both of the agent's own programs are execute-only too, so the kernel marks them non-dumpable from their exec: in
# DevTools mode Chrome shares their uid and PID namespace, and could otherwise reach the agent's mount namespace through
# /proc/<pid>/root, a handle that a later exec of the driver would not revoke.
COPY --from=build --chmod=0111 /out/agent-mcp /usr/local/bin/agent-mcp
COPY --from=build --chmod=0111 /out/agent-idle /usr/local/bin/agent-idle
# The Cua Driver's own license and third-party notices, and those of the crates it links (scripts/notices/README.md).
COPY --from=build /out/notices/agent/ /usr/share/doc/easi.run/
COPY scripts/notices/cua-driver-rs-0.36.0/ /usr/share/doc/easi.run/cua-driver/
USER 10005:10005
ENTRYPOINT ["/usr/local/bin/agent-idle"]

# What Google Chrome (stable, the owner-approved proprietary exception in docs/licenses.md) runs with, without Chrome:
# the easi.run compositor's client side, the session and accessibility buses, the PulseAudio daemon that carries its
# sound to the display, and every package Chrome's package depends on, which apt-get satisfy installs from its
# Pre-Depends and Depends fields, so no file of Chrome's, nor anything its install scripts write (its icons, its
# repository's key), is here. The fields are kept, for the `chrome-files` stage to compare with the package it adds.
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS browser-rootfs
COPY --from=chrome /chrome/google-chrome-stable.deb /tmp/google-chrome-stable.deb
RUN apt-get update && deb=/tmp/google-chrome-stable.deb \
    && DEBIAN_FRONTEND=noninteractive apt-get satisfy -y --no-install-recommends "$(dpkg-deb -f "$deb" Pre-Depends)" \
        "$(dpkg-deb -f "$deb" Depends)" "dbus-daemon, at-spi2-core, pulseaudio, libegl1, libgles2, fonts-dejavu-core, \
        fonts-liberation2, fonts-noto-core, fonts-noto-color-emoji, fonts-noto-cjk" \
    && mkdir -p /etc/display-browser && dpkg-deb -f "$deb" Pre-Depends Depends >/etc/display-browser/chrome-depends \
    && rm -rf /var/lib/apt/lists/* /tmp/google-chrome-stable.deb \
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
COPY --from=build /out/notices/browser/ /usr/share/doc/easi.run/
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
FROM scratch AS browser-base
COPY --from=browser-rootfs / /
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/browser-launch"]

FROM ${BROWSER_BASE} AS browser-parent

# Chrome's files, as dpkg would install them here (without the manual page and changelog this image's dpkg leaves out,
# and the cron job that would add Google's repository), and the base's package database with Chrome's entry added, so
# the image's SBOM lists it. A Chrome whose package declares other dependencies than the ones the base was built for
# (exactly) stops here: the next release's base follows them.
FROM chrome AS chrome-files
COPY --from=browser-parent /etc/display-browser/chrome-depends /var/lib/dpkg/status /base/
RUN set -eu; deb=/chrome/google-chrome-stable.deb; \
    if [ "$(dpkg-deb -f "$deb" Pre-Depends Depends)" != "$(cat /base/chrome-depends)" ]; then \
      echo "Google's newest Chrome stable, $(dpkg-deb -f "$deb" Version), declares other dependencies than this release's browser image was built for: the next release's image follows them" >&2; \
      exit 1; \
    fi; \
    dpkg-deb -x "$deb" /out; \
    rm -rf /out/etc/cron.daily /out/usr/share/man /out/usr/share/doc/google-chrome-stable/changelog.gz; \
    mkdir -p /out/var/lib/dpkg; \
    { printf '%s\n\n' "$(cat /base/status)"; dpkg-deb -f "$deb" | sed '/^Package:/a Status: install ok installed'; echo; } \
        >/out/var/lib/dpkg/status

# The browser: the base with Chrome. Chrome printing its version proves it finds every library it links (the RUN, in
# exec form, needs no shell, and leaves no file).
FROM browser-parent AS browser
COPY --from=chrome-files /out/ /
RUN ["/opt/google/chrome/chrome", "--version"]

FROM runtime AS controller
COPY --from=build /out/controller /controller
COPY browser/seccomp.json /etc/controller/browser-seccomp.json
COPY --from=build /out/notices/controller/ /usr/share/doc/easi.run/
# The Docker socket's group and the controller volume's group (10015) come from the service's group_add.
USER 10006:10006
ENTRYPOINT ["/controller"]

FROM runtime AS proxy
COPY --from=build /out/proxy /proxy
COPY --from=build /out/notices/proxy/ /usr/share/doc/easi.run/
USER 10003:10003
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
COPY browser/launch/src/chrome-flags /usr/local/share/baseline/chrome-flags
# By name, so Docker applies its supplementary ssl-cert group (a numeric USER gets none).
USER browser
ENTRYPOINT ["/usr/local/bin/baseline-launch"]
