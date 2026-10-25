#!/usr/bin/env bash
# Tests install.sh without changing this machine: it sources the installer and runs it against stubs of sudo,
# systemctl, iptables, docker, ss, df, curl, claude, id and uname in PATH, a fake root for the systemd files, and a
# local git repository standing in for GitHub. Nothing here needs root, Docker or the network.
#   scripts/install-test.sh
# The stubs' bodies are single-quoted on purpose: they expand when they run.
# shellcheck disable=SC2016
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap '[ -n "${KEEP:-}" ] || rm -rf "$work"' EXIT
real_uid="$(id -u)"
base_path="$PATH"
failures=0

fail() {
  printf 'FAIL %s: %s\n' "$current" "$*" >&2
  failures=$((failures + 1))
}
check() { "$@" || fail "expected: $*"; }
contains() { grep -qF -- "$2" "$1" || fail "$1 lacks: $2"; }
lacks() { ! grep -qF -- "$2" "$1" || fail "$1 has: $2"; }

# The stubs, each logging its call to $LOG.
mkdir -p "$work/bin"
stub() {
  printf '#!/usr/bin/env bash\nset -eu\nprintf "%%s\\n" "$(basename "$0") $*" >>"$LOG"\n%s\n' "$2" >"$work/bin/$1"
  chmod +x "$work/bin/$1"
}
stub id 'case "$1" in -u) echo "${FAKE_UID}" ;; -un) echo tester ;; *) exec /usr/bin/id "$@" ;; esac'
stub uname 'case "$1" in -s) echo Linux ;; -m) echo "${FAKE_ARCH:-x86_64}" ;; -n) echo box ;; esac'
stub df 'case "$2" in "$WORK/containerd") kb="${FAKE_CONTAINERD_KB:-50000000}" ;; "$WORK/srv-containerd") kb="${FAKE_MOVED_KB:-50000000}" ;; *) kb="${FAKE_DISK_KB:-50000000}" ;; esac
printf "Filesystem 1024-blocks Used Available Capacity Mounted\n/dev/x 1 1 %s 1%% /\n" "$kb"'
stub ss 'if [ -n "${FAKE_PORT_BUSY:-}" ]; then echo "LISTEN 0 4096 127.0.0.1:9443 0.0.0.0:*"; fi'
# curl answers once the certificate is there, unless FAKE_ADMIN_DOWN.
stub curl '[ -z "${FAKE_ADMIN_DOWN:-}" ] || exit 7; for a in "$@"; do [ "${next:-}" = 1 ] && { [ -s "$a" ] && exit 0 || exit 7; }; [ "$a" = --cacert ] && next=1; done; exit 7'
stub claude 'case "$1 $2" in
  "mcp get") [ -f "$WORK/claude-easi" ] && cat "$WORK/claude-easi" || exit 1 ;;
  "mcp remove") rm -f "$WORK/claude-easi" ;;
  "mcp add") shift 5; printf "easi:\n  Command: %s\n  Args: %s\n" "$2" "${*:3}" >"$WORK/claude-easi" ;;
esac'
# iptables over $WORK/rules, the raw table's PREROUTING rules in order; FAKE_IPTABLES_STUCK ignores insertions.
stub iptables 'rules="$WORK/rules"; touch "$rules"; [ "$1" = -w ] && shift
[ "$1 $2" = "-t raw" ] || exit 2
shift 2
case "$1" in
  -D) shift 2; line="-A PREROUTING $*"; n="$(grep -nxF -- "$line" "$rules" | head -n 1 | cut -d: -f1)"
      [ -n "$n" ] || exit 1; sed -i "${n}d" "$rules" ;;
  -I) [ -n "${FAKE_IPTABLES_STUCK:-}" ] || { shift 3; { echo "-A PREROUTING $*"; cat "$rules"; } >"$rules.new"; mv "$rules.new" "$rules"; } ;;
  -S) if [ $# -ge 3 ]; then sed -n "${3}p" "$rules" | grep .; else cat "$rules"; fi ;;
esac'
# systemctl: docker.service's ExecStartPre lines are its drop-in's, as systemd shows them (\$\$ read as \$).
# FAKE_UNIT is docker.service's load state (loaded unless set), FAKE_INACTIVE stops it, and FAKE_NOT_LOADED keeps its
# drop-in from loading.
stub systemctl 'case "$1 ${4:-}" in
  "show LoadState") echo "${FAKE_UNIT:-loaded}" ;;
  "show ExecStartPre") f="$FAKEROOT/etc/systemd/system/docker.service.d/easi-run-firewall.conf"
        if [ -z "${FAKE_NOT_LOADED:-}" ] && [ -f "$f" ]; then sed -n "s/^$4=//p" "$f" | sed "s/\\$\\$/\\$/g"; fi ;;
  "is-active "*) [ -z "${FAKE_INACTIVE:-}" ] ;;
esac'
# sudo: FAKE_SUDO_PASSWORD makes it need a password, and FAKE_IPTABLES names where root finds iptables.
stub sudo 'case "$1" in
  -n) [ -z "${FAKE_SUDO_PASSWORD:-}" ] ;;
  -v) exit 0 ;;
  sh) echo "${FAKE_IPTABLES:-/usr/sbin/iptables}" ;;
  */iptables) shift; exec iptables "$@" ;;
  install) shift; args=(); while [ $# -gt 0 ]; do case "$1" in -o|-g) shift 2 ;; *) args+=("$1"); shift ;; esac; done
           exec install "${args[@]}" ;;
  *) exec "$@" ;;
esac'
# docker: FAKE_NETWORKS and FAKE_VOLUMES are what its listings find, FAKE_CONTROLLER_ENV is the controller's
# environment, FAKE_OTHERS what the other containers carry ("label|environment"), FAKE_IMAGE_IN_USE fails image rm; FAKE_CONTEXT is the context's endpoint; FAKE_OURS is this checkout's admin publishing port 9443, and
# FAKE_OTHER one that a query without both of those filters would find as well; a service is in FAKE_STATE ("running 0"
# unless set), or with FAKE_RESTARTS restarted once more at each look; Compose logs a variable of the caller's shell or
# a build with attestations.
stub docker 'case "$1" in
  context) echo "${FAKE_CONTEXT:-unix://$FAKE_SOCKET}" ;;
  version) echo "${FAKE_DOCKER_VERSION:-28.3.0}" ;;
  info) case "$*" in
      *DriverStatus*) echo "${FAKE_DRIVER:-[[\"driver-type\",\"io.containerd.snapshotter.v1\"]]}" ;;
      *) echo "${FAKE_OS:-Debian GNU/Linux 13 (trixie)}|${FAKE_SECURITY:-[\"name=seccomp,profile=builtin\"]}|$WORK" ;;
    esac ;;
  image) case "$2" in
      rm) [ -z "${FAKE_IMAGE_IN_USE:-}" ] ;;
      *) [ -f "$WORK/images" ] && cat "$WORK/images" || exit 1 ;;
    esac ;;
  network | volume) [ "$2" != ls ] || if [ "$1" = network ]; then echo "${FAKE_NETWORKS:-}"; else echo "${FAKE_VOLUMES:-}"; fi ;;
  ps) case "$*" in
      *"label=com.docker.compose.project.working_dir=$WORK/home/easi.run "*"publish=9443"*) echo "${FAKE_OURS:-}" ;;
      *) echo "${FAKE_OURS:-}${FAKE_OTHER:-}" ;;
    esac ;;
  inspect) case "$*" in
      *println*) echo "${FAKE_CONTROLLER_ENV:-PATH=/usr/bin}" ;;
      *run.easi.instance*) echo "${FAKE_OTHERS:-|PATH=/usr/bin }" ;;
      *RestartCount*) n="$(cat "$WORK/looks" 2>/dev/null || echo 0)"; echo $((n + 1)) >"$WORK/looks"
        if [ -n "${FAKE_RESTARTS:-}" ]; then echo "running $n"; else echo "${FAKE_STATE:-running 0}"; fi ;;
      *) echo /easi-run-admin-1 ;;
    esac ;;
  exec) cat >/dev/null
    case "$*" in
      *" admin setup") echo "${FAKE_SETUP_OUT-https://localhost:9443/#setup=one-time-token}"; exit "${FAKE_SETUP_STATUS:-0}" ;;
    esac
    [ -z "${FAKE_MCP_DOWN:-}" ] || exit 1
    echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-11-25\"}}" ;;
  compose) shift; [ "$1" = version ] && { echo 2.39.1; exit 0; }
    printf "compose env ADMIN_MAX_SESSIONS=%s\n" "${ADMIN_MAX_SESSIONS:-}" >>"$LOG"
    env | grep -q "^COMPOSE_" && echo "compose with the shell: $(env | grep "^COMPOSE_" | tr "\n" " ")" >>"$LOG"
    while [ "${1#--}" != "$1" ]; do shift 2; done
    case "$1" in
      build) [ "${BUILDX_NO_DEFAULT_ATTESTATIONS:-}" = 1 ] || echo "a build with attestations" >>"$LOG"
        [ -n "${FAKE_SAME_IMAGES:-}" ] || date +%N >>"$WORK/images"; exit "${FAKE_BUILD_STATUS:-0}" ;;
      up) exit "${FAKE_UP_STATUS:-0}" ;;
      cp) echo CERT >"$3" ;;
      logs) echo "admin-1  | admin: ADMIN_MAX_SESSIONS must be 1-16" ;;
      ps) case "$*" in
            *controller*) if [ -n "${FAKE_NEW_CONTROLLER:-}" ] && grep -q " up -d$" "$LOG"; then echo new; else echo old; fi ;;
            *) echo abc123 ;;
          esac ;;
    esac ;;
esac'

# A stand-in for the GitHub repository, with release v0.1.0.
origin="$work/origin"
git init -q -b main "$origin"
git -C "$origin" -c user.name=t -c user.email=t@t commit -q --allow-empty -m one
git -C "$origin" tag v0.1.0
first="$(git -C "$origin" rev-parse HEAD)"

# A fresh machine for each case: an empty home, no systemd files, the raw table's PREROUTING holding one of Docker's.
fresh() {
  current="$1"
  rm -rf "${work:?}/home" "$work/fakeroot" "$work/rules" "$work/log" "$work/claude-easi" "$work/images" "$work/looks"
  mkdir -p "$work/home" "$work/fakeroot/etc/systemd/system" "$work/containerd"
  echo "-A PREROUTING -d 172.17.0.2/32 ! -i docker0 -j DROP" >"$work/rules"
  printf 'MemTotal:        8000000 kB\n' >"$work/meminfo"
  : >"$work/docker.sock"
  : >"$work/log"
}

# Runs install.sh's main with these arguments in a Bash of its own, as `bash install.sh` would (set -e included); its
# output is in $work/out, its status in $status.
cat >"$work/run" <<EOF
# shellcheck disable=SC1091
source "$root/install.sh"
# What the installer reads, pointed at this test's machine.
has_systemd() { true; }
MEMINFO="$work/meminfo"
DOCKER_SOCKET="$work/docker.sock"
DROPIN_DIR="$work/fakeroot/etc/systemd/system/docker.service.d"
DROPIN_FILE="\$DROPIN_DIR/easi-run-firewall.conf"
CONTAINERD_CONFIG="\${FAKE_CONTAINERD_CONFIG:-$work/no-containerd-config}"
CONTAINERD_ROOT="$work/containerd"
PINNED_COMMIT="\${PINNED:-\$PINNED_COMMIT}"
main "\$@"
EOF
# Runs "$@" in the installer's environment: its output in $work/out, its exit status in $status.
in_env() {
  status=0
  # shellcheck disable=SC2030
  HOME="$work/home" PATH="$work/bin:$base_path" LOG="$work/log" WORK="$work" FAKEROOT="$work/fakeroot" \
    FAKE_SOCKET="$work/docker.sock" FAKE_UID="${FAKE_UID:-$real_uid}" EASI_RUN_REPO="$origin" \
    "$@" >"$work/out" 2>&1 || status=$?
}
installer() { in_env bash "$work/run" "$@" </dev/null; }
# A run again with nothing rebuilt, its log cleared first (assignments before it reach the installer too).
again() { : >"$work/log"; FAKE_SAME_IMAGES=1 installer; }
# Deletes every release tag of the stand-in repository.
untag() { for tag in $(git -C "$origin" tag); do git -C "$origin" tag -d "$tag" >/dev/null; done; }

fresh "a first install"
installer
check [ "$status" = 0 ]
contains "$work/out" "easi.run is running: release v0.1.0, in $work/home/easi.run."
check [ "$(git -C "$work/home/easi.run" rev-parse HEAD)" = "$first" ]
check [ "$(stat -c %a "$work/home/.config/easi-run")" = 700 ]
contains "$work/log" "docker exec -i easi-run-admin-1 admin setup"
contains "$work/out" "Password:  create it at https://localhost:9443/#setup=one-time-token (one use, valid for up to 15 minutes; to print it again: docker exec -i easi-run-admin-1 admin setup)"
contains "$work/out" "ssh -N -L 9443:127.0.0.1:9443 tester@box, then open the setup link above"
rule="-A PREROUTING -i er-+ -m addrtype --dst-type LOCAL -j DROP"
check [ "$(head -n 1 "$work/rules")" = "$rule" ]
dropin="$work/fakeroot/etc/systemd/system/docker.service.d/easi-run-firewall.conf"
contains "$dropin" "ExecStartPre=/bin/sh -c 'test \"\$\$(/usr/sbin/iptables -w -t raw -S PREROUTING 1)\" = \"$rule\" || { /usr/sbin/iptables -w -t raw -D PREROUTING -i er-+ -m addrtype --dst-type LOCAL -j DROP 2>/dev/null; /usr/sbin/iptables -w -t raw -I PREROUTING 1 -i er-+ -m addrtype --dst-type LOCAL -j DROP; }'"
contains "$dropin" "ExecStartPre=/bin/sh -c 'test \"\$\$(/usr/sbin/iptables -w -t raw -S PREROUTING 1)\" = \"$rule\"'"
# Each drop-in command, run as systemd would, keeps the rule in place once and fails unless it is.
sed -n "s/^ExecStartPre=//p" "$dropin" | sed 's/\$\$/$/g; s#/usr/sbin/iptables#iptables#g' >"$work/pre"
while IFS= read -r line; do PATH="$work/bin:$base_path" WORK="$work" LOG=/dev/null eval "$line" || fail "$line"; done <"$work/pre"
check [ "$(grep -c 'er-+' "$work/rules")" = 1 ]
# The firewall is in place before anything is built or started; the setup link is asked for once the admin answers.
order="$(grep -nE '^(iptables -w -t raw -I|docker compose .* (build|up -d$)|curl |docker exec .* admin setup)' "$work/log" |
  cut -d: -f2- | uniq)"
check [ "$(echo "$order" | sed -n 1p)" = "iptables -w -t raw -I PREROUTING 1 -i er-+ -m addrtype --dst-type LOCAL -j DROP" ]
echo "$order" | sed -n 2p | grep -q " build$" || fail "build second: $order"
echo "$order" | sed -n 3p | grep -q " up -d$" || fail "up third: $order"
echo "$order" | sed -n 4p | grep -q "^curl " || fail "the admin answering fourth: $order"
echo "$order" | sed -n 5p | grep -q "admin setup$" || fail "the setup link last: $order"
lacks "$work/log" "systemctl restart"
lacks "$work/log" "a build with attestations"
contains "$work/log" "docker compose --project-directory $work/home/easi.run --project-name easi-run --file $work/home/easi.run/compose.yaml build"
contains "$work/log" "curl --noproxy * -fsS --max-time 5 --cacert $work/home/.config/easi-run/tls-cert.pem"
lacks "$work/log" "curl -k"
contains "$work/log" "claude mcp add --scope user easi -- docker exec -i easi-run-admin-1 admin mcp"
contains "$work/out" "MCP:       ready (Claude Code: easi)"
contains "$work/out" "claude mcp add --scope user easi -- ssh -T tester@box docker exec -i easi-run-admin-1 admin mcp"
contains "$work/out" "codex mcp add easi -- docker exec -i easi-run-admin-1 admin mcp"

check grep -q "compose.yaml up -d --no-deps --force-recreate controller admin" "$work/log"
lacks "$work/out" "A session image changed"

current="a second run at the same release"
again
lacks "$work/log" "--force-recreate controller"
lacks "$work/out" "A session image changed"
check [ "$status" = 0 ]
lacks "$work/log" "claude mcp add"
check [ "$(grep -c 'er-+' "$work/rules")" = 1 ]

fresh "a password set already"
FAKE_SETUP_STATUS=3 FAKE_SETUP_OUT="" installer
check [ "$status" = 0 ]
contains "$work/out" "Password:  set already"
lacks "$work/out" "#setup="
contains "$work/out" "ssh -N -L 9443:127.0.0.1:9443 tester@box, then open https://localhost:9443"

for out in "" "admin setup: no setup link is ready" "not a link"; do
  for code in 1 0; do
    fresh "admin setup failing (exit $code, output \"$out\")"
    FAKE_SETUP_STATUS="$code" FAKE_SETUP_OUT="$out" installer
    check [ "$status" = 0 ]
    contains "$work/out" "easi.run is running"
    contains "$work/out" "Password:  create it at the link this prints: docker exec -i easi-run-admin-1 admin setup"
    contains "$work/out" "then open https://localhost:9443"
    [ -z "$out" ] || lacks "$work/out" "$out"
  done
done

fresh "a failing build"
FAKE_BUILD_STATUS=1 installer
check [ "$status" != 0 ]
lacks "$work/log" " up -d"
lacks "$work/out" "easi.run is running"

for state in "restarting 3" "exited 0" "none"; do
  fresh "the admin not answering while it is $state"
  FAKE_ADMIN_DOWN=1 FAKE_STATE="$state" installer
  check [ "$status" = 1 ]
  contains "$work/out" "admin: ADMIN_MAX_SESSIONS must be 1-16"
  contains "$work/out" "the controller or the admin stopped (their last lines are above)"
  contains "$work/log" "logs --no-color --tail 5 controller admin"
  lacks "$work/out" "easi.run is running"
done

fresh "the admin restarting while the installer waits"
FAKE_ADMIN_DOWN=1 FAKE_RESTARTS=1 installer
check [ "$status" = 1 ]
contains "$work/out" "the controller or the admin stopped"
check [ "$(grep -c '^curl ' "$work/log")" = 2 ]

fresh "a configuration directory that others could read"
mkdir -m 755 "$work/home/.config" "$work/home/.config/easi-run"
installer
check [ "$status" = 0 ]
check [ "$(stat -c %a "$work/home/.config/easi-run")" = 700 ]

fresh "a configuration directory that is a symbolic link"
mkdir -p "$work/home/.config" "$work/elsewhere"
ln -s "$work/elsewhere" "$work/home/.config/easi-run"
installer
check [ "$status" = 1 ]
contains "$work/out" "is a symbolic link"
lacks "$work/log" "docker compose --project-directory"

fresh "the MCP server not answering"
FAKE_MCP_DOWN=1 installer
check [ "$status" = 0 ]
contains "$work/out" "MCP:       not answering"
lacks "$work/log" "claude mcp add"

fresh "an override file and an egress file in the checkout"
installer
touch "$work/home/easi.run/compose.override.yaml" "$work/home/easi.run/egress.json"
again
check [ "$status" = 0 ]
contains "$work/log" "--file $work/home/easi.run/compose.yaml --file $work/home/easi.run/compose.override.yaml --file $work/home/easi.run/compose.egress.yaml up -d"

for line in "EGRESS_FILE=/srv/egress.json" "export EGRESS_FILE=/srv/egress.json" "  EGRESS_FILE = /srv/egress.json"; do
  fresh "an egress file named in .env: $line"
  installer
  printf '%s\n' "$line" >"$work/home/easi.run/.env"
  again
  contains "$work/log" "--file $work/home/easi.run/compose.egress.yaml up -d"
done

fresh "the admin's port moved in .env"
installer
printf 'ADMIN_PORT=9444\n' >"$work/home/easi.run/.env"
again
check [ "$status" = 0 ]
contains "$work/log" "ss -Hltn sport = :9444"
contains "$work/log" "--resolve localhost:9444:127.0.0.1"
contains "$work/out" "Admin:     https://localhost:9444"
contains "$work/out" "ssh -N -L 9444:127.0.0.1:9444 tester@box"
FAKE_PORT_BUSY=1 FAKE_SAME_IMAGES=1 installer
check [ "$status" = 1 ]
contains "$work/out" "port 9444 is in use"
# As Compose reads .env: in quotes or not, with a comment after a space; empty, Compose's default.
for line in "ADMIN_PORT='9445'" 'export ADMIN_PORT = "9446"' "ADMIN_PORT=9447 # mine" "ADMIN_PORT=" "ADMIN_PORT=\"\""; do
  printf '%s\n' "$line" >"$work/home/easi.run/.env"
  port="$(printf '%s\n' "$line" | tr -cd '0-9')"
  FAKE_SAME_IMAGES=1 installer
  check [ "$status" = 0 ]
  contains "$work/out" "Admin:     https://localhost:${port:-9443}"
done
for port in 94a4 70000 "9444#x" 0944; do
  printf 'ADMIN_PORT=%s\n' "$port" >"$work/home/easi.run/.env"
  again
  check [ "$status" = 1 ]
  contains "$work/out" "ADMIN_PORT in $work/home/easi.run/.env is not a number from 1 to 65535 such as ADMIN_PORT=9444: ADMIN_PORT=$port"
  lacks "$work/log" "docker compose --project-directory"
done

fresh "sessions at once from this server's memory"
installer
contains "$work/out" "Sessions:  2 at once (from this server's memory"
contains "$work/log" "ADMIN_MAX_SESSIONS=2"
printf 'MemTotal:        67108864 kB\n' >"$work/meminfo"
installer
contains "$work/out" "Sessions:  16 at once"
printf 'MemTotal:        4000000 kB\n' >"$work/meminfo"
installer
contains "$work/out" "Sessions:  1 at once"
printf 'MemTotal:        8000000 kB\n' >"$work/meminfo"
printf 'BROWSER_MEMORY_MIB=1024\n' >"$work/home/easi.run/.env"
installer
contains "$work/out" "Sessions:  3 at once"
printf 'ADMIN_MAX_SESSIONS="5" # mine\n' >"$work/home/easi.run/.env"
installer
contains "$work/out" "Sessions:  5 at once (set in $work/home/easi.run/.env)"
printf 'ADMIN_MAX_SESSIONS=20\n' >"$work/home/easi.run/.env"
installer
check [ "$status" = 1 ]
contains "$work/out" "ADMIN_MAX_SESSIONS in $work/home/easi.run/.env is not a number from 1 to 16"

fresh "a newer release"
installer
git -C "$origin" -c user.name=t -c user.email=t@t commit -q --allow-empty -m two
git -C "$origin" -c user.name=t -c user.email=t@t tag -a -m two v0.10.0
second="$(git -C "$origin" rev-parse HEAD)"
installer
check [ "$status" = 1 ]
contains "$work/out" "Run bash install.sh --update"
check [ "$(git -C "$work/home/easi.run" rev-parse HEAD)" = "$first" ]
installer --update
check [ "$status" = 0 ]
check [ "$(git -C "$work/home/easi.run" rev-parse HEAD)" = "$second" ]
contains "$work/out" "live browser sessions end"
contains "$work/out" "release v0.10.0"

fresh "a checkout with local changes"
installer
echo changed >"$work/home/easi.run/tracked"
git -C "$work/home/easi.run" add tracked
installer --update
check [ "$status" = 1 ]
contains "$work/out" "has local changes"

fresh "no release yet"
untag
PINNED="$first" installer
check [ "$status" = 0 ]
contains "$work/out" "commit $first (no release yet)"
check [ "$(git -C "$work/home/easi.run" rev-parse HEAD)" = "$first" ]
git -C "$origin" tag v0.1.0 "$first"

fresh "the rule behind another one, an ACCEPT"
{ echo "-A PREROUTING -j ACCEPT"; echo "-A PREROUTING -i er-+ -m addrtype --dst-type LOCAL -j DROP"; } >"$work/rules"
installer
check [ "$status" = 0 ]
check [ "$(head -n 1 "$work/rules")" = "-A PREROUTING -i er-+ -m addrtype --dst-type LOCAL -j DROP" ]
check [ "$(grep -c 'er-+' "$work/rules")" = 1 ]

fresh "a rule that cannot be put first"
FAKE_IPTABLES_STUCK=1 installer
check [ "$status" = 1 ]
contains "$work/out" "the rule is not first in the raw table's PREROUTING"
lacks "$work/log" "docker compose --project-directory"

fresh "another easi server in Claude Code"
printf 'easi:\n  Command: npx\n  Args: something-else\n' >"$work/claude-easi"
installer
check [ "$status" = 0 ]
contains "$work/out" "Claude Code has another server named easi, left alone"
lacks "$work/log" "claude mcp add"

fresh "a dry run"
installer --dry-run
check [ "$status" = 0 ]
contains "$work/out" "Checks passed."
check [ ! -e "$work/home/easi.run" ] && check [ ! -e "$work/home/.config/easi-run" ]
lacks "$work/log" "sudo"
lacks "$work/log" "docker compose --project-directory"

# A refusal: the case's name, what the output says, and the installer's arguments. Nothing is installed or removed.
refused() {
  fresh "$1"
  local says="$2"
  shift 2
  installer "$@"
  check [ "$status" = 1 ]
  contains "$work/out" "$says"
  lacks "$work/log" "sudo install"
  lacks "$work/log" " rm "
}
FAKE_UID=0 refused "root" "not as root"
FAKE_ARCH=aarch64 refused "another architecture" "needs Linux on x86-64"
FAKE_DOCKER_VERSION=27.5.1 refused "an older Docker" "Docker Engine 28 or later"
FAKE_SECURITY='["name=seccomp,profile=builtin","name=rootless"]' refused "rootless Docker" "rootless Docker is not supported"
FAKE_OS="Docker Desktop" refused "Docker Desktop" "Docker Desktop is not supported"
FAKE_DISK_KB=1000000 refused "a small disk" "needs 20 GB free"
FAKE_PORT_BUSY=1 refused "a busy port" "port 9443 is in use"
FAKE_UNIT=not-found refused "Docker without docker.service (the snap package)" "not run by docker.service"
FAKE_INACTIVE=1 refused "docker.service not running the Docker that answers" "not run by docker.service"
DOCKER_HOST=tcp://10.0.0.5:2375 refused "another Docker host" "DOCKER_HOST is tcp://10.0.0.5:2375"
FAKE_CONTEXT=unix:///run/user/1000/docker.sock refused "a Docker context on another socket" "the Docker context reaches unix:///run/user/1000/docker.sock"
FAKE_PORT_BUSY=1 FAKE_OTHER=def456 refused "the port held by another checkout's admin" "port 9443 is in use"
FAKE_SUDO_PASSWORD=1 refused "sudo needing a password, with no terminal" "there is no terminal to ask it in"
FAKE_IPTABLES=/home/tester/bin/iptables refused "iptables outside the system's directories" "outside the system's own directories"
FAKE_UID=12345 refused "a configuration directory of another user" "belongs to another user"

fresh "the drop-in not loaded by systemd"
FAKE_NOT_LOADED=1 installer
check [ "$status" = 1 ]
contains "$work/out" "docker.service does not put the rule in place before it starts"
lacks "$work/log" "docker compose --project-directory"

fresh "a checkout of another repository"
installer
git -C "$work/home/easi.run" remote set-url origin https://example.com/other.git
installer
check [ "$status" = 1 ]
contains "$work/out" "is a checkout of another repository"

fresh "a pinned commit that is not a whole commit id"
untag
PINNED=753c55b42c32 installer
check [ "$status" = 1 ]
contains "$work/out" "no commit to install from"
git -C "$origin" tag v0.1.0 "$first"

fresh "Compose variables in the caller's shell"
COMPOSE_FILE=elsewhere.yaml COMPOSE_PROJECT_NAME=other COMPOSE_PROFILES=bench installer
check [ "$status" = 0 ]
lacks "$work/log" "compose with the shell"

fresh "a build that changed a session image after an interrupted run"
installer
date +%N >>"$work/images"
FAKE_SAME_IMAGES=1 FAKE_UP_STATUS=1 installer
again
check [ "$status" = 0 ]
contains "$work/log" "--force-recreate controller admin"
contains "$work/out" "A session image changed (a newer Chrome, say): the controller and the admin start afresh; live browser sessions end."

fresh "the controller recreated by a change of its own"
installer
FAKE_NEW_CONTROLLER=1 again
check [ "$status" = 0 ]
contains "$work/log" "--force-recreate admin"
lacks "$work/log" "--force-recreate controller"

fresh "an uninstall"
installer
echo "$rule" >>"$work/rules"
: >"$work/log"
FAKE_OURS=abc123 FAKE_NETWORKS=net1 FAKE_VOLUMES=vol1 installer --uninstall --yes
check [ "$status" = 0 ]
check [ ! -e "$dropin" ] && check [ ! -e "$(dirname "$dropin")" ]
contains "$work/log" "systemctl daemon-reload"
lacks "$work/rules" "er-+"
contains "$work/rules" "-A PREROUTING -d 172.17.0.2/32 ! -i docker0 -j DROP"
check [ ! -e "$work/home/.config/easi-run" ]
check [ ! -e "$work/claude-easi" ]
contains "$work/log" "claude mcp remove --scope user easi"
check [ -d "$work/home/easi.run/.git" ]
for removed in "container rm -f abc123" "network rm net1" "volume rm vol1" "image rm easi-run/admin:dev" \
  "image rm easi-run/controller:dev" "image rm easi-run/browser:dev"; do
  contains "$work/log" "docker $removed"
done
contains "$work/log" "docker ps -aq --filter label=run.easi.instance=easirun"
lacks "$work/log" "docker compose"
contains "$work/out" "easi.run is removed from this server."
contains "$work/out" "Kept: $work/home/easi.run, the checkout"
lacks "$work/out" "not this installation's"
# Every session's container is gone before the rule that keeps its pages off this server.
first_rule="$(grep -n "^iptables -w -t raw -D" "$work/log" | head -n 1 | cut -d: -f1)"
last_container="$(grep -n "^docker container rm" "$work/log" | tail -n 1 | cut -d: -f1)"
check [ "$last_container" -lt "$first_rule" ]

fresh "an uninstall without --yes, and no terminal to ask in"
installer
: >"$work/log"
FAKE_OURS=abc123 installer --uninstall
check [ "$status" = 1 ]
contains "$work/out" "there is no terminal to ask in: run it in a terminal, or add --yes"
check [ -e "$dropin" ]
lacks "$work/log" "sudo"
lacks "$work/log" " rm "

# Runs the uninstall on a terminal of its own, answering its question with $1.
prompted() { FAKE_OURS=abc123 in_env script -qec "bash $work/run --uninstall" /dev/null <<<"$1"; }
fresh "an uninstall answered at its question"
installer
: >"$work/log"
prompted no
check [ "$status" = 1 ]
contains "$work/out" "Type yes to remove easi.run:"
contains "$work/out" "nothing removed"
check [ -e "$dropin" ]
lacks "$work/log" "docker container rm"
prompted yes
check [ "$status" = 0 ]
check [ ! -e "$dropin" ]
contains "$work/log" "docker container rm -f abc123"

fresh "an uninstall beside another easi server in Claude Code"
installer
printf 'easi:\n  Command: ssh\n  Args: -T me@elsewhere docker exec -i easi-run-admin-1 admin mcp\n' >"$work/claude-easi"
installer --uninstall --yes
check [ "$status" = 0 ]
check [ -e "$work/claude-easi" ]
lacks "$work/log" "claude mcp remove"
contains "$work/out" "Kept: Claude Code's server easi, which is not this installation's"

fresh "an uninstall of a controller with an instance of its own"
installer
FAKE_OURS=abc123 FAKE_CONTROLLER_ENV="CONTROLLER_INSTANCE=mine" installer --uninstall --yes
check [ "$status" = 0 ]
contains "$work/log" "docker ps -aq --filter label=run.easi.instance=mine"
lacks "$work/log" "label=run.easi.instance=easirun"

for others in "demo|PATH=/usr/bin " "|IMAGE_BROWSER=easi-run-demo/browser:latest PATH=/usr/bin "; do
  fresh "an uninstall beside another installation ($others)"
  installer
  : >"$work/log"
  FAKE_OURS=abc123 FAKE_OTHERS="$others" installer --uninstall --yes
  check [ "$status" = 0 ]
  check [ -e "$dropin" ]
  contains "$work/rules" "$rule"
  lacks "$work/log" "daemon-reload"
  contains "$work/out" "Kept: the firewall rule and $dropin, which another easi.run installation here still needs"
  check [ ! -e "$work/home/.config/easi-run" ]
done

fresh "an uninstall with an image still in use"
installer
FAKE_OURS=abc123 FAKE_IMAGE_IN_USE=1 installer --uninstall --yes
check [ "$status" = 1 ]
contains "$work/out" "docker image rm easi-run/admin:dev failed: remove the container that uses it"
check [ -e "$dropin" ]
contains "$work/rules" "$rule"

fresh "an uninstall with nothing installed"
installer --uninstall --yes
check [ "$status" = 0 ]
lacks "$work/log" "daemon-reload"
lacks "$work/log" "docker container rm"
lacks "$work/log" "docker image rm"
contains "$work/out" "easi.run is removed from this server."

FAKE_UID=0 refused "an uninstall as root" "not as root" --uninstall --yes
DOCKER_HOST=tcp://10.0.0.5:2375 refused "an uninstall on another Docker host" "DOCKER_HOST is tcp://10.0.0.5:2375" --uninstall --yes
FAKE_SUDO_PASSWORD=1 refused "an uninstall where sudo needs a password, with no terminal" \
  "there is no terminal to ask it in" --uninstall --yes
FAKE_IPTABLES=/home/tester/bin/iptables refused "an uninstall with iptables outside the system's directories" \
  "outside the system's own directories" --uninstall --yes
for args in "--yes" "--update --dry-run" "--uninstall --update" "--uninstall --yes --update" "--remove"; do
  # shellcheck disable=SC2086
  FAKE_OURS=abc123 refused "the arguments $args" "usage: bash install.sh [--update | --dry-run | --uninstall [--yes]]" $args
done

fresh "containerd's root moved by its configuration"
mkdir -p "$work/etc-containerd" "$work/srv-containerd" && printf "version = 2\n  root = '%s'\n" "$work/srv-containerd" >"$work/etc-containerd/config.toml"
FAKE_CONTAINERD_CONFIG="$work/etc-containerd/config.toml" FAKE_MOVED_KB=1000000 installer
check [ "$status" = 1 ]
contains "$work/out" "($work/srv-containerd)"

fresh "no containerd root of its own"
rmdir "$work/containerd"
FAKE_CONTAINERD_KB=1000000 installer
check [ "$status" = 0 ]

fresh "little room in containerd's store"
FAKE_CONTAINERD_KB=1000000 installer
check [ "$status" = 1 ]
contains "$work/out" "needs 20 GB free where Docker keeps its images ($work/containerd)"

fresh "Docker's own image store"
FAKE_DRIVER='[["Backing Filesystem","extfs"]]' FAKE_CONTAINERD_KB=1000000 installer
check [ "$status" = 0 ]

fresh "little memory"
printf 'MemTotal:        2000000 kB\n' >"$work/meminfo"
installer
check [ "$status" = 1 ]
contains "$work/out" "needs 4 GB of memory"

fresh "the port held by this installation"
FAKE_PORT_BUSY=1 FAKE_OURS=abc123 installer
check [ "$status" = 0 ]

current="versions"
# shellcheck disable=SC1091
source "$root/install.sh"
check major_at_least 28.0.1 28
check major_at_least v2.39.1 2
! major_at_least 27.5.1 28 || fail "27 < 28"
! major_at_least "" 2 || fail "empty"
! major_at_least x.1 2 || fail "not a number"

if [ "$failures" -gt 0 ]; then
  echo "install.sh: $failures failed" >&2
  exit 1
fi
echo "install.sh: every case passed"
