#!/usr/bin/env bash
# easi.run's installer, for one Linux x86-64 server with Docker. Read it before you run it, then run it as the account
# that will own the installation, never as root:
#
#   curl -fsSLO https://easi.run/install.sh && less install.sh && bash install.sh
#
#   bash install.sh            installs the release this file names, or checks again an installation already at it
#                              (and takes Google's newest Chrome stable, if there is one: its live browser sessions end)
#   bash install.sh --update   moves an installation to the release this file names: its live browser sessions end
#   bash install.sh --dry-run  runs the checks and says what it would do, changing nothing
#   bash install.sh --uninstall [--yes]
#                              removes easi.run from this server after asking (--yes: without asking), and says what
#                              it keeps: the checkout and Docker's build cache
#
# In order, it:
#   1. checks this server: Linux on x86-64 with systemd; rootful Docker Engine 28+ on this server's own socket, run by
#      docker.service, with Compose v2 and Buildx; git, curl, ss and flock; 4 GB of memory; 10 GB free where
#      Docker keeps its images; the admin's port (9443, or ADMIN_PORT in the checkout's .env) free, or held by this
#      installation;
#   2. clones easi.run into ~/easi.run (EASI_RUN_DIR) at the commit named below;
#   3. as root (see below), installs a drop-in for docker.service that puts the rule
#      `iptables -t raw -I PREROUTING 1 -i er-+ -m addrtype --dst-type LOCAL -j DROP` first every time Docker starts,
#      before it does: whatever a session bridge (er-<session>) sends an address of this server's own is dropped before
#      Docker's port redirects, the ports Docker publishes included. Docker, and every container it restarts, starts
#      only once the rule is there; and it puts the rule first now;
#   4. as root, when the kernel has BBR: sets this server's TCP to pace with fq and BBR and not restart slow after an
#      idle second (net.core.default_qdisc=fq, net.ipv4.tcp_congestion_control=bbr,
#      net.ipv4.tcp_slow_start_after_idle=0), now and at every boot, in /etc/sysctl.d/90-easi-run.conf and
#      /etc/modules-load.d/easi-run.conf, which keep the values they replace for --uninstall to put back; without BBR it
#      leaves TCP as it is;
#   5. pulls the images CI built from that commit, each by the digest named below, adds Google's current Chrome
#      stable from Google's signed repository to the browser image (Google's terms do not let anyone else ship it),
#      starts the stack, and waits for the admin over HTTPS, checked against the admin's own certificate;
#   6. says whether the admin's password is set, as the admin's status answers, and where you create it if not;
#   7. adds the admin's MCP server to Claude Code (`claude mcp add --scope user easi`) when claude is installed and no
#      other `easi` server is there.
#
# The stream's media go over UDP (the admin's port, 9443/udp, which Docker publishes) where the viewer's network lets
# them, and over the HTTPS tunnel otherwise: the installer tells the admin the address you reached this server at over
# SSH, kept in ~/.config/easi-run for later runs.
#
# It needs no sudo: the account only has to be in the docker group, which Docker's own documentation says is
# root-equivalent. Steps 3 and 4, and their undoing by --uninstall, run as root on this server through Docker: a
# short-lived privileged container of Debian's image (ROOT_IMAGE, pulled by its digest) whose nsenter enters this
# server's own mount, UTS, IPC and network namespaces, so that this server's own shell, iptables, systemctl, sysctl and
# modprobe run as root, as sudo would run them. Docker must let the account run such a container (--privileged, with
# the host's PID and user namespaces): an authorization plugin that refuses it stops the installer.
#
# It never sees the admin's password, which you create in the admin's own screen, and it runs nothing from the
# checkout as root.
set -euo pipefail

REPO="${EASI_RUN_REPO:-https://github.com/julianosig/easi.run}"
# The release: the commit installed, and the images CI built from it and published as
# $REGISTRY-<role>, each pulled by its digest and nothing else (scripts/pin.py writes both). The browser image carries
# no Chrome: the installer adds it.
PINNED_COMMIT=1f0bef1caf26d8d649a908494c5d86f49ad6740d
REGISTRY=ghcr.io/julianosig/easi.run
PINNED_IMAGES=(
  admin@sha256:f5d451a0e7e1392711e2183085391f22ae503101e31e467d41d218e0841ea51e
  controller@sha256:92a7f6544c878281730d77c8f5b56df5f9fadd0047677780b78da17874c257d6
  display@sha256:e27c1c33fabe2d9f6f0dee803422e03db5714c875b05d43b3fbd3378dc89ce3c
  agent@sha256:fbfcdaa5e0f24def7e97ec3e85283faf9408c11e782e22a813e9926a59f0ddc1
  proxy@sha256:6494ae5b6fe1268671e781ea43cfade073ff0f40923f6e824ff06b3d9107f26e
  browser@sha256:ea13feafb216f97681131d737374bdba266446829727d8acd5b9b6c8130e63be
)
# The roles, as PINNED_IMAGES names them in this order.
ROLES=(admin controller display agent proxy browser)
# The label of the browser images the installer builds.
BROWSER_LABEL=run.easi.installer=browser
DIR="${EASI_RUN_DIR:-$HOME/easi.run}"
CONFIG="$HOME/.config/easi-run"
PORT=9443
# Browser sessions at once, and where that number comes from (see max_sessions).
SESSIONS=1
SESSIONS_NOTE=""
PROJECT=easi-run
DROPIN_DIR=/etc/systemd/system/docker.service.d
DROPIN_FILE=$DROPIN_DIR/easi-run-firewall.conf
# The rule, as iptables' raw table takes it after -D PREROUTING (delete) or -I PREROUTING 1 (insert).
RULE="-i er-+ -m addrtype --dst-type LOCAL -j DROP"
# The TCP settings, the files that keep them, and where the kernel lists its congestion controls.
SYSCTL_FILE=/etc/sysctl.d/90-easi-run.conf
MODULES_FILE=/etc/modules-load.d/easi-run.conf
TCP_SETTINGS=(net.core.default_qdisc=fq net.ipv4.tcp_congestion_control=bbr net.ipv4.tcp_slow_start_after_idle=0)
CONGESTION_CONTROLS=/proc/sys/net/ipv4/tcp_available_congestion_control
# The root steps: the image whose nsenter runs them (the Debian the Dockerfile's Chrome stage builds from), by digest;
# the PATH they run with on this server; and where they find iptables, the first of these there (never one a PATH
# finds).
ROOT_IMAGE=docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
ROOT_PATH=/usr/sbin:/usr/bin:/sbin:/bin
IPTABLES_PATHS=(/usr/sbin/iptables /usr/bin/iptables /sbin/iptables /bin/iptables)
# Where viewers reach the admin's UDP port (https://address:port), once known, and whether .env says.
UDP_URL=""
UDP_FROM_ENV=""
DOCKER_SOCKET=/var/run/docker.sock
CONTAINERD_ROOT=/var/lib/containerd
CONTAINERD_CONFIG=/etc/containerd/config.toml
# The images the controller starts sessions from, which it looks up once, when it starts.
SESSION_IMAGES=(easi-run/display:dev easi-run/browser:dev easi-run/agent:dev easi-run/proxy:dev)
MEMINFO=/proc/meminfo
# 4 GB and 10 GB, in KiB.
MIN_MEMORY_KB=3906250
MIN_DISK_KB=9765625
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

# Docker Compose, for this checkout and nothing else: its project name, Compose files and port, whatever the shell or
# an environment file sets (only the checkout's own .env is read, and these flags win over it). The files are
# compose.yaml, then the checkout's own compose.override.yaml when there is one, as Compose itself would read it. Builds
# carry no attestation, so an image that did not change keeps its id.
compose() {
  local files=(--file "$DIR/compose.yaml") udp=()
  [ ! -f "$DIR/compose.override.yaml" ] || files+=(--file "$DIR/compose.override.yaml")
  # Where viewers reach the UDP port, once known (otherwise whatever .env says), and the port published on every address
  # whenever the admin answers there, unless .env says otherwise.
  [ -z "$UDP_URL" ] || udp=(ADMIN_UDP_URL="$UDP_URL")
  [ -z "$UDP_URL$UDP_FROM_ENV" ] || env_sets ADMIN_UDP_BIND || udp+=(ADMIN_UDP_BIND=0.0.0.0)
  env -u COMPOSE_PROJECT_NAME -u COMPOSE_FILE -u COMPOSE_PROFILES -u COMPOSE_ENV_FILES -u COMPOSE_PATH_SEPARATOR \
    ADMIN_PORT="$PORT" ADMIN_MAX_SESSIONS="$SESSIONS" DOCKER_GID="$DOCKER_GID" BUILDX_NO_DEFAULT_ATTESTATIONS=1 \
    "${udp[@]}" docker compose --project-directory "$DIR" --project-name "$PROJECT" "${files[@]}" "$@"
}

preflight_host() {
  [ "$(id -u)" != 0 ] || die "run it as the account that will own easi.run, not as root: it needs no sudo, only the docker group"
  if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
    die "easi.run needs Linux on x86-64; this is $(uname -s) on $(uname -m)"
  fi
  has_systemd || die "this installer needs systemd, which keeps the firewall rule in place before Docker starts"
  local tool
  for tool in git curl docker ss systemctl awk stat df timeout flock; do
    command -v "$tool" >/dev/null || die "$tool is missing: install it, then run install.sh again"
  done
  case "$DIR" in /*) ;; *) die "EASI_RUN_DIR must be an absolute path: $DIR" ;; esac
  local memory
  memory="$(memory_kb)"
  [ "${memory:-0}" -ge "$MIN_MEMORY_KB" ] || die "easi.run needs 4 GB of memory; this server has $((${memory:-0} / 1000)) MB"
}

# This server's own Docker, answering: its version in DOCKER_VERSION.
docker_endpoint() {
  if [ -n "${DOCKER_HOST:-}" ] && [ "$DOCKER_HOST" != "unix://$DOCKER_SOCKET" ]; then
    die "DOCKER_HOST is $DOCKER_HOST: easi.run runs on this server's own Docker ($DOCKER_SOCKET); unset it"
  fi
  local endpoint
  endpoint="$(docker context inspect --format '{{.Endpoints.docker.Host}}' 2>/dev/null)" ||
    die "docker context inspect failed: is the Docker CLI installed whole?"
  [ "$endpoint" = "unix://$DOCKER_SOCKET" ] ||
    die "the Docker context reaches $endpoint, not $DOCKER_SOCKET: run docker context use default"
  DOCKER_VERSION="$(docker version --format '{{.Server.Version}}' 2>/dev/null)" ||
    die "Docker does not answer: is it running, and is $(id -un) in the docker group (sudo usermod -aG docker $(id -un), then log in again)?"
}

preflight_docker() {
  docker_endpoint
  major_at_least "$DOCKER_VERSION" 28 || die "easi.run needs Docker Engine 28 or later; this one is $DOCKER_VERSION"
  # The firewall rule comes with docker.service's drop-in, so the Docker that answers must be the one it starts.
  if [ "$(systemctl show docker.service --property LoadState --value 2>/dev/null)" != loaded ] ||
    ! systemctl is-active --quiet docker.service; then
    die "the Docker that answers is not run by docker.service (Docker's snap package, say), which keeps the firewall rule in place: install Docker Engine from docs.docker.com, then run install.sh again"
  fi
  local info
  info="$(docker info --format '{{.OperatingSystem}}|{{json .SecurityOptions}}|{{.DockerRootDir}}')" ||
    die "docker info failed"
  case "$info" in *"Docker Desktop"*) die "Docker Desktop is not supported: use Docker Engine on Linux" ;; esac
  case "$info" in *rootless*) die "rootless Docker is not supported: easi.run needs the system's rootful Docker Engine" ;; esac
  DOCKER_ROOT="${info##*|}"
  local compose_version
  compose_version="$(docker compose version --short 2>/dev/null)" || die "Docker Compose v2 (the docker compose plugin) is missing"
  major_at_least "$compose_version" 2 || die "easi.run needs Docker Compose v2; this one is $compose_version"
  docker buildx version >/dev/null 2>&1 || die "Docker Buildx (the docker buildx plugin), which adds Chrome to the browser image, is missing"
  DOCKER_GID="$(stat -c %g "$DOCKER_SOCKET")"
  # Docker's data root, and containerd's when Docker keeps its images in the containerd store.
  local roots=("$DOCKER_ROOT") root free
  # (A containerd run by dockerd itself keeps its root under Docker's, checked already.)
  root="$(containerd_root)"
  case "$(docker info --format '{{json .DriverStatus}}')" in *io.containerd.snapshotter*) [ ! -d "$root" ] || roots+=("$root") ;; esac
  for root in "${roots[@]}"; do
    free="$(df -Pk "$root" | awk 'NR == 2 {print $4}')" || die "cannot measure the free space in $root"
    [ "${free:-0}" -ge "$MIN_DISK_KB" ] ||
      die "easi.run needs 10 GB free where Docker keeps its images ($root); $((${free:-0} / 1000000)) GB are"
  done
}

# A whole number set in the checkout's .env, read as Compose reads it there (in quotes or not, a comment after a space),
# within $2 to $3: printed, or nothing when .env does not set it or leaves it empty (Compose's default). $4 is an
# example for the error.
env_number() {
  local name="$1" low="$2" high="$3" example="$4" line value
  local assignment="^[[:space:]]*(export[[:space:]]+)?${name}[[:space:]]*=[[:space:]]*(\"([^\"]*)\"|'([^']*)'|([^[:space:]\"'#][^[:space:]]*))?([[:space:]]+#.*)?[[:space:]]*\$"
  line="$(grep -E "^[[:space:]]*(export[[:space:]]+)?${name}[[:space:]]*=" "$DIR/.env" 2>/dev/null | tail -n 1)" || true
  [ -n "$line" ] || return 0
  [[ "$line" =~ $assignment ]] || die "$name in $DIR/.env is not a number from $low to $high such as $example: $line"
  value="${BASH_REMATCH[3]}${BASH_REMATCH[4]}${BASH_REMATCH[5]}"
  [ -n "$value" ] || return 0
  if [[ ! "$value" =~ ^[1-9][0-9]{0,5}$ ]] || [ "$value" -lt "$low" ] || [ "$value" -gt "$high" ]; then
    die "$name in $DIR/.env is not a number from $low to $high such as $example: $line"
  fi
  printf '%s\n' "$value"
}

# The admin's port: ADMIN_PORT in the checkout's .env, or 9443.
admin_port() {
  local value
  value="$(env_number ADMIN_PORT 1 65535 ADMIN_PORT=9444)"
  [ -z "$value" ] || PORT="$value"
}

# Browser sessions at once: ADMIN_MAX_SESSIONS in the checkout's .env, or as many as this server's memory holds, from
# 1 to 16. A session may use its browser's memory (BROWSER_MEMORY_MIB, 2048 MiB unless .env sets it) and 1152 MiB
# beside it (its display, agent and proxy), and 1 GiB stays for the system and the admin.
max_sessions() {
  local value browser memory
  value="$(env_number ADMIN_MAX_SESSIONS 1 16 ADMIN_MAX_SESSIONS=2)"
  if [ -n "$value" ]; then
    SESSIONS="$value"
    SESSIONS_NOTE="set in $DIR/.env"
    return 0
  fi
  browser="$(env_number BROWSER_MEMORY_MIB 1024 65536 BROWSER_MEMORY_MIB=2048)"
  memory="$(memory_kb)"
  SESSIONS=$(((${memory:-0} / 1024 - 1024) / (${browser:-2048} + 1152)))
  [ "$SESSIONS" -ge 1 ] || SESSIONS=1
  [ "$SESSIONS" -le 16 ] || SESSIONS=16
  SESSIONS_NOTE="from this server's memory; set ADMIN_MAX_SESSIONS in $DIR/.env to change it"
}

# The admin's port free, TCP and UDP, or published by this checkout's own admin.
preflight_port() {
  local proto ours
  for proto in tcp udp; do
    [ -n "$(ss -Hl --"$proto" -n "sport = :$PORT")" ] || continue
    ours="$(docker ps -q --filter "label=com.docker.compose.project=$PROJECT" \
      --filter label=com.docker.compose.service=admin \
      --filter "label=com.docker.compose.project.working_dir=$DIR" --filter "publish=$PORT/$proto")"
    [ -n "$ours" ] || die "port $PORT is in use by something other than this installation: free it, then run install.sh again"
  done
}

# The release this file names, whole: a commit, and one image digest for each role, in ROLES' order (the browser's
# last). Each image's reference, in IMAGE_REFS; the browser's, which Chrome is added to, in BROWSER_BASE.
resolve_target() {
  local i
  TARGET_COMMIT="$PINNED_COMMIT"
  case "$TARGET_COMMIT" in *[!0-9a-f]* | '') die "this install.sh names no commit to install: download it again from https://easi.run/install.sh" ;; esac
  [ "${#TARGET_COMMIT}" = 40 ] || die "this install.sh names no commit to install: download it again from https://easi.run/install.sh"
  TARGET="release ${TARGET_COMMIT:0:12}"
  [ "${#PINNED_IMAGES[@]}" = "${#ROLES[@]}" ] ||
    die "this install.sh does not name an image for each of ${ROLES[*]}: download it again from https://easi.run/install.sh"
  IMAGE_REFS=()
  for i in "${!ROLES[@]}"; do
    [[ "${PINNED_IMAGES[$i]}" =~ ^${ROLES[$i]}@sha256:[0-9a-f]{64}$ ]] ||
      die "this install.sh names ${PINNED_IMAGES[$i]} where it should name ${ROLES[$i]}@sha256:<digest>: download it again from https://easi.run/install.sh"
    IMAGE_REFS+=("$REGISTRY-${PINNED_IMAGES[$i]}")
  done
  BROWSER_BASE="${IMAGE_REFS[${#IMAGE_REFS[@]} - 1]}"
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
    # An older copy of the installer (the checkout's own, say) names an earlier release: it never moves back to it.
    ! git -C "$DIR" merge-base --is-ancestor "$TARGET_COMMIT" "$current" 2>/dev/null ||
      die "$DIR is at ${current:0:12}, newer than $TARGET, which this install.sh installs: run the newest one (curl -fsSLO https://easi.run/install.sh && bash install.sh)"
    [ "$UPDATE" = 1 ] ||
      die "$DIR is at ${current:0:12}; this install.sh installs $TARGET. Run bash install.sh --update to move to it (live browser sessions end)"
  else
    git clone --quiet --no-checkout "$REPO" "$DIR"
  fi
  git -C "$DIR" fetch --quiet origin "$TARGET_COMMIT"
  git -C "$DIR" -c advice.detachedHead=false checkout --quiet --detach "$TARGET_COMMIT"
  [ "$(git -C "$DIR" rev-parse HEAD)" = "$TARGET_COMMIT" ] || die "the checkout is not at $TARGET_COMMIT"
}

# docker.service's drop-in, with fixed commands only: before every start of Docker, it puts the rule first unless it is
# first already, then fails unless it is. Docker appends its own rules to the chain, after it. It removes no copy of the
# rule: a copy behind others changes nothing, and stays there should the insertion fail (Docker's live restore keeps
# containers running across its restarts).
dropin_text() {
  local iptables="$1"
  cat <<EOF
# Installed by easi.run's install.sh. Keeps browser sessions off this server's own services: every time Docker starts,
# before it does, one rule goes first in the raw table's PREROUTING and drops whatever a session bridge (er-<session>)
# sends an address of this server's own, before Docker's port redirects, and Docker starts only once it is there.
# Restarting Docker puts it back after a firewall reload.
[Unit]
After=ufw.service firewalld.service netfilter-persistent.service iptables.service nftables.service

[Service]
ExecStartPre=/bin/sh -c 'test "\$\$($iptables -w -t raw -S PREROUTING 1)" = "-A PREROUTING $RULE" || $iptables -w -t raw -I PREROUTING 1 $RULE'
ExecStartPre=/bin/sh -c 'test "\$\$($iptables -w -t raw -S PREROUTING 1)" = "-A PREROUTING $RULE"'
EOF
}

# The modules file's text: tcp_bbr, under the line that says whose it is.
modules_text() {
  printf '%s\n%s\n' "# Installed by easi.run's install.sh, with $SYSCTL_FILE: bash install.sh --uninstall removes it." tcp_bbr
}

# The system's iptables, in IPTABLES: the first of IPTABLES_PATHS there, never one a PATH finds; empty when there is
# none.
system_iptables() {
  local path
  IPTABLES=""
  for path in "${IPTABLES_PATHS[@]}"; do
    if [ -x "$path" ]; then
      IPTABLES="$path"
      return 0
    fi
  done
}

# A POSIX shell's assignment of the value $2 to the variable $1, the value in single quotes.
assign() {
  local quote="'\\''"
  printf "%s='%s'\n" "$1" "${2//\'/$quote}"
}

# The script the root steps run on this server, in its own shell (POSIX, dash on Debian): what it works on, set from
# this installer's values, then what it does, by phase ($1): check (that it runs as root there), apply (the firewall
# rule and the TCP settings) or uninstall (both undone). It stops at the first thing that fails, saying why.
root_script() {
  assign IPTABLES "$IPTABLES"
  assign RULE "$RULE"
  assign DROPIN_DIR "$DROPIN_DIR"
  assign DROPIN_FILE "$DROPIN_FILE"
  assign DROPIN_TEXT "$(dropin_text "$IPTABLES")"
  assign SYSCTL_FILE "$SYSCTL_FILE"
  assign MODULES_FILE "$MODULES_FILE"
  assign MODULES_TEXT "$(modules_text)"
  assign CONGESTION_CONTROLS "$CONGESTION_CONTROLS"
  assign TCP_SETTINGS "${TCP_SETTINGS[*]}"
  cat <<'EOF'
# (With -f, nothing globs: the unquoted $RULE, $TCP_SETTINGS and $previous only split into their words.)
set -euf
umask 022
fail() {
  printf 'install.sh: %s\n' "$*" >&2
  exit 1
}
[ "$(id -u)" = 0 ] || fail "the root step does not run as root on this server"

# Writes $2 to the file $1 (mode 0644) unless it holds that already, through a file beside it (not named *.conf, so
# nothing reads it) renamed over it; succeeds only when it wrote.
place() {
  [ "$(cat "$1" 2>/dev/null || true)" != "$2" ] || return 1
  temp="$(mktemp "$(dirname "$1")/.easi-run.XXXXXX")" || fail "could not write $1"
  if printf '%s\n' "$2" >"$temp" && chmod 0644 "$temp" && mv -f "$temp" "$1"; then
    return 0
  fi
  rm -f "$temp"
  fail "could not write $1"
}

# systemd's configuration loaded again whenever its files changed since it last loaded them (it says so for
# docker.service), however that came about: so that one write then a failed reload, then a run again, still reloads.
# Any other answer than yes or no stops it.
reload() {
  need="$(systemctl show docker.service --property NeedDaemonReload --value)" ||
    fail "systemctl could not say whether docker.service needs a reload"
  case "$need" in
  no) ;;
  yes) systemctl daemon-reload || fail "systemctl daemon-reload failed" ;;
  *) fail "systemctl said \"$need\" to whether docker.service needs a reload" ;;
  esac
}

# Whether the rule is first in the raw table's PREROUTING, where nothing before it can let a packet past.
rule_first() { [ "$("$IPTABLES" -w -t raw -S PREROUTING 1 2>/dev/null)" = "-A PREROUTING $RULE" ]; }

# Whether the settings file, and the modules file, are the installer's own: as it writes them.
our_sysctl_file() { grep -qx "# Installed by easi.run's install.sh.*" "$SYSCTL_FILE" 2>/dev/null; }
our_modules_file() { [ "$(cat "$MODULES_FILE" 2>/dev/null || true)" = "$MODULES_TEXT" ]; }

# The settings file's text: the settings, and the values they replaced ($1), which --uninstall puts back.
sysctl_text() {
  printf '%s\n' \
    "# Installed by easi.run's install.sh, which also loads tcp_bbr at boot ($MODULES_FILE): TCP that paces what it sends (fq)" \
    "# and keeps its rate on lossy, long paths (BBR), and no slow start again after an idle second, for streams to viewers" \
    "# far from this server. bash install.sh --uninstall puts back the values these replaced:" "# previous: $1"
  printf '%s\n' $TCP_SETTINGS
}

# docker.service's drop-in, loaded, and the rule first now as well, for the Docker already running, which is never
# restarted here.
firewall() {
  [ -x "$IPTABLES" ] || fail "iptables is not at $IPTABLES on this server: install it, then run install.sh again"
  mkdir -p "$DROPIN_DIR" || fail "could not make $DROPIN_DIR"
  place "$DROPIN_FILE" "$DROPIN_TEXT" || true
  reload
  case "$(systemctl show docker.service --property ExecStartPre --value)" in
  *"$IPTABLES -w -t raw -I PREROUTING 1 $RULE"*) ;;
  *) fail "docker.service does not put the rule in place before it starts: $DROPIN_FILE did not load" ;;
  esac
  # Put first before any copy behind other rules goes, so that the rule is never away meanwhile: a copy left behind it
  # changes nothing, and --uninstall removes every copy.
  if ! rule_first; then
    "$IPTABLES" -w -t raw -I PREROUTING 1 $RULE ||
      fail "iptables could not add the rule (its raw table and addrtype match are needed): $IPTABLES -t raw -I PREROUTING 1 $RULE"
  fi
  rule_first || fail "the rule is not first in the raw table's PREROUTING: $IPTABLES -t raw -S PREROUTING"
}

# fq, BBR and no slow start after idle for this server's TCP, now and at every boot, when the kernel has BBR; without
# it, TCP stays as it is. The values replaced are kept in the file, from the first run on.
tcp_settings() {
  grep -qw bbr "$CONGESTION_CONTROLS" 2>/dev/null || modprobe tcp_bbr 2>/dev/null || true
  grep -qw bbr "$CONGESTION_CONTROLS" 2>/dev/null || return 0
  # A file of that name the installer did not write is someone else's: it stops rather than overwrite it.
  [ ! -e "$SYSCTL_FILE" ] || our_sysctl_file ||
    fail "$SYSCTL_FILE is there and not easi.run's: move it, then run install.sh again"
  [ ! -e "$MODULES_FILE" ] || our_modules_file ||
    fail "$MODULES_FILE is there and not easi.run's: move it, then run install.sh again"
  previous=""
  ! our_sysctl_file || previous="$(sed -n 's/^# previous: //p' "$SYSCTL_FILE")"
  if [ -z "$previous" ]; then
    for setting in $TCP_SETTINGS; do
      value="$(sysctl -n "${setting%%=*}")" || fail "sysctl could not read ${setting%%=*}"
      previous="$previous ${setting%%=*}=$value"
    done
    previous="${previous# }"
  fi
  mkdir -p "$(dirname "$SYSCTL_FILE")" "$(dirname "$MODULES_FILE")" ||
    fail "could not make $(dirname "$SYSCTL_FILE") and $(dirname "$MODULES_FILE")"
  place "$MODULES_FILE" "$MODULES_TEXT" || true
  place "$SYSCTL_FILE" "$(sysctl_text "$previous")" || true
  sysctl -q -w $TCP_SETTINGS >/dev/null || fail "sysctl could not set $TCP_SETTINGS"
}

# What the installer put in place, removed: the drop-in, every copy of the rule (when there is an iptables; then it
# checks none is left), and the installer's own TCP files, with the values before them put back. A file of those names
# it did not write stays.
uninstall() {
  if [ -e "$DROPIN_FILE" ]; then
    rm -f "$DROPIN_FILE" || fail "could not remove $DROPIN_FILE"
    rmdir --ignore-fail-on-non-empty "$DROPIN_DIR" || fail "could not remove $DROPIN_DIR"
  fi
  reload
  if [ -n "$IPTABLES" ]; then
    [ -x "$IPTABLES" ] || fail "iptables is not at $IPTABLES on this server"
    while "$IPTABLES" -w -t raw -D PREROUTING $RULE 2>/dev/null; do :; done
    rules="$("$IPTABLES" -w -t raw -S PREROUTING)" ||
      fail "iptables could not list the raw table's PREROUTING, so whether the rule is gone is unknown"
    ! printf '%s\n' "$rules" | grep -qxF -- "-A PREROUTING $RULE" ||
      fail "the rule is still in the raw table's PREROUTING: $IPTABLES -t raw -S PREROUTING"
  fi
  printf '\n==> %s\n' "Putting back the TCP settings"
  if our_sysctl_file; then
    previous="$(sed -n 's/^# previous: //p' "$SYSCTL_FILE")"
    [ -z "$previous" ] || sysctl -q -w $previous >/dev/null || fail "sysctl could not put back $previous"
    rm -f "$SYSCTL_FILE" || fail "could not remove $SYSCTL_FILE"
  fi
  if our_modules_file; then
    rm -f "$MODULES_FILE" || fail "could not remove $MODULES_FILE"
  fi
}

case "${1:-}" in
check) ;;
apply)
  firewall
  printf '\n==> %s\n' "Setting this server's TCP for streams far away"
  tcp_settings
  ;;
uninstall) uninstall ;;
*) fail "no root step named ${1:-}" ;;
esac
EOF
}

# The image the root steps run in, pulled by its digest unless Docker has it already; $1 says what stopping leaves.
root_image() {
  docker image inspect "$ROOT_IMAGE" >/dev/null 2>&1 || pull_image "$ROOT_IMAGE" ||
    die "could not pull $ROOT_IMAGE, which runs the root steps (the lines above say why): $1; run install.sh again"
}

# Runs the root script's phase $1 as root on this server: in a short-lived privileged container of ROOT_IMAGE (in the
# server's PID namespace, its user namespace even under userns-remap, and with no network of its own), whose nsenter
# enters the mount, UTS, IPC and network namespaces of the server's init, so that the server's own shell and commands
# run, with a clean environment and ROOT_PATH. It stops the installer unless the phase ends well; when Docker itself
# refused the container (125), it says what Docker must allow.
as_root() {
  local script status=0 hint=""
  script="$(root_script)" || die "could not write the root step ($1)"
  docker run --rm -i --pull=never --privileged --userns=host --pid=host --network=none "$ROOT_IMAGE" \
    nsenter -t 1 -m -u -i -n -- /usr/bin/env -i HOME=/root USER=root LOGNAME=root LC_ALL=C PATH="$ROOT_PATH" \
    /bin/sh -s -- "$1" <<<"$script" || status=$?
  [ "$status" != 125 ] ||
    hint=": Docker must let $(id -un) run a privileged container in this server's PID and user namespaces"
  [ "$status" = 0 ] || die "the root step ($1), run through Docker, stopped with status $status (the lines above say why)$hint"
}

firewall() {
  system_iptables
  [ -n "$IPTABLES" ] || die "iptables is missing: install it, then run install.sh again"
  root_image "easi.run's containers are as they were"
  as_root apply
}

# Whether the checkout's .env gives the variable $1 a value.
env_sets() {
  grep -qE "^[[:space:]]*(export[[:space:]]+)?$1[[:space:]]*=[[:space:]]*[^[:space:]#]" "$DIR/.env" 2>/dev/null
}

# Where viewers reach the admin's UDP port, in UDP_URL: none when the checkout's .env sets ADMIN_UDP_URL (Compose reads
# it there; UDP_FROM_ENV says so); else the server's IPv4 address of the SSH session this runs in, when it is one a
# person reaches from elsewhere (as for the tunnel line), kept for later runs; else the one an earlier run kept; empty
# when none says.
udp_address() {
  local client client_port address port saved="$CONFIG/udp-url"
  UDP_URL="" UDP_FROM_ENV=""
  if env_sets ADMIN_UDP_URL; then
    UDP_FROM_ENV=1
    return 0
  fi
  if [[ "${SSH_CONNECTION:-}" =~ ^[0-9A-Fa-f:.]+\ [0-9]+\ [0-9A-Fa-f:.]+\ [0-9]+$ ]]; then
    read -r client client_port address port <<<"$SSH_CONNECTION"
    # IPv4 only: Docker publishes the port on 0.0.0.0, which an IPv6 address does not reach.
    if ipv4 "$address" && reachable_address "$address" && [ "$client" != "$address" ]; then
      UDP_URL="https://$address:$PORT"
      printf '%s\n' "$UDP_URL" >"$saved"
      return 0
    fi
  fi
  UDP_URL="$(cat "$saved" 2>/dev/null || true)"
  [[ "$UDP_URL" =~ ^https://[0-9.]+:$PORT$ ]] || UDP_URL=""
}

config_dir() {
  [ ! -L "$CONFIG" ] || die "$CONFIG is a symbolic link: easi.run keeps its files only in a real directory"
  (umask 077 && mkdir -p "$CONFIG")
  [ "$(stat -c %u "$CONFIG")" = "$(id -u)" ] || die "$CONFIG belongs to another user"
  chmod 0700 "$CONFIG"
}

# One run at a time: this one holds the lock in the configuration directory until it ends.
lock() {
  config_dir
  exec 9>"$CONFIG/lock"
  flock -n 9 || die "another run of install.sh is under way on this server: let it end, then run this again"
}

# The controller's and the admin's states and restart counts, one line each.
service_states() {
  local service
  for service in controller admin; do
    docker inspect --format '{{.State.Status}} {{.RestartCount}}' "$(compose ps -aq "$service")" 2>/dev/null || echo none
  done
}

# Pulls the release's images, each by its digest, which Docker checks what it receives against; one already here by
# that digest is that image, and is not asked for again. A pull that stops is tried twice more (a busy network); a
# third failure stops the installer with easi.run's containers as they were.
pull() {
  local ref
  for ref in "${IMAGE_REFS[@]}"; do
    docker image inspect "$ref" >/dev/null 2>&1 || pull_image "$ref" ||
      die "could not pull $ref (the lines above say why): easi.run's containers are as they were; run bash install.sh again"
  done
}

# Pulls the image $1, by the digest it names, trying twice more when a pull stops; fails after the third.
pull_image() {
  local try
  for try in 1 2 3; do
    docker pull --quiet "$1" >/dev/null && return 0
    [ "$try" = 3 ] || say "Pulling $1 stopped; trying again ($((try + 1)) of 3)."
  done
  return 1
}

# Adds Google's current Chrome stable to the pulled browser image, as its Dockerfile's browser target does, in Docker's
# own builder and with no build context: nothing of the checkout is sent or compiled, so a step that would need it
# fails. Google's terms let no one else ship Chrome. The new image, labelled as the installer's own, stays untagged,
# its id in BROWSER_ID, until every image is ready. A build that stops is tried twice more, reusing what the last try
# built.
chrome() {
  local try iid="$CONFIG/browser.iid"
  for try in 1 2 3; do
    rm -f "$iid"
    if BUILDX_NO_DEFAULT_ATTESTATIONS=1 docker buildx build --builder default --target browser \
      --build-arg "BROWSER_BASE=$BROWSER_BASE" --label "$BROWSER_LABEL" --iidfile "$iid" - <"$DIR/Dockerfile"; then
      BROWSER_ID="$(cat "$iid")"
      rm -f "$iid"
      return 0
    fi
    [ "$try" = 3 ] || say "Adding Chrome stopped; trying again ($((try + 1)) of 3), from where it stopped."
  done
  die "Chrome could not be added to the browser image (the lines above say why): easi.run's containers are as they were; run bash install.sh again"
}

# The release's images, tagged as compose.yaml and the controller name them: the browser's is the one chrome built.
tag_images() {
  local i
  for i in "${!ROLES[@]}"; do
    [ "${ROLES[$i]}" = browser ] || docker tag "${IMAGE_REFS[$i]}" "easi-run/${ROLES[$i]}:dev"
  done
  docker tag "$BROWSER_ID" easi-run/browser:dev
}

# Every image of the release's registry repositories (this installation's: one per server), as repository@digest.
registry_images() {
  local role
  for role in "${ROLES[@]}"; do
    docker image ls --digests --format '{{.Repository}}@{{.Digest}}' "$REGISTRY-$role" | grep -E "^$REGISTRY-$role@sha256:[0-9a-f]{64}\$" || true
  done
}

# The browser images the installer built (it labels them), by id, one per line.
built_images() { docker image ls --quiet --no-trunc --filter "label=$BROWSER_LABEL"; }

# Once the release runs: removes the images earlier ones left, those of the registry repositories this release does not
# name and the browser images the installer built but this one. One that a container still uses stays, for a later
# run to remove, and the run says so.
prune_images() {
  local ref kept=""
  for ref in $(registry_images) $(built_images); do
    case " ${IMAGE_REFS[*]} $BROWSER_ID " in *" $ref "*) continue ;; esac
    docker image inspect "$ref" >/dev/null 2>&1 || continue
    docker image rm "$ref" >/dev/null 2>&1 || kept="$kept $ref"
  done
  [ -z "$kept" ] || say "Kept images of an earlier release, which a container still uses (the next run removes them):$kept"
}

# Up once the admin answers over HTTPS, checked against the certificate it made, copied out of its volume. It fails at
# once when the controller or the admin stops or restarts meanwhile: either exits on a setting it refuses (in .env, say),
# and its restart policy would only start it again.
wait_for_admin() {
  local cert="$CONFIG/tls-cert.pem" deadline=$((SECONDS + 180)) seen="" states
  until compose cp admin:/var/lib/admin/tls-cert.pem "$cert.new" >/dev/null 2>&1 && mv -f "$cert.new" "$cert" &&
    curl --noproxy '*' -fsS --max-time 5 --cacert "$cert" --resolve "localhost:$PORT:127.0.0.1" -o /dev/null \
      "https://localhost:$PORT/api/status" 2>/dev/null; do
    states="$(service_states)"
    if [ "$(printf '%s\n' "$states" | grep -cE '^running [0-9]+$')" != 2 ] || { [ -n "$seen" ] && [ "$states" != "$seen" ]; }; then
      compose logs --no-color --tail 5 controller admin >&2 || true
      die "the controller or the admin stopped (their last lines are above): docker compose logs controller admin, in $DIR"
    fi
    seen="$states"
    [ "$SECONDS" -lt "$deadline" ] ||
      die "the admin did not answer on https://localhost:$PORT within 3 minutes: docker compose logs controller admin, in $DIR"
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

# Whether the admin's MCP server answers an initialize through `admin mcp`.
mcp_answers() {
  local hello='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"install.sh","version":"1"}}}'
  local answer
  answer="$(printf '%s\n' "$hello" | timeout 30 docker exec -i "$CONTAINER" admin mcp 2>/dev/null)" || return 1
  case "$answer" in *'"protocolVersion"'*) return 0 ;; *) return 1 ;; esac
}

# Whether the admin's password is set, as its status answers over HTTPS (checked against its certificate, as in
# wait_for_admin): until it is, the admin's first screen creates it.
password_note() {
  local status
  status="$(curl --noproxy '*' -fsS --max-time 5 --cacert "$CONFIG/tls-cert.pem" --resolve "localhost:$PORT:127.0.0.1" \
    "https://localhost:$PORT/api/status" 2>/dev/null)" || status=""
  case "$status" in
  *'"enrolled":true'*) PASSWORD_NOTE="set already" ;;
  *'"enrolled":false'*) PASSWORD_NOTE="create it at https://localhost:$PORT (open it first: until then, no one can log in)" ;;
  *) PASSWORD_NOTE="unknown (the admin's status did not answer); unless one is set, create it at https://localhost:$PORT" ;;
  esac
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

# Whether $1 is an IPv4 address: four decimal parts of 0-255, none with a leading zero.
ipv4() {
  local IFS=. part parts
  [[ "$1" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ ]] || return 1
  read -ra parts <<<"$1"
  for part in "${parts[@]}"; do
    [[ "$part" =~ ^(0|[1-9][0-9]*)$ ]] && [ "$part" -le 255 ] || return 1
  done
}

# Whether $1 is an IPv6 address: eight groups of 1-4 hex digits, or fewer around one "::".
ipv6() {
  local h='[0-9A-Fa-f]{1,4}' IFS=: part parts groups=0
  [[ "$1" =~ ^$h(:$h){7}$ ]] && return 0
  [[ "$1" =~ ^($h(:$h)*)?::($h(:$h)*)?$ ]] || return 1
  read -ra parts <<<"$1"
  for part in "${parts[@]}"; do [ -z "$part" ] || groups=$((groups + 1)); done
  [ "$groups" -le 7 ]
}

# Whether $1 is a TCP port: 1-65535, without a leading zero.
tcp_port() { [[ "$1" =~ ^[1-9][0-9]{0,4}$ ]] && [ "$1" -le 65535 ]; }

# Whether the IPv4 address $1, as a number, is in the range $2 (address/length).
in_range() {
  local IFS=./ a b c d length
  read -r a b c d length <<<"$2"
  [ $(($1 >> (32 - length))) = $((((a << 24) | (b << 16) | (c << 8) | d) >> (32 - length))) ]
}

# Whether the address $1 can be the one a person reached this server at from elsewhere: an IPv4 address outside the
# special-purpose ranges below, or an IPv6 one in 2000::/3 outside 2001::/23, 2001:db8::/32 and 3fff::/20.
reachable_address() {
  local IFS=. a b c d number range first second rest
  if ipv4 "$1"; then
    read -r a b c d <<<"$1"
    number=$(((a << 24) | (b << 16) | (c << 8) | d))
    for range in 0.0.0.0/8 10.0.0.0/8 100.64.0.0/10 127.0.0.0/8 169.254.0.0/16 172.16.0.0/12 192.0.0.0/24 \
      192.0.2.0/24 192.88.99.0/24 192.168.0.0/16 198.18.0.0/15 198.51.100.0/24 203.0.113.0/24 224.0.0.0/3; do
      ! in_range "$number" "$range" || return 1
    done
    return 0
  fi
  ipv6 "$1" || return 1
  first="${1%%:*}" rest="${1#*:}"
  second="${rest%%:*}"
  [ "${#first}" = 4 ] || return 1
  first=$((16#$first)) second=$((16#${second:-0}))
  [ "$first" -ge $((0x2000)) ] && [ "$first" -le $((0x3fff)) ] &&
    { [ "$first" != $((0x2001)) ] || { [ "$second" -ge $((0x200)) ] && [ "$second" != $((0xdb8)) ]; }; } &&
    { [ "$first" != $((0x3fff)) ] || [ "$second" -ge $((0x1000)) ]; }
}

# The SSH login this session came in by, as ssh's arguments ("[-p PORT ]user@address"), from SSH_CONNECTION
# ("client_ip client_port server_ip server_port") and the account running this. Empty unless all of it is certain:
# outside an SSH session, when SSH_CONNECTION is anything but those four, when logname cannot tell the account logged
# in or it is not this one (sudo, su), or when either address is a private, link-local, loopback or other special one
# (behind NAT, the server's is not the one the person reached) or both are the same (a session from this server).
ssh_login() {
  local client client_port address port user name
  [[ "${SSH_CONNECTION:-}" =~ ^[0-9A-Fa-f:.]+\ [0-9]+\ [0-9A-Fa-f:.]+\ [0-9]+$ ]] || return 0
  read -r client client_port address port <<<"$SSH_CONNECTION"
  reachable_address "$client" && tcp_port "$client_port" && reachable_address "$address" && tcp_port "$port" &&
    [ "$client" != "$address" ] || return 0
  user="$(id -un)"
  [[ "$user" =~ ^[A-Za-z_][A-Za-z0-9._-]{0,31}$ ]] || return 0
  [ -z "${SUDO_USER:-}" ] && [ "${USER:-$user}" = "$user" ] && [ "${LOGNAME:-$user}" = "$user" ] || return 0
  name="$(logname 2>/dev/null)" && [ "$name" = "$user" ] || return 0
  if [ "$port" = 22 ]; then
    printf '%s@%s\n' "$user" "$address"
  else
    printf -- '-p %s %s@%s\n' "$port" "$user" "$address"
  fi
}

summary() {
  local login hint=" (for example, as you connected now)"
  login="$(ssh_login)"
  [ -n "$login" ] || { login="you@your-server" hint=" (with your usual login)"; }
  say ""
  say "easi.run is running: $TARGET, in $DIR."
  say "  Admin:     https://localhost:$PORT (its certificate: $CONFIG/tls-cert.pem)"
  say "  Password:  $PASSWORD_NOTE"
  say "  Sessions:  $SESSIONS at once ($SESSIONS_NOTE)"
  say "  MCP:       $MCP_STATUS"
  say "  Codex CLI: codex mcp add easi -- docker exec -i $CONTAINER admin mcp"
  say "From another computer: use the command you already use to log in to this server, adding -N -L $PORT:127.0.0.1:$PORT."
  say "  Tunnel:    ssh -N -L $PORT:127.0.0.1:$PORT $login$hint, then open https://localhost:$PORT"
  say "  MCP:       claude mcp add --scope user easi -- ssh -T $login docker exec -i $CONTAINER admin mcp"
  say "  Editors:   VS Code and Cursor (Remote-SSH) forward the port for you: open the link from their terminal, or add port $PORT in their Ports view; either way, the local port must be $PORT too"
  [ -z "$UDP_URL$UDP_FROM_ENV" ] ||
    say "Smoother on lossy networks: if your provider has a firewall for this server, let $PORT/udp through it."
}

dry_run() {
  say "Checks passed. Without --dry-run, this would:"
  if [ -e "$DIR" ]; then
    say "  - use the checkout in $DIR, moved to $TARGET only with --update"
  else
    say "  - clone $REPO into $DIR at $TARGET"
  fi
  say "  - as root on this server, through a short-lived privileged container of this image (pulled by its digest"
  say "    unless Docker has it), whose nsenter enters this server's namespaces:"
  say "      $ROOT_IMAGE"
  say "    - install $DROPIN_FILE, and put the firewall rule in place"
  say "    - when the kernel has BBR: set this server's TCP to fq, BBR and no slow start after idle, in"
  say "      $SYSCTL_FILE and $MODULES_FILE, which keep the values they replace"
  say "  - pull these images, each by its digest:"
  local ref
  for ref in "${IMAGE_REFS[@]}"; do say "      $ref"; done
  say "  - add Google's current Chrome stable to the browser image, from Google's signed repository"
  say "  - start the stack in $DIR, and wait for https://localhost:$PORT"
  say "  - say whether the admin's password is set, and where you create it if not"
  say "  - add the MCP server easi to Claude Code, if claude is installed"
}

# Docker's resources of one kind (container, network or volume) that carry this label.
labelled() {
  case "$1" in
  container) docker ps -aq --filter "label=$2" ;;
  *) docker "$1" ls -q --filter "label=$2" ;;
  esac
}

# The controller's instance, which labels each of its sessions' containers, networks and volumes: CONTROLLER_INSTANCE
# in its environment (an override file may set it), or its default, also when there is no controller. When Docker
# cannot say, the installer stops before it removes anything.
controller_instance() {
  local environment="" ids
  if ! environment="$(docker inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$PROJECT-controller-1" \
    2>/dev/null)"; then
    ids="$(docker ps -aq --filter "name=^/$PROJECT-controller-1\$")" ||
      die "docker ps failed, so the instance easi.run's sessions carry is unknown: nothing is removed; run install.sh --uninstall again"
    [ -z "$ids" ] ||
      die "docker inspect $PROJECT-controller-1 failed, so the instance its sessions carry is unknown: nothing is removed; run install.sh --uninstall again"
  fi
  environment="$(sed -n 's/^CONTROLLER_INSTANCE=//p' <<<"$environment")"
  printf '%s\n' "${environment:-easirun}"
}

# Whether another easi.run installation is on this server, which the firewall rule protects too: a controller (its
# environment names the session images) or a session of any instance. When Docker cannot say, the installer stops, and
# the rule and the TCP settings stay.
another_installation() {
  local ids found unknown="so whether another easi.run installation here needs the firewall rule and the TCP settings is unknown: they stay; run install.sh --uninstall again"
  ids="$(docker ps -aq)" || die "docker ps failed, $unknown"
  [ -n "$ids" ] || return 1
  # A container that goes meanwhile fails the inspection: then once more, of the containers still there.
  # shellcheck disable=SC2086
  if ! found="$(docker inspect --format "$INSTALLATION_FORMAT" $ids 2>/dev/null)"; then
    ids="$(docker ps -aq)" || die "docker ps failed, $unknown"
    [ -n "$ids" ] || return 1
    # shellcheck disable=SC2086
    found="$(docker inspect --format "$INSTALLATION_FORMAT" $ids)" || die "docker inspect failed, $unknown"
  fi
  grep -qE '^[^|]+[|]|IMAGE_BROWSER=' <<<"$found"
}
# What another_installation reads of each container: its instance label, and its environment.
INSTALLATION_FORMAT='{{index .Config.Labels "run.easi.instance"}}|{{range .Config.Env}}{{.}} {{end}}'

# Removes what the installer put on this server, after asking, and says what it keeps.
uninstall() {
  local yes="$1" answer kind label ids force image instance mcp="" rule=""
  say "This removes easi.run from this server:"
  say "  - its containers, with every browser session, and its volumes, networks and images: the admin's password,"
  say "    agent tokens and certificate go with its state volume"
  say "  - $DROPIN_FILE and the firewall rule, and $SYSCTL_FILE and $MODULES_FILE with the TCP"
  say "    settings they made (putting back the values before them), unless another easi.run installation here needs them:"
  say "    as root on this server, through a short-lived privileged container of $ROOT_IMAGE"
  say "  - $CONFIG"
  say "  - the MCP server easi in Claude Code, if it is this installation's"
  say "It keeps $DIR, Docker's build cache and $ROOT_IMAGE."
  if [ "$yes" != yes ]; then
    [ -t 0 ] || die "--uninstall asks before it removes anything, and there is no terminal to ask in: run it in a terminal, or add --yes"
    read -r -p "Type yes to remove easi.run: " answer
    [ "$answer" = yes ] || die "nothing removed"
  fi
  lock
  system_iptables
  # Whatever the root steps need, before anything is removed.
  root_image "nothing is removed"
  as_root check
  step "Removing the containers, volumes, networks and images"
  instance="$(controller_instance)"
  # The stack first, so nothing starts a session meanwhile; then the sessions' own containers, networks and volumes,
  # all before the rule that keeps their pages off this server goes.
  for kind in container network volume; do
    force=()
    [ "$kind" != container ] || force=(-f)
    for label in "com.docker.compose.project=$PROJECT" "run.easi.instance=$instance"; do
      ids="$(labelled "$kind" "$label")"
      if [ -n "$ids" ]; then
        # shellcheck disable=SC2086
        docker "$kind" rm "${force[@]}" $ids >/dev/null
      fi
    done
  done
  for image in easi-run/admin:dev easi-run/controller:dev "${SESSION_IMAGES[@]}" $(registry_images) $(built_images); do
    if docker image inspect "$image" >/dev/null 2>&1; then
      docker image rm "$image" >/dev/null ||
        die "docker image rm $image failed: remove the container that uses it, then run install.sh --uninstall again"
    fi
  done
  step "Removing the firewall rule"
  if another_installation; then
    rule="  Kept: the firewall rule and $DROPIN_FILE, which another easi.run installation here still needs"
    rule="$rule; and the TCP settings of $SYSCTL_FILE"
  else
    # (It says when it puts back the TCP settings.)
    as_root uninstall
    [ -n "$IPTABLES" ] ||
      rule="  Kept: the firewall rule, if one is there: this server has no iptables in ${IPTABLES_PATHS[*]} to remove it"
  fi
  rm -rf "$CONFIG"
  if command -v claude >/dev/null && answer="$(claude mcp get easi </dev/null 2>/dev/null)"; then
    case "$answer" in
    *"Command: docker"*"Args: exec -i $PROJECT-admin-1 admin mcp"*)
      claude mcp remove --scope user easi </dev/null >/dev/null || die "claude mcp remove --scope user easi failed"
      ;;
    *) mcp="  Kept: Claude Code's server easi, which is not this installation's" ;;
    esac
  fi
  say ""
  say "easi.run is removed from this server."
  say "  Kept: $DIR, the checkout, with any .env or compose.override.yaml you put there: rm -rf $DIR"
  say "  Kept: Docker's build cache, which docker builder prune removes, for every project on this server"
  say "  Kept: $ROOT_IMAGE, which ran the root steps (another project may use it): docker image rm $ROOT_IMAGE"
  [ -z "$rule" ] || say "$rule"
  [ -z "$mcp" ] || say "$mcp"
  say "  Agents on other computers keep their easi server: claude mcp remove --scope user easi, there"
}

main() {
  UPDATE=0
  local dry=0 remove=""
  case "$#:${1:-}:${2:-}" in
  0::) ;;
  1:--update:) UPDATE=1 ;;
  1:--dry-run:) dry=1 ;;
  1:--uninstall:) remove=ask ;;
  2:--uninstall:--yes) remove=yes ;;
  *) die "usage: bash install.sh [--update | --dry-run | --uninstall [--yes]]" ;;
  esac
  if [ -n "$remove" ]; then
    [ "$(id -u)" != 0 ] || die "run it as the account that owns easi.run, not as root"
    docker_endpoint
    uninstall "$remove"
    return 0
  fi
  step "Checking this server"
  preflight_host
  admin_port
  max_sessions
  preflight_docker
  preflight_port
  resolve_target
  if [ "$dry" = 1 ]; then
    dry_run
    return 0
  fi
  lock
  step "Getting easi.run ($TARGET)"
  checkout
  step "Putting the firewall rule in place"
  firewall
  udp_address
  if [ "$UPDATE" = 1 ]; then
    say "--update: if the admin or a session image changed, live browser sessions end."
  fi
  step "Pulling the images"
  pull
  step "Adding Google Chrome to the browser image"
  chrome
  step "Starting easi.run"
  local controller images before
  controller="$(compose ps -q controller)"
  tag_images
  compose up -d --no-build --pull never
  # The controller starts sessions from the images it found when it started, so it restarts when one of them is not
  # the one it last started with. The admin restarts whenever the controller did, ending the sessions it no longer
  # holds.
  images="$(session_images)"
  before="$(cat "$CONFIG/images" 2>/dev/null || true)"
  if [ "$images" != "$before" ]; then
    [ -z "$before" ] ||
      say "A session image changed (a newer Chrome, say): the controller and the admin start afresh; live browser sessions end."
    compose up -d --no-build --pull never --no-deps --force-recreate controller admin
  elif [ "$(compose ps -q controller)" != "$controller" ]; then
    compose up -d --no-build --pull never --no-deps --force-recreate admin
  fi
  wait_for_admin
  printf '%s\n' "$images" >"$CONFIG/images"
  prune_images
  admin_container
  password_note
  step "Connecting MCP"
  connect_mcp
  summary
}

# Run, unless sourced (by the installer's tests).
if [ "${BASH_SOURCE[0]:-$0}" = "$0" ]; then
  main "$@"
fi
