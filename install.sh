#!/usr/bin/env bash
# easi.run's installer, for one Linux x86-64 server with Docker. Read it before you run it, then run it as the account
# that will own the installation, never as root:
#
#   curl -fsSLO https://easi.run/install.sh && less install.sh && bash install.sh
#
#   bash install.sh            installs easi.run, or checks again an installation already at the latest release (and
#                              takes Google's newest Chrome stable, if there is one: its live browser sessions end)
#   bash install.sh --update   moves an installation to the latest release: its live browser sessions end
#   bash install.sh --dry-run  runs the checks and says what it would do, changing nothing
#   bash install.sh --uninstall [--yes]
#                              removes easi.run from this server after asking (--yes: without asking), and says what
#                              it keeps: the checkout and Docker's build cache
#
# In order, it:
#   1. checks this server: Linux on x86-64 with systemd; rootful Docker Engine 28+ on this server's own socket, run by
#      docker.service, with Compose v2; git, curl, sudo and ss; 4 GB of memory; 20 GB free where Docker
#      keeps its images; the admin's port (9443, or ADMIN_PORT in the checkout's .env) free, or held by this
#      installation;
#   2. clones easi.run into ~/easi.run (EASI_RUN_DIR) at the latest release tag, or at the commit named below while
#      there is none;
#   3. with sudo, installs a drop-in for docker.service that puts the rule
#      `iptables -t raw -I PREROUTING 1 -i er-+ -m addrtype --dst-type LOCAL -j DROP` first every time Docker starts,
#      before it does: whatever a session bridge (er-<session>) sends an address of this server's own is dropped before
#      Docker's port redirects, the ports Docker publishes included. Docker, and every container it restarts, starts
#      only once the rule is there; and it puts the rule first now;
#   4. builds and starts the stack, with Google's current Chrome stable from Google's signed repository, and waits for
#      the admin over HTTPS, checked against the admin's own certificate;
#   5. while no password is set, prints the admin's one-time setup link (`admin setup`), which opens the screen
#      where you create the password;
#   6. adds the admin's MCP server to Claude Code (`claude mcp add --scope user easi`) when claude is installed and no
#      other `easi` server is there.
#
# It never sees the admin's password, which you create in the admin's own screen, and it runs nothing from the
# checkout as root.
set -euo pipefail

REPO="${EASI_RUN_REPO:-https://github.com/julianosig/easi.run}"
# The commit installed while the repository has no release tag (vMAJOR.MINOR.PATCH); a release tag always wins.
PINNED_COMMIT=105fc9ec24c9e14bd6deb00d8e68d3fa9fba2a14
DIR="${EASI_RUN_DIR:-$HOME/easi.run}"
CONFIG="$HOME/.config/easi-run"
PORT=9443
# Browser sessions at once, and where that number comes from (see max_sessions).
SESSIONS=1
SESSIONS_NOTE=""
PROJECT=easi-run
DROPIN_DIR=/etc/systemd/system/docker.service.d
DROPIN_FILE=$DROPIN_DIR/easi-run-firewall.conf
# The rule, as iptables' raw table takes it after -C (check) or -I PREROUTING 1 (insert), and as one line.
RULE_ARGS=(-i er-+ -m addrtype --dst-type LOCAL -j DROP)
RULE="${RULE_ARGS[*]}"
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
SETUP_LINK=""

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
# compose.yaml, then the checkout's own compose.override.yaml when there is one, as Compose itself would read it, and
# compose.egress.yaml when the checkout holds egress.json or its .env names an EGRESS_FILE. Builds carry no
# attestation, so an image that did not change keeps its id.
compose() {
  local files=(--file "$DIR/compose.yaml")
  [ ! -f "$DIR/compose.override.yaml" ] || files+=(--file "$DIR/compose.override.yaml")
  if [ -f "$DIR/egress.json" ] || grep -Eqs '^[[:space:]]*(export[[:space:]]+)?EGRESS_FILE[[:space:]]*=' "$DIR/.env"; then
    files+=(--file "$DIR/compose.egress.yaml")
  fi
  env -u COMPOSE_PROJECT_NAME -u COMPOSE_FILE -u COMPOSE_PROFILES -u COMPOSE_ENV_FILES -u COMPOSE_PATH_SEPARATOR \
    ADMIN_PORT="$PORT" ADMIN_MAX_SESSIONS="$SESSIONS" DOCKER_GID="$DOCKER_GID" BUILDX_NO_DEFAULT_ATTESTATIONS=1 \
    docker compose --project-directory "$DIR" --project-name "$PROJECT" "${files[@]}" "$@"
}

preflight_host() {
  [ "$(id -u)" != 0 ] || die "run it as the account that will own easi.run, not as root: it asks sudo for the firewall rule only"
  if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
    die "easi.run needs Linux on x86-64; this is $(uname -s) on $(uname -m)"
  fi
  has_systemd || die "this installer needs systemd, which keeps the firewall rule in place before Docker starts"
  local tool
  for tool in git curl sudo docker ss systemctl awk stat df timeout; do
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
  DOCKER_GID="$(stat -c %g "$DOCKER_SOCKET")"
  # Docker's data root, and containerd's when Docker keeps its images in the containerd store.
  local roots=("$DOCKER_ROOT") root free
  # (A containerd run by dockerd itself keeps its root under Docker's, checked already.)
  root="$(containerd_root)"
  case "$(docker info --format '{{json .DriverStatus}}')" in *io.containerd.snapshotter*) [ ! -d "$root" ] || roots+=("$root") ;; esac
  for root in "${roots[@]}"; do
    free="$(df -Pk "$root" | awk 'NR == 2 {print $4}')" || die "cannot measure the free space in $root"
    [ "${free:-0}" -ge "$MIN_DISK_KB" ] ||
      die "easi.run needs 20 GB free where Docker keeps its images ($root); $((${free:-0} / 1000000)) GB are"
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

# The admin's port free, or published by this checkout's own admin.
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
  say "sudo asks for your password once, for the firewall rule only."
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

# docker.service's drop-in, with fixed commands only: before every start of Docker, it moves the rule first unless it is
# first already, then fails unless it is. Docker appends its own rules to the chain, after it.
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
ExecStartPre=/bin/sh -c 'test "\$\$($iptables -w -t raw -S PREROUTING 1)" = "-A PREROUTING $RULE" || { $iptables -w -t raw -D PREROUTING $RULE 2>/dev/null; $iptables -w -t raw -I PREROUTING 1 $RULE; }'
ExecStartPre=/bin/sh -c 'test "\$\$($iptables -w -t raw -S PREROUTING 1)" = "-A PREROUTING $RULE"'
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

# Whether the rule is first in the raw table's PREROUTING, where nothing before it can let a packet past.
rule_first() { [ "$(sudo "$1" -w -t raw -S PREROUTING 1 2>/dev/null)" = "-A PREROUTING $RULE" ]; }

firewall() {
  local iptables pre
  system_iptables
  iptables="$IPTABLES"
  [ -n "$iptables" ] || die "iptables is missing: install it, then run install.sh again"
  sudo install -d -m 0755 -o root -g root "$DROPIN_DIR"
  if place "$DROPIN_FILE" "$(dropin_text "$iptables")"; then
    sudo systemctl daemon-reload
  fi
  pre="$(systemctl show docker.service --property ExecStartPre --value)"
  case "$pre" in *"$iptables -w -t raw -I PREROUTING 1 $RULE"*) ;;
  *) die "docker.service does not put the rule in place before it starts: $DROPIN_FILE did not load" ;;
  esac
  # Now as well, for the Docker already running, which is never restarted here.
  if ! rule_first "$iptables"; then
    sudo "$iptables" -w -t raw -D PREROUTING "${RULE_ARGS[@]}" 2>/dev/null || true
    sudo "$iptables" -w -t raw -I PREROUTING 1 "${RULE_ARGS[@]}" ||
      die "iptables could not add the rule (its raw table and addrtype match are needed): sudo $iptables -t raw -I PREROUTING 1 $RULE"
  fi
  rule_first "$iptables" || die "the rule is not first in the raw table's PREROUTING: sudo $iptables -t raw -S PREROUTING"
}

config_dir() {
  [ ! -L "$CONFIG" ] || die "$CONFIG is a symbolic link: easi.run keeps its files only in a real directory"
  (umask 077 && mkdir -p "$CONFIG")
  [ "$(stat -c %u "$CONFIG")" = "$(id -u)" ] || die "$CONFIG belongs to another user"
  chmod 0700 "$CONFIG"
}

# The controller's and the admin's states and restart counts, one line each.
service_states() {
  local service
  for service in controller admin; do
    docker inspect --format '{{.State.Status}} {{.RestartCount}}' "$(compose ps -aq "$service")" 2>/dev/null || echo none
  done
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

# The one-time link to the admin's first screen, where the person creates the password, while none is set.
setup_link() {
  local link status=0
  link="$(timeout 30 docker exec -i "$CONTAINER" admin setup </dev/null 2>/dev/null)" || status=$?
  case "$status:$link" in
  0:https://*'#setup='*)
    SETUP_LINK="$link"
    PASSWORD_NOTE="create it at $link (one use, valid for up to 15 minutes; to print it again: docker exec -i $CONTAINER admin setup)"
    ;;
  3:*) PASSWORD_NOTE="set already" ;;
  *) PASSWORD_NOTE="create it at the link this prints: docker exec -i $CONTAINER admin setup" ;;
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

summary() {
  local who
  who="$(id -un)@$(uname -n)"
  say ""
  say "easi.run is running: $TARGET, in $DIR."
  say "  Admin:     https://localhost:$PORT (its certificate: $CONFIG/tls-cert.pem)"
  say "  Password:  $PASSWORD_NOTE"
  say "  Sessions:  $SESSIONS at once ($SESSIONS_NOTE)"
  say "  MCP:       $MCP_STATUS"
  say "  Codex CLI: codex mcp add easi -- docker exec -i $CONTAINER admin mcp"
  say "From another computer (use this server's address in place of $(uname -n) if it does not resolve):"
  if [ -n "$SETUP_LINK" ]; then
    say "  Viewer:    ssh -N -L $PORT:127.0.0.1:$PORT $who, then open the setup link above"
  else
    say "  Viewer:    ssh -N -L $PORT:127.0.0.1:$PORT $who, then open https://localhost:$PORT"
  fi
  say "  MCP:       claude mcp add --scope user easi -- ssh -T $who docker exec -i $CONTAINER admin mcp"
}

dry_run() {
  say "Checks passed. Without --dry-run, this would:"
  if [ -e "$DIR" ]; then
    say "  - use the checkout in $DIR, moved to $TARGET only with --update"
  else
    say "  - clone $REPO into $DIR at $TARGET"
  fi
  say "  - install (with sudo) $DROPIN_FILE, and put the firewall rule in place"
  say "  - build and start the stack in $DIR, and wait for https://localhost:$PORT"
  say "  - print the one-time link where you create the admin's password, unless one is set"
  say "  - add the MCP server easi to Claude Code, if claude is installed"
}

# The iptables root finds, in IPTABLES (empty when there is none), only from the system's own directories.
system_iptables() {
  IPTABLES="$(sudo sh -c 'command -v iptables')" || IPTABLES=""
  case "$IPTABLES" in '' | /usr/sbin/iptables | /sbin/iptables | /usr/bin/iptables | /bin/iptables) ;;
  *) die "iptables is at $IPTABLES, outside the system's own directories" ;;
  esac
}

# Docker's resources of one kind (container, network or volume) that carry this label.
labelled() {
  case "$1" in
  container) docker ps -aq --filter "label=$2" ;;
  *) docker "$1" ls -q --filter "label=$2" ;;
  esac
}

# The controller's instance, which labels each of its sessions' containers, networks and volumes: CONTROLLER_INSTANCE
# in its environment (an override file may set it), or its default.
controller_instance() {
  local instance
  instance="$(docker inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$PROJECT-controller-1" 2>/dev/null |
    sed -n 's/^CONTROLLER_INSTANCE=//p')" || true
  printf '%s\n' "${instance:-easirun}"
}

# Whether another easi.run installation is on this server, which the firewall rule protects too: a controller (its
# environment names the session images) or a session of any instance.
another_installation() {
  local ids found
  ids="$(docker ps -aq)"
  [ -n "$ids" ] || return 1
  # (One that goes meanwhile fails the inspection, not the answer for the others.)
  # shellcheck disable=SC2086
  found="$(docker inspect --format '{{index .Config.Labels "run.easi.instance"}}|{{range .Config.Env}}{{.}} {{end}}' $ids \
    2>/dev/null)" || true
  grep -qE '^[^|]+[|]|IMAGE_BROWSER=' <<<"$found"
}

# Removes what the installer put on this server, after asking, and says what it keeps.
uninstall() {
  local yes="$1" answer kind label ids force image instance mcp="" rule=""
  say "This removes easi.run from this server:"
  say "  - its containers, with every browser session, and its volumes, networks and images: the admin's password,"
  say "    agent tokens and certificate go with its state volume"
  say "  - (with sudo) $DROPIN_FILE and the firewall rule, unless another easi.run installation here needs them"
  say "  - $CONFIG"
  say "  - the MCP server easi in Claude Code, if it is this installation's"
  say "It keeps $DIR and Docker's build cache."
  if [ "$yes" != yes ]; then
    [ -t 0 ] || die "--uninstall asks before it removes anything, and there is no terminal to ask in: run it in a terminal, or add --yes"
    read -r -p "Type yes to remove easi.run: " answer
    [ "$answer" = yes ] || die "nothing removed"
  fi
  preflight_sudo
  system_iptables
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
  for image in easi-run/admin:dev easi-run/controller:dev "${SESSION_IMAGES[@]}"; do
    if docker image inspect "$image" >/dev/null 2>&1; then
      docker image rm "$image" >/dev/null ||
        die "docker image rm $image failed: remove the container that uses it, then run install.sh --uninstall again"
    fi
  done
  step "Removing the firewall rule"
  if another_installation; then
    rule="  Kept: the firewall rule and $DROPIN_FILE, which another easi.run installation here still needs"
  elif sudo test -e "$DROPIN_FILE"; then
    sudo rm -f "$DROPIN_FILE"
    sudo rmdir --ignore-fail-on-non-empty "$DROPIN_DIR"
    sudo systemctl daemon-reload
  fi
  if [ -z "$rule" ] && [ -n "$IPTABLES" ]; then
    while sudo "$IPTABLES" -w -t raw -D PREROUTING "${RULE_ARGS[@]}" 2>/dev/null; do :; done
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
  say "  Kept: $DIR, the checkout, with any .env, compose.override.yaml or egress.json you put there: rm -rf $DIR"
  say "  Kept: Docker's build cache, which docker builder prune removes, for every project on this server"
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
  preflight_sudo
  step "Getting easi.run ($TARGET)"
  checkout
  config_dir
  step "Putting the firewall rule in place"
  firewall
  if [ "$UPDATE" = 1 ]; then
    say "--update: if the admin or a session image changed, live browser sessions end."
  fi
  step "Building the images (the first build takes a while)"
  compose build
  step "Starting easi.run"
  local controller images before
  controller="$(compose ps -q controller)"
  compose up -d
  # The controller starts sessions from the images it found when it started, so it restarts when one of them is not
  # the one it last started with. The admin restarts whenever the controller did, ending the sessions it no longer
  # holds.
  images="$(session_images)"
  before="$(cat "$CONFIG/images" 2>/dev/null || true)"
  if [ "$images" != "$before" ]; then
    [ -z "$before" ] ||
      say "A session image changed (a newer Chrome, say): the controller and the admin start afresh; live browser sessions end."
    compose up -d --no-deps --force-recreate controller admin
  elif [ "$(compose ps -q controller)" != "$controller" ]; then
    compose up -d --no-deps --force-recreate admin
  fi
  wait_for_admin
  printf '%s\n' "$images" >"$CONFIG/images"
  admin_container
  setup_link
  step "Connecting MCP"
  connect_mcp
  summary
}

# Run, unless sourced (by the installer's tests).
if [ "${BASH_SOURCE[0]:-$0}" = "$0" ]; then
  main "$@"
fi
