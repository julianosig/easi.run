# A chat's container (docs/topology.md): a shell's usual tools, for what its program runs, the terminal and the forwarder,
# and the empty home an account's volume takes its ownership from. No program of any vendor: each comes from its vendor's
# installer, into the account's home, on first use. install.sh builds this on the server it installs, from Debian's own
# packages, as it adds Chrome to the browser image: easi.run publishes no image of it.
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        bash ca-certificates curl findutils git procps ripgrep socat tmux util-linux \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /home/runner /run/chat/proxy /run/chat/mcp /etc/runner && chown 10007:10007 /home/runner
COPY --chmod=0755 start /usr/local/bin/runner-start
COPY --chmod=0755 attach /usr/local/bin/runner-attach
COPY --chmod=0755 chat /usr/local/bin/runner-chat
COPY --chmod=0755 mcp /usr/local/bin/runner-mcp
COPY claude-mcp.json tmux.conf /etc/runner/
USER 10007:10007
ENTRYPOINT ["/usr/local/bin/runner-start"]
