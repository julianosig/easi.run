#!/usr/bin/env bash
# easi.run's installer, for one Linux x86-64 server with Docker. Read it before you run it, then run it as the account
# that will own the installation, never as root:
#
#   curl -fsSLO https://easi.run/install.sh && less install.sh && bash install.sh
#
#   bash install.sh            installs easi.run, or checks again an installation already at the latest release
#   bash install.sh --update   moves an installation to the latest release: its live browser sessions end
#   bash install.sh --dry-run  runs the checks and says what it would do, changing nothing
#
# In order, it:
#   1. checks this server: Linux on x86-64 with systemd; rootful Docker Engine 28+ on this server's own socket, with
#      Compose v2; git, curl, sudo and ss; 4 GB of memory; 20 GB free where Docker
#      keeps its images; port 9443 free, or held by this installation;
#   2. clones easi.run into ~/easi.run (EASI_RUN_DIR) at the latest release tag, or at the commit named below while
#      there is none, and records which;
#   3. with sudo, installs a drop-in for docker.service that puts `iptables -I INPUT -i er-+ -j DROP` first in INPUT
#      (pages reach the internet, never this server's own services) every time Docker starts, before it does: Docker,
#      and every container it restarts, starts only once the rule is in place; and puts it in place now;
#   4. the first time, writes a random password to ~/.config/easi-run/password (mode 0600) and gives it to the admin's
#      `admin init` on stdin, which stores only its Argon2id hash;
#   5. builds and starts the stack, waits for the admin over HTTPS, checked against the admin's own certificate, and
#      checks that Docker runs its default userland proxy for it, which the firewall rule relies on;
#   6. adds the admin's MCP server to Claude Code (`claude mcp add --scope user easi`) when claude is installed and no
#      other `easi` server is there.
#
# It never prints the password or keeps it anywhere else, and it runs nothing from the checkout as root.
set -euo pipefail

REPO="${EASI_RUN_REPO:-https://github.com/julianosig/easi.run}"
# The commit installed while the repository has no release tag (vMAJOR.MINOR.PATCH); a release tag always wins.
PINNED_COMMIT=a6ccbd803b4e87879b9c0b6ddac0f14ef9b52e0b
DIR="${EASI_RUN_DIR:-$HOME/easi.run}"
CONFIG="$HOME/.config/easi-run"
PORT=9443
PROJECT=easi-run
DROPIN_DIR=/etc/systemd/system/docker.service.d
DROPIN_FILE=$DROPIN_DIR/easi-run-firewall.conf
RULE="-A INPUT -i er-+ -j DROP"
DOCKER_SOCKET=/var/run/docker.sock
CONTAINERD_ROOT=/var/lib/containerd
CONTAINERD_CONFIG=/etc/containerd/config.toml
# The images the controller starts sessions from, which it looks up once, when it starts.
SESSION_IMAGES=(easi-run/display:dev easi-run/browser:dev easi-run/agent:dev easi-run/proxy:dev)
MEMINFO=/proc/meminfo
# 4 GB and 20 GB, in KiB.
MIN_MEMORY_KB=3906250
MIN_DISK_KB=19531250
# What the summary reports, set as the steps run.
MCP_STATUS=""
PASSWORD_NOTE=""

say() { printf '%s\n' "$*"; }
step() { printf '\n==> %s\n' "$*"; }
die() {
  printf 'install.sh: %s\n' "$*" >&2
  exit 1
}

# Whether version $1 (such as 28.1.1 or v2.30.0) has a major number of at least $2.
major_at_least() {
  local major="${1#v}"
  major="${major%%.*}"
  case "$major" in '' | *[!0-9]*) return 1 ;; esac
  [ "$major" -ge "$2" ]
}

has_systemd() { [ -d /run/systemd/system ]; }

memory_kb() { awk '$1 == "MemTotal:" {print $2}' "$MEMINFO"; }

# Docker Compose, for this checkout and nothing else: its project name, Compose file and port, whatever the shell or
# an environment file sets (only the checkout's own .env is read, and these flags win over it). Builds carry no
# attestation, so an image that did not change keeps its id.
compose() {
  env -u COMPOSE_PROJECT_NAME -u COMPOSE_FILE -u COMPOSE_PROFILES -u COMPOSE_ENV_FILES -u COMPOSE_PATH_SEPARATOR \
    ADMIN_PORT="$PORT" DOCKER_GID="$DOCKER_GID" BUILDX_NO_DEFAULT_ATTESTATIONS=1 \
    docker compose --project-directory "$DIR" --project-name "$PROJECT" --file "$DIR/compose.yaml" "$@"
}

preflight_host() {
  [ "$(id -u)" != 0 ] || die "run it as the account that will own easi.run, not as root: it asks sudo for the firewall rule, and to look at Docker's processes"
  if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
    die "easi.run needs Linux on x86-64; this is $(uname -s) on $(uname -m)"
  fi
  has_systemd || die "this installer needs systemd, which keeps the firewall rule in place before Docker starts"
  local tool
  for tool in git curl sudo docker ss systemctl pgrep awk stat df timeout; do
    command -v "$tool" >/dev/null || die "$tool is missing: install it, then run install.sh again"
  done
  case "$DIR" in /*) ;; *) die "EASI_RUN_DIR must be an absolute path: $DIR" ;; esac
  local memory
  memory="$(memory_kb)"
  [ "${memory:-0}" -ge "$MIN_MEMORY_KB" ] || die "easi.run needs 4 GB of memory; this server has $((${memory:-0} / 1000)) MB"
}

preflight_docker() {
  if [ -n "${DOCKER_HOST:-}" ] && [ "$DOCKER_HOST" != "unix://$DOCKER_SOCKET" ]; then
    die "DOCKER_HOST is $DOCKER_HOST: easi.run runs on this server's own Docker ($DOCKER_SOCKET); unset it"
  fi
  local endpoint
  endpoint="$(docker context inspect --format '{{.Endpoints.docker.Host}}' 2>/dev/null)" ||
    die "docker context inspect failed: is the Docker CLI installed whole?"
  [ "$endpoint" = "unix://$DOCKER_SOCKET" ] ||
    die "the Docker context reaches $endpoint, not $DOCKER_SOCKET: run docker context use default"
  local version
  version="$(docker version --format '{{.Server.Version}}' 2>/dev/null)" ||
    die "Docker does not answer: is it running, and is $(id -un) in the docker group (sudo usermod -aG docker $(id -un), then log in again)?"
  major_at_least "$version" 28 || die "easi.run needs Docker Engine 28 or later; this one is $version"
  local info
  info="$(docker info --format '{{.OperatingSystem}}|{{json .SecurityOptions}}|{{.DockerRootDir}}')" ||
    die "docker info failed"
  case "$info" in *"Docker Desktop"*) die "Docker Desktop is not supported: use Docker Engine on Linux" ;; esac
  case "$info" in *rootless*) die "rootless Docker is not supported: easi.run needs the system's rootful Docker Engine" ;; esac
  DOCKER_ROOT="${info##*|}"
  local compose_version
  compose_version="$(docker compose version --short 2>/dev/null)" || die "Docker Compose v2 (the docker compose plugin) is missing"
  major_at_least "$compose_version" 2 || die "easi.run needs Docker Compose v2; this one is $compose_version"
  DOCKER_GID="$(stat -c %g "$DOCKER_SOCKET")"
  # Docker's data root, and containerd's when Docker keeps its images in the containerd store.
  local roots=("$DOCKER_ROOT") root free
  case "$(docker info --format '{{json .DriverStatus}}')" in *io.containerd.snapshotter*) roots+=("$(containerd_root)") ;; esac
  for root in "${roots[@]}"; do
    free="$(df -Pk "$root" | awk 'NR == 2 {print $4}')" || die "cannot measure the free space in $root"
    [ "${free:-0}" -ge "$MIN_DISK_KB" ] ||
      die "easi.run needs 20 GB free where Docker keeps its images ($root); $((${free:-0} / 1000000)) GB are"
  done
}

# Port 9443 free, or published by this checkout's own admin.
preflight_port() {
  [ -n "$(ss -Hltn "sport = :$PORT")" ] || return 0
  local ours
  ours="$(docker ps -q --filter "label=com.docker.compose.project=$PROJECT" \
    --filter label=com.docker.compose.service=admin \
    --filter "label=com.docker.compose.project.working_dir=$DIR" --filter "publish=$PORT")"
  [ -n "$ours" ] || die "port $PORT is in use by something other than this installation: free it, then run install.sh again"
}

preflight_sudo() {
  sudo -n true 2>/dev/null && return 0
  [ -t 0 ] || die "sudo needs a password and there is no terminal to ask it in: run bash install.sh yourself, in a terminal"
  say "sudo asks for your password: for the firewall rule, and to look at Docker's processes."
  sudo -v
}

# The release to install: the latest vMAJOR.MINOR.PATCH tag, or the pinned commit while there is none.
resolve_target() {
  local tags latest
  tags="$(git ls-remote --tags "$REPO" 'refs/tags/v*')" || die "cannot reach $REPO"
  latest="$(printf '%s\n' "$tags" | awk '{print $2}' |
    sed -n 's#^refs/tags/\(v[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)$#\1#p' | sort -V | tail -n 1)"
  if [ -n "$latest" ]; then
    TARGET="release $latest"
    # An annotated tag's commit is its peeled line; a lightweight tag names the commit itself.
    TARGET_COMMIT="$(printf '%s\n' "$tags" | awk -v t="refs/tags/$latest^{}" '$2 == t {print $1}')"
    [ -n "$TARGET_COMMIT" ] || TARGET_COMMIT="$(printf '%s\n' "$tags" | awk -v t="refs/tags/$latest" '$2 == t {print $1}')"
  else
    TARGET="commit $PINNED_COMMIT (no release yet)"
    TARGET_COMMIT="$PINNED_COMMIT"
  fi
  case "$TARGET_COMMIT" in *[!0-9a-f]* | '') die "no commit to install from $REPO" ;; esac
  [ "${#TARGET_COMMIT}" = 40 ] || die "no commit to install from $REPO"
}

# The checkout at the target commit: cloned the first time; moved only with --update.
checkout() {
  if [ -e "$DIR" ]; then
    [ -d "$DIR/.git" ] || die "$DIR is there and is not an easi.run checkout: move it, or set EASI_RUN_DIR"
    [ "$(git -C "$DIR" remote get-url origin)" = "$REPO" ] || die "$DIR is a checkout of another repository"
    [ -z "$(git -C "$DIR" status --porcelain --untracked-files=no)" ] ||
      die "$DIR has local changes: commit or discard them, then run install.sh again"
    local current
    current="$(git -C "$DIR" rev-parse HEAD)"
    [ "$current" = "$TARGET_COMMIT" ] && return 0
    [ "$UPDATE" = 1 ] ||
      die "$DIR is at ${current:0:12}; the latest is $TARGET. Run bash install.sh --update to move to it (live browser sessions end)"
  else
    git clone --quiet --no-checkout "$REPO" "$DIR"
  fi
  git -C "$DIR" fetch --quiet origin "$TARGET_COMMIT"
  git -C "$DIR" -c advice.detachedHead=false checkout --quiet --detach "$TARGET_COMMIT"
  [ "$(git -C "$DIR" rev-parse HEAD)" = "$TARGET_COMMIT" ] || die "the checkout is not at $TARGET_COMMIT"
}

# docker.service's drop-in, with fixed commands only: before every start of Docker, it puts the rule first in INPUT
# unless it is first already, then fails unless it is.
dropin_text() {
  local iptables="$1"
  cat <<EOF
# Installed by easi.run's install.sh. Keeps browser sessions off this server's own services: every time Docker starts,
# before it does, one rule goes first in INPUT, dropping every packet that comes in through a session bridge
# (er-<session>), and Docker starts only once it is there. Restarting Docker puts it back after a firewall reload.
[Unit]
After=ufw.service firewalld.service netfilter-persistent.service iptables.service nftables.service

[Service]
ExecStartPre=/bin/sh -c 'test "\$\$($iptables -w -S INPUT 1)" = "$RULE" || $iptables -w -I INPUT 1 -i er-+ -j DROP'
ExecStartPre=/bin/sh -c 'test "\$\$($iptables -w -S INPUT 1)" = "$RULE"'
EOF
}

# Writes $2 to the root-owned file $1 (mode 0644) unless it holds that already; succeeds only when it wrote.
place() {
  local current temp
  current="$(sudo cat "$1" 2>/dev/null || true)"
  [ "$current" != "$2" ] || return 1
  temp="$(mktemp)"
  printf '%s\n' "$2" >"$temp"
  sudo install -m 0644 -o root -g root "$temp" "$1"
  rm -f "$temp"
}

rule_first() { [ "$(sudo "$1" -w -S INPUT 1 2>/dev/null)" = "$RULE" ]; }

firewall() {
  local iptables pre
  iptables="$(sudo sh -c 'command -v iptables')" || die "iptables is missing: install it, then run install.sh again"
  case "$iptables" in /usr/sbin/iptables | /sbin/iptables | /usr/bin/iptables | /bin/iptables) ;;
  *) die "iptables is at $iptables, outside the system's own directories" ;;
  esac
  sudo install -d -m 0755 -o root -g root "$DROPIN_DIR"
  if place "$DROPIN_FILE" "$(dropin_text "$iptables")"; then
    sudo systemctl daemon-reload
  fi
  pre="$(systemctl show docker.service --property ExecStartPre --value)"
  case "$pre" in *"$iptables -w -I INPUT 1 -i er-+ -j DROP"*) ;;
  *) die "docker.service does not put the rule in place before it starts: $DROPIN_FILE did not load" ;;
  esac
  # Now as well, for the Docker already running, which is never restarted here.
  rule_first "$iptables" || sudo "$iptables" -w -I INPUT 1 -i er-+ -j DROP
  rule_first "$iptables" || die "the rule is not first in INPUT: sudo $iptables -S INPUT"
}

config_dir() {
  [ ! -L "$CONFIG" ] || die "$CONFIG is a symbolic link: easi.run keeps its password only in a real directory"
  (umask 077 && mkdir -p "$CONFIG")
  [ "$(stat -c %u "$CONFIG")" = "$(id -u)" ] || die "$CONFIG belongs to another user"
  chmod 0700 "$CONFIG"
}

# The password file, if there is one: a regular file of this user's, mode 0600.
password_file() {
  local file="$CONFIG/password"
  [ ! -L "$file" ] || die "$file is a symbolic link: remove it"
  [ -e "$file" ] || return 1
  if [ ! -f "$file" ] || [ "$(stat -c %u "$file")" != "$(id -u)" ]; then
    die "$file is not a file of $(id -un)'s"
  fi
  chmod 0600 "$file"
}

# A new random password of 32 letters and digits, written whole or not at all, and never over another file.
new_password() {
  local chars temp
  chars="$(head -c 1024 /dev/urandom | LC_ALL=C tr -dc 'A-Za-z0-9')"
  [ "${#chars}" -ge 32 ] || die "could not draw a random password"
  temp="$(mktemp "$CONFIG/.password.XXXXXXXX")"
  printf '%s\n' "${chars:0:32}" >"$temp"
  sync "$temp" 2>/dev/null || true
  if ! ln "$temp" "$CONFIG/password"; then
    rm -f "$temp"
    die "could not write $CONFIG/password"
  fi
  rm -f "$temp"
}

# `admin init`, a one-off container with no network and no log, given the password on stdin.
initialize() {
  local generated=0 status=0
  config_dir
  if ! password_file; then
    new_password
    generated=1
  fi
  compose run --rm -T --no-deps init <"$CONFIG/password" || status=$?
  case "$status" in
  0) PASSWORD_NOTE="$CONFIG/password" ;;
  3)
    if [ "$generated" = 1 ]; then
      # Not the admin's password: the one set before stays, and this file would only mislead.
      rm -f "$CONFIG/password"
      PASSWORD_NOTE="set before this install, and kept (no file holds it)"
    else
      PASSWORD_NOTE="set another way, and kept: $CONFIG/password does not hold it"
    fi
    ;;
  *) die "admin init failed (exit $status)" ;;
  esac
}

# Up once the admin answers over HTTPS, checked against the certificate it made, copied out of its volume.
wait_for_admin() {
  local cert="$CONFIG/tls-cert.pem" deadline=$((SECONDS + 180))
  until compose cp admin:/var/lib/admin/tls-cert.pem "$cert.new" >/dev/null 2>&1 && mv -f "$cert.new" "$cert" &&
    curl --noproxy '*' -fsS --max-time 5 --cacert "$cert" --resolve "localhost:$PORT:127.0.0.1" -o /dev/null \
      "https://localhost:$PORT/api/status" 2>/dev/null; do
    [ "$SECONDS" -lt "$deadline" ] ||
      die "the admin did not answer on https://localhost:$PORT within 3 minutes: docker compose logs admin, in $DIR"
    sleep 2
  done
}

# containerd's root: the one its configuration names, or its default.
containerd_root() {
  local root=""
  [ -r "$CONTAINERD_CONFIG" ] && root="$(sed -nE -e 's/^[[:space:]]*root[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' \
    -e "s/^[[:space:]]*root[[:space:]]*=[[:space:]]*'([^']+)'.*/\\1/p" "$CONTAINERD_CONFIG" | head -n 1)"
  printf '%s\n' "${root:-$CONTAINERD_ROOT}"
}

# The session images' ids, one per line (an empty line for one not built yet).
session_images() {
  local image
  for image in "${SESSION_IMAGES[@]}"; do
    docker image inspect --format '{{.Id}}' "$image" 2>/dev/null || echo
  done
}

# The admin's container, by its name.
admin_container() {
  CONTAINER="$(docker inspect --format '{{.Name}}' "$(compose ps -q admin)")"
  CONTAINER="${CONTAINER#/}"
}

# The firewall rule covers what sessions send the host's published ports only through Docker's userland proxy, which
# the running Docker shows by the docker-proxy it runs for the admin's own mapping (127.0.0.1:9443 to the admin's
# address). Without it, the stack stops.
check_proxy() {
  local ip proxies
  ip="$(docker inspect --format '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$CONTAINER")"
  # Through sudo, so a /proc mounted with hidepid still shows Docker's processes. Its time may have run out during the
  # build: that says nothing about the proxy.
  if ! sudo -n true 2>/dev/null; then
    compose down
    die "sudo's time ran out, so Docker's processes could not be checked: run bash install.sh again"
  fi
  proxies="$(sudo -n pgrep -a -x docker-proxy 2>/dev/null || true)"
  if [ -z "$ip" ] || ! printf '%s\n' "$proxies" |
    grep -Fe "-host-ip 127.0.0.1 -host-port $PORT -container-ip $ip -container-port 9443" >/dev/null; then
    compose down
    die "Docker runs without its userland proxy (\"userland-proxy\": false, or --userland-proxy=false), which the firewall rule relies on: turn it on, restart Docker (sudo systemctl restart docker), then run install.sh again"
  fi
}

# Whether the admin's MCP server answers an initialize through `admin mcp`.
mcp_answers() {
  local hello='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"install.sh","version":"1"}}}'
  local answer
  answer="$(printf '%s\n' "$hello" | timeout 30 docker exec -i "$CONTAINER" admin mcp 2>/dev/null)" || return 1
  case "$answer" in *'"protocolVersion"'*) return 0 ;; *) return 1 ;; esac
}

connect_mcp() {
  local add existing
  add="claude mcp add --scope user easi -- docker exec -i $CONTAINER admin mcp"
  if ! mcp_answers; then
    MCP_STATUS="not answering (docker exec -i $CONTAINER admin mcp); see docker compose logs admin, in $DIR"
    return 0
  fi
  if ! command -v claude >/dev/null; then
    MCP_STATUS="add it to Claude Code with: $add"
  elif existing="$(claude mcp get easi </dev/null 2>/dev/null)"; then
    case "$existing" in
    *"exec -i $CONTAINER admin mcp"*) MCP_STATUS="ready (Claude Code: easi)" ;;
    *) MCP_STATUS="Claude Code has another server named easi, left alone; to replace it: claude mcp remove --scope user easi && $add" ;;
    esac
  elif claude mcp add --scope user easi -- docker exec -i "$CONTAINER" admin mcp </dev/null >/dev/null; then
    MCP_STATUS="ready (Claude Code: easi)"
  else
    MCP_STATUS="claude mcp add failed; add it with: $add"
  fi
}

summary() {
  local who
  who="$(id -un)@$(uname -n)"
  say ""
  say "easi.run is running: $TARGET, in $DIR."
  say "  Admin:     https://localhost:$PORT (its certificate: $CONFIG/tls-cert.pem)"
  say "  Password:  $PASSWORD_NOTE"
  say "  MCP:       $MCP_STATUS"
  say "  Codex CLI: codex mcp add easi -- docker exec -i $CONTAINER admin mcp"
  say "From another computer (use this server's address in place of $(uname -n) if it does not resolve):"
  say "  Viewer:    ssh -N -L $PORT:127.0.0.1:$PORT $who, then open https://localhost:$PORT"
  say "  MCP:       claude mcp add --scope user easi -- ssh -T $who docker exec -i $CONTAINER admin mcp"
}

dry_run() {
  say "Checks passed. Without --dry-run, this would:"
  if [ -e "$DIR" ]; then
    say "  - use the checkout in $DIR, moved to $TARGET only with --update"
  else
    say "  - clone $REPO into $DIR at $TARGET"
  fi
  say "  - install (with sudo) $DROPIN_FILE, and put the firewall rule first in INPUT"
  say "  - write a random password to $CONFIG/password unless one is there, and give it to admin init"
  say "  - build and start the stack in $DIR, and wait for https://localhost:$PORT"
  say "  - add the MCP server easi to Claude Code, if claude is installed"
}

main() {
  UPDATE=0
  local dry=0
  case "${1:-}" in
  '') ;;
  --update) UPDATE=1 ;;
  --dry-run) dry=1 ;;
  *) die "usage: bash install.sh [--update | --dry-run]" ;;
  esac
  [ "$#" -le 1 ] || die "usage: bash install.sh [--update | --dry-run]"
  step "Checking this server"
  preflight_host
  preflight_docker
  preflight_port
  resolve_target
  if [ "$dry" = 1 ]; then
    dry_run
    return 0
  fi
  preflight_sudo
  step "Getting easi.run ($TARGET)"
  checkout
  config_dir
  printf '%s %s\n' "$TARGET" "$TARGET_COMMIT" >"$CONFIG/release"
  step "Putting the firewall rule in place"
  firewall
  if [ "$UPDATE" = 1 ]; then
    say "--update: if the admin or a session image changed, live browser sessions end."
  fi
  step "Building the images (the first build takes a while)"
  compose build
  step "Setting the admin's password"
  initialize
  step "Starting easi.run"
  # The check after the start reads Docker's processes through sudo: its time must not run out in between.
  sudo -n true 2>/dev/null || { [ -t 0 ] && sudo -v; } ||
    die "sudo's time ran out during the build, and there is no terminal to ask again: run bash install.sh again"
  local controller images
  controller="$(compose ps -q controller)"
  compose up -d
  # The controller starts sessions from the images it found when it started, so it restarts when one of them is not
  # the one it last started with. The admin restarts whenever the controller did, ending the sessions it no longer
  # holds.
  images="$(session_images)"
  if [ "$images" != "$(cat "$CONFIG/images" 2>/dev/null || true)" ]; then
    compose up -d --no-deps --force-recreate controller admin
  elif [ "$(compose ps -q controller)" != "$controller" ]; then
    compose up -d --no-deps --force-recreate admin
  fi
  wait_for_admin
  printf '%s\n' "$images" >"$CONFIG/images"
  admin_container
  check_proxy
  step "Connecting MCP"
  connect_mcp
  summary
}

# Run, unless sourced (by the installer's tests).
if [ "${BASH_SOURCE[0]:-$0}" = "$0" ]; then
  main "$@"
fi
