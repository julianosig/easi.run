#!/usr/bin/env bash
# Tests install.sh without changing this machine: it sources the installer and runs it against stubs of sudo,
# systemctl, iptables, docker, ss, df, curl, claude, id and uname in PATH, a fake root for the systemd files, and a
# local git repository standing in for GitHub. Nothing here needs root, Docker or the network. The cases run at the
# same time, as many at once as there are processors, each on a machine of its own: a work directory of its own
# under one temporary directory, which KEEP=1 keeps.
#   scripts/install-test.sh
# The stubs' bodies are single-quoted on purpose: they expand when they run.
# shellcheck disable=SC2016
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap '[ -n "${KEEP:-}" ] || rm -rf "${tmp:?}"' EXIT
real_uid="$(id -u)"
# The installer reads the SSH session it runs in: none unless a case gives one.
unset SSH_CONNECTION SUDO_USER
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
bin="$tmp/bin"
mkdir -p "$bin"
stub() {
  printf '#!/usr/bin/env bash\nset -eu\nprintf "%%s\\n" "$(basename "$0") $*" >>"$LOG"\n%s\n' "$2" >"$bin/$1"
  chmod +x "$bin/$1"
}
stub id 'case "$1" in -u) echo "${FAKE_UID}" ;; -un) echo "${FAKE_USERNAME:-tester}" ;; *) exec /usr/bin/id "$@" ;; esac'
stub logname 'if [ -n "${FAKE_LOGNAME:-}" ]; then echo "$FAKE_LOGNAME"; else exit 1; fi'
stub uname 'case "$1" in -s) echo Linux ;; -m) echo "${FAKE_ARCH:-x86_64}" ;; -n) echo box ;; esac'
stub df 'case "$2" in "$WORK/containerd") kb="${FAKE_CONTAINERD_KB:-50000000}" ;; "$WORK/srv-containerd") kb="${FAKE_MOVED_KB:-50000000}" ;; *) kb="${FAKE_DISK_KB:-50000000}" ;; esac
printf "Filesystem 1024-blocks Used Available Capacity Mounted\n/dev/x 1 1 %s 1%% /\n" "$kb"'
stub ss 'if [ -n "${FAKE_PORT_BUSY:-}" ]; then echo "LISTEN 0 4096 127.0.0.1:9443 0.0.0.0:*"; fi'
# curl answers once the certificate is there, unless FAKE_ADMIN_DOWN; the admin's status, unless that was all the
# call asked (-o /dev/null), is FAKE_STATUS (no password set unless given), or nothing with FAKE_STATUS_FAILS.
stub curl '[ -z "${FAKE_ADMIN_DOWN:-}" ] || exit 7; for a in "$@"; do [ "${next:-}" = 1 ] && { [ -s "$a" ] || exit 7; checked=1; next=; }; [ "$a" = --cacert ] && next=1; [ "$a" = -o ] && quiet=1; done
[ -n "${checked:-}" ] || exit 7; [ -z "${quiet:-}" ] || exit 0; [ -z "${FAKE_STATUS_FAILS:-}" ] || exit 22
body="{\"demo\":null,\"enrolled\":false,\"passkey\":false}"; printf "%s" "${FAKE_STATUS-$body}"'
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
  *) AS_ROOT=1 exec "$@" ;;
esac'
# docker: FAKE_NETWORKS and FAKE_VOLUMES are what its listings find, FAKE_CONTROLLER_ENV is the controller's
# environment, FAKE_OTHERS what the other containers carry ("label|environment"), FAKE_IMAGE_IN_USE (a pattern) fails
# image rm of what it matches; images pulled are in $WORK/pulled, a pull of what FAKE_PULL_FAILS (a pattern) matches
# fails with FAKE_PULL_ERROR (the first FAKE_PULL_FAILURES times only, when set), and $WORK/tags holds each tag with
# what it names (an image's id is what its tag names); each browser image Buildx builds is a line of $WORK/built, a new
# one unless FAKE_SAME_IMAGES, FAKE_BUILD_FAILS builds fail first and FAKE_BUILD_STATUS fails them all, and
# FAKE_NO_BUILDX takes Buildx away; FAKE_CONTEXT is the context's endpoint; FAKE_OURS is this checkout's admin
# publishing port 9443, and FAKE_OTHER one that a query without both of those filters would find as well; a service is
# in FAKE_STATE ("running 0" unless set), or with FAKE_RESTARTS restarted once more at each look; the log notes a
# variable of the caller's shell reaching Compose, a build with attestations, a context or no Dockerfile, a Compose
# build, and an up that could build or pull.
stub docker 'case "$1" in
  context) echo "${FAKE_CONTEXT:-unix://$FAKE_SOCKET}" ;;
  version) echo "${FAKE_DOCKER_VERSION:-28.3.0}" ;;
  info) case "$*" in
      *DriverStatus*) echo "${FAKE_DRIVER:-[[\"driver-type\",\"io.containerd.snapshotter.v1\"]]}" ;;
      *) echo "${FAKE_OS:-Debian GNU/Linux 13 (trixie)}|${FAKE_SECURITY:-[\"name=seccomp,profile=builtin\"]}|$WORK" ;;
    esac ;;
  pull) case "$3" in ${FAKE_PULL_FAILS:-none}) n="$(cat "$WORK/pull-failures" 2>/dev/null || echo 0)"
      echo $((n + 1)) >"$WORK/pull-failures"
      if [ "$n" -lt "${FAKE_PULL_FAILURES:-9}" ]; then echo "${FAKE_PULL_ERROR:-Error response from daemon: connection reset}" >&2; exit 1; fi ;;
    esac
    grep -qxF -- "$3" "$WORK/pulled" 2>/dev/null || echo "$3" >>"$WORK/pulled" ;;
  tag) { grep -v "^$3 " "$WORK/tags" 2>/dev/null || true; echo "$3 $2"; } >"$WORK/tags.new"; mv "$WORK/tags.new" "$WORK/tags" ;;
  image) name="${!#}"; touch "$WORK/pulled" "$WORK/tags"
    case "$2" in
      rm) case "$name" in ${FAKE_IMAGE_IN_USE:-none}) exit 1 ;; esac
        grep -vxF -- "$name" "$WORK/pulled" >"$WORK/pulled.new" || true; mv "$WORK/pulled.new" "$WORK/pulled"
        touch "$WORK/built"; grep -vxF -- "$name" "$WORK/built" >"$WORK/built.new" || true; mv "$WORK/built.new" "$WORK/built"
        grep -v "^$name " "$WORK/tags" >"$WORK/tags.new" || true; mv "$WORK/tags.new" "$WORK/tags" ;;
      ls) case "$*" in
            *"--filter label=run.easi.installer=browser"*) cat "$WORK/built" 2>/dev/null || true ;;
            *) grep -F -- "$name@" "$WORK/pulled" || true ;;
          esac ;;
      *) if grep -q "^$name " "$WORK/tags"; then grep "^$name " "$WORK/tags" | cut -d" " -f2
         elif grep -qxF -- "$name" "$WORK/pulled" "$WORK/built" 2>/dev/null; then echo "$name"
         else exit 1; fi ;;
    esac ;;
  buildx) [ "$2" != version ] || exit "${FAKE_NO_BUILDX:-0}"
    [ "${BUILDX_NO_DEFAULT_ATTESTATIONS:-}" = 1 ] || echo "a build with attestations" >>"$LOG"
    [ -s /dev/stdin ] && [ "${!#}" = - ] || echo "a build with a context or without a Dockerfile" >>"$LOG"
    case "$*" in *"--label run.easi.installer=browser "*) ;; *) echo "a build without the installer label" >>"$LOG" ;; esac
    n="$(cat "$WORK/builds" 2>/dev/null || echo 0)"; echo $((n + 1)) >"$WORK/builds"
    [ "$n" -ge "${FAKE_BUILD_FAILS:-0}" ] || exit 1
    [ -z "${FAKE_BUILD_STATUS:-}" ] || exit "$FAKE_BUILD_STATUS"
    # A new Chrome makes a new image, unless FAKE_SAME_IMAGES.
    if [ -z "${FAKE_SAME_IMAGES:-}" ] || [ ! -s "$WORK/built" ]; then echo "sha256:chrome$(date +%N)" >>"$WORK/built"; fi
    tail -n 1 "$WORK/built" >"$WORK/last-built"
    while [ $# -gt 0 ]; do [ "$1" = --iidfile ] && cp "$WORK/last-built" "$2"; shift; done ;;
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
    esac
    [ -z "${FAKE_MCP_DOWN:-}" ] || exit 1
    echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-11-25\"}}" ;;
  compose) shift; [ "$1" = version ] && { echo 2.39.1; exit 0; }
    printf "compose env ADMIN_MAX_SESSIONS=%s\n" "${ADMIN_MAX_SESSIONS:-}" >>"$LOG"
    printf "compose env ADMIN_UDP_URL=%s ADMIN_UDP_BIND=%s\n" "${ADMIN_UDP_URL:-}" "${ADMIN_UDP_BIND:-}" >>"$LOG"
    env | grep -q "^COMPOSE_" && echo "compose with the shell: $(env | grep "^COMPOSE_" | tr "\n" " ")" >>"$LOG"
    while [ "${1#--}" != "$1" ]; do shift 2; done
    case "$1" in
      build) echo "a Compose build" >>"$LOG"; exit 1 ;;
      up) case "$*" in *"--no-build --pull never"*) ;; *) echo "an up that may build or pull" >>"$LOG" ;; esac
        exit "${FAKE_UP_STATUS:-0}" ;;
      cp) echo CERT >"$3" ;;
      logs) echo "admin-1  | admin: ADMIN_MAX_SESSIONS must be 1-16" ;;
      ps) case "$*" in
            *controller*) if [ -n "${FAKE_NEW_CONTROLLER:-}" ] && grep -q " up -d --no-build --pull never$" "$LOG"; then echo new; else echo old; fi ;;
            *) echo abc123 ;;
          esac ;;
    esac ;;
esac'

# A stand-in for the GitHub repository, with the commit the installer pins and one after it on main, which a clone
# checks out first: each case installs from a copy of its own.
git init -q -b main "$tmp/origin"
echo "FROM scratch AS browser" >"$tmp/origin/Dockerfile"
git -C "$tmp/origin" add Dockerfile
git -C "$tmp/origin" -c user.name=t -c user.email=t@t commit -q -m one
first="$(git -C "$tmp/origin" rev-parse HEAD)"
git -C "$tmp/origin" -c user.name=t -c user.email=t@t commit -q --allow-empty -m unreleased
# The images the installer pins, a digest for each role ($1 tells one release's apart from another's), and the
# references it pulls them by.
digests() {
  local role
  for role in admin controller display agent proxy browser; do printf '%s@sha256:%s ' "$role" "$(printf "$1%.0s" {1..64})"; done
}
registry=ghcr.io/julianosig/easi.run
# sysctl over $WORK/sysctl (name=value lines), and modprobe, which adds BBR to the kernel's congestion controls unless
# FAKE_NO_BBR.
# Only through sudo: it is in /usr/sbin, which an account's PATH may leave out.
stub sysctl '[ -n "${AS_ROOT:-}" ] || { echo "sysctl: command not found" >&2; exit 127; }
state="$WORK/sysctl"
case "$1" in
  -n) sed -n "s/^$2=//p" "$state" ;;
  -q) shift 2; for s in "$@"; do grep -v "^${s%%=*}=" "$state" >"$state.new" || true; echo "$s" >>"$state.new"; mv "$state.new" "$state"; done ;;
esac'
stub modprobe '[ -z "${FAKE_NO_BBR:-}" ] || exit 1; echo "reno cubic bbr" >"$WORK/congestion"'
# The rule the installer puts first in the raw table's PREROUTING.
rule="-A PREROUTING -i er-+ -m addrtype --dst-type LOCAL -j DROP"

# A fresh machine for each case, in its work directory: an empty home, no systemd files, the raw table's PREROUTING
# holding one of Docker's, and the stand-in repository as it starts.
fresh() {
  current="$1"
  rm -rf "${work:?}"
  mkdir -p "$work/home" "$work/fakeroot/etc/systemd/system" "$work/containerd"
  cp -R "$tmp/origin" "$work/origin"
  origin="$work/origin"
  dropin="$work/fakeroot/etc/systemd/system/docker.service.d/easi-run-firewall.conf"
  echo "-A PREROUTING -d 172.17.0.2/32 ! -i docker0 -j DROP" >"$work/rules"
  printf 'MemTotal:        8000000 kB\n' >"$work/meminfo"
  echo "reno cubic" >"$work/congestion"
  printf 'net.core.default_qdisc=fq_codel\nnet.ipv4.tcp_congestion_control=cubic\nnet.ipv4.tcp_slow_start_after_idle=1\n' >"$work/sysctl"
  : >"$work/docker.sock"
  : >"$work/log"
}

# Runs install.sh's main with these arguments in a Bash of its own, as `bash install.sh` would (set -e included); its
# output is in $work/out, its status in $status.
cat >"$tmp/run" <<EOF
# shellcheck disable=SC1091
source "$root/install.sh"
# What the installer reads, pointed at this case's machine.
has_systemd() { true; }
MEMINFO="\$WORK/meminfo"
DOCKER_SOCKET="\$WORK/docker.sock"
DROPIN_DIR="\$WORK/fakeroot/etc/systemd/system/docker.service.d"
DROPIN_FILE="\$DROPIN_DIR/easi-run-firewall.conf"
SYSCTL_FILE="\$WORK/fakeroot/etc/sysctl.d/90-easi-run.conf"
MODULES_FILE="\$WORK/fakeroot/etc/modules-load.d/easi-run.conf"
CONGESTION_CONTROLS="\$WORK/congestion"
CONTAINERD_CONFIG="\${FAKE_CONTAINERD_CONFIG:-\$WORK/no-containerd-config}"
CONTAINERD_ROOT="\$WORK/containerd"
PINNED_COMMIT="\$PINNED"
read -ra PINNED_IMAGES <<<"\$IMAGES"
main "\$@"
EOF
# Runs "$@" in the installer's environment: its output in $work/out, its exit status in $status.
in_env() {
  status=0
  # shellcheck disable=SC2030
  HOME="$work/home" PATH="$bin:$base_path" LOG="$work/log" WORK="$work" FAKEROOT="$work/fakeroot" \
    FAKE_SOCKET="$work/docker.sock" FAKE_UID="${FAKE_UID:-$real_uid}" EASI_RUN_REPO="$origin" \
    PINNED="${PINNED-$first}" IMAGES="${IMAGES-$(digests a)}" \
    "$@" >"$work/out" 2>&1 || status=$?
}
installer() { in_env bash "$tmp/run" "$@" </dev/null; }
# The line of the log where the Nth (default first) call matching an extended expression is.
at() { grep -nE "$1" "$work/log" | sed -n "${2:-1}p" | cut -d: -f1; }
# A run again with nothing rebuilt, its log cleared first (assignments before it reach the installer too).
again() { : >"$work/log"; FAKE_SAME_IMAGES=1 installer; }

# The cases, each a command that job adds to the list run at the end.
cases=()
job() {
  local line
  printf -v line '%q ' "$@"
  cases+=("$line")
}

case_first_install() {
  fresh "a first install"
  installer
  check [ "$status" = 0 ]
  contains "$work/out" "easi.run is running: release ${first:0:12}, in $work/home/easi.run."
  check [ "$(git -C "$work/home/easi.run" rev-parse HEAD)" = "$first" ]
  check [ "$(stat -c %a "$work/home/.config/easi-run")" = 700 ]
  contains "$work/out" "Password:  create it at https://localhost:9443 (open it first: until then, no one can log in)"
  contains "$work/out" "ssh -N -L 9443:127.0.0.1:9443 you@your-server (with your usual login), then open https://localhost:9443"
  check [ "$(head -n 1 "$work/rules")" = "$rule" ]
  contains "$dropin" "ExecStartPre=/bin/sh -c 'test \"\$\$(/usr/sbin/iptables -w -t raw -S PREROUTING 1)\" = \"$rule\" || { /usr/sbin/iptables -w -t raw -D PREROUTING -i er-+ -m addrtype --dst-type LOCAL -j DROP 2>/dev/null; /usr/sbin/iptables -w -t raw -I PREROUTING 1 -i er-+ -m addrtype --dst-type LOCAL -j DROP; }'"
  contains "$dropin" "ExecStartPre=/bin/sh -c 'test \"\$\$(/usr/sbin/iptables -w -t raw -S PREROUTING 1)\" = \"$rule\"'"
  # Each drop-in command, run as systemd would, keeps the rule in place once and fails unless it is.
  sed -n "s/^ExecStartPre=//p" "$dropin" | sed 's/\$\$/$/g; s#/usr/sbin/iptables#iptables#g' >"$work/pre"
  while IFS= read -r line; do PATH="$bin:$base_path" WORK="$work" LOG=/dev/null eval "$line" || fail "$line"; done <"$work/pre"
  check [ "$(grep -c 'er-+' "$work/rules")" = 1 ]
  # Each image pulled by its pinned digest, and Chrome added to the pulled browser image in Docker's own builder, from
  # the checkout's Dockerfile with no context; the others tagged as compose.yaml names them, the browser's as built.
  for image in $(digests a); do
    contains "$work/log" "docker pull --quiet $registry-$image"
    [ "${image%%@*}" = browser ] || contains "$work/tags" "easi-run/${image%%@*}:dev $registry-$image"
  done
  check [ "$(grep -c '^docker pull ' "$work/log")" = 6 ]
  contains "$work/log" "docker buildx build --builder default --target browser --build-arg BROWSER_BASE=$registry-browser@sha256:$(printf 'a%.0s' {1..64}) --label run.easi.installer=browser --iidfile $work/home/.config/easi-run/browser.iid -"
  contains "$work/tags" "easi-run/browser:dev $(tail -n 1 "$work/built")"
  lacks "$work/log" "docker tag $registry-browser@"
  lacks "$work/log" "a build with a context"
  lacks "$work/log" "a build without the installer label"
  lacks "$work/log" "a Compose build"
  lacks "$work/log" "an up that may build or pull"
  # The firewall is in place before anything is pulled or started, the images are tagged once Chrome is added, and
  # the password's state is asked for once the admin answers, last.
  steps=("$(at '^iptables -w -t raw -I')" "$(at '^docker pull ')" "$(at '^docker pull ' 6)" "$(at '^docker buildx build ')"
    "$(at '^docker tag ')" "$(at ' up -d --no-build --pull never$')" "$(at '^curl ')"
    "$(at '^curl .*9443:127.0.0.1 https://localhost:9443/api/status$')")
  local step
  for step in 1 2 3 4 5 6 7; do
    if [ -z "${steps[$((step - 1))]}" ] || [ -z "${steps[$step]}" ] || [ "${steps[$((step - 1))]}" -ge "${steps[$step]}" ]; then
      fail "step $step out of order: ${steps[*]}"
    fi
  done
  grep '^curl ' "$work/log" | tail -n 1 | grep -q "9443:127.0.0.1 https://localhost:9443/api/status$" ||
    fail "the password's state last: $(grep '^curl ' "$work/log" | tail -n 1)"
  lacks "$work/log" "systemctl restart"
  lacks "$work/log" "a build with attestations"
  lacks "$work/log" "docker image rm"
  contains "$work/log" "curl --noproxy * -fsS --max-time 5 --cacert $work/home/.config/easi-run/tls-cert.pem"
  lacks "$work/log" "curl -k"
  contains "$work/log" "claude mcp add --scope user easi -- docker exec -i easi-run-admin-1 admin mcp"
  contains "$work/out" "MCP:       ready (Claude Code: easi)"
  contains "$work/out" "claude mcp add --scope user easi -- ssh -T you@your-server docker exec -i easi-run-admin-1 admin mcp"
  contains "$work/out" "codex mcp add easi -- docker exec -i easi-run-admin-1 admin mcp"

  check grep -q "compose.yaml up -d --no-build --pull never --no-deps --force-recreate controller admin" "$work/log"
  lacks "$work/out" "A session image changed"

  current="a second run at the same release"
  again
  lacks "$work/log" "--force-recreate controller"
  lacks "$work/log" "docker image rm"
  contains "$work/tags" "easi-run/browser:dev $(tail -n 1 "$work/built")"
  lacks "$work/out" "A session image changed"
  check [ "$status" = 0 ]
  lacks "$work/log" "claude mcp add"
  check [ "$(grep -c 'er-+' "$work/rules")" = 1 ]
}
job case_first_install

# The tunnel line the summary prints from an SSH session's SSH_CONNECTION ("$1") as tester, logged in as tester,
# with any assignments after it.
summary_in_session() {
  SSH_CONNECTION="$1" USER=tester LOGNAME=tester FAKE_LOGNAME=tester in_env env "${@:2}" bash "$tmp/run" </dev/null
}
case_tunnel_as_connected() {
  fresh "the tunnel line as the person connected"
  summary_in_session "81.2.69.160 50123 52.14.0.7 2222"
  check [ "$status" = 0 ]
  contains "$work/out" "From another computer: use the command you already use to log in to this server, adding -N -L 9443:127.0.0.1:9443."
  contains "$work/out" "Tunnel:    ssh -N -L 9443:127.0.0.1:9443 -p 2222 tester@52.14.0.7 (for example, as you connected now), then open https://localhost:9443"
  contains "$work/out" "claude mcp add --scope user easi -- ssh -T -p 2222 tester@52.14.0.7 docker exec -i easi-run-admin-1 admin mcp"
  contains "$work/out" "or add port 9443 in their Ports view; either way, the local port must be 9443 too"
  current="the tunnel line from a private address"
  summary_in_session "81.2.69.160 50123 10.0.0.7 22"
  check [ "$status" = 0 ]
  contains "$work/out" "Tunnel:    ssh -N -L 9443:127.0.0.1:9443 you@your-server (with your usual login), then open"
  lacks "$work/out" "10.0.0.7"
  lacks "$work/out" "as you connected now"
}
job case_tunnel_as_connected

# What ssh_login makes of SSH_CONNECTION "$1" as tester, logged in as tester, with any assignments after it.
login_of() {
  SSH_CONNECTION="$1" USER=tester LOGNAME=tester FAKE_LOGNAME=tester in_env env "${@:2}" \
    bash -c 'source "$1"; ssh_login' _ "$root/install.sh"
  check [ "$status" = 0 ]
  cat "$work/out"
}
case_ssh_logins() {
  fresh "the SSH login read from SSH_CONNECTION"
  local connection address
  check [ "$(login_of "81.2.69.160 50123 52.14.0.7 22")" = "tester@52.14.0.7" ]
  check [ "$(login_of "2a02:c7f::4 50123 2a01:4f8::7 22")" = "tester@2a01:4f8::7" ]
  check [ "$(login_of "2a02:c7f::4 50123 2a01:04f8:0:0:0:0:0:7 22")" = "tester@2a01:04f8:0:0:0:0:0:7" ]
  check [ "$(login_of "81.2.69.160 50123 52.14.0.7 2222")" = "-p 2222 tester@52.14.0.7" ]
  check [ "$(login_of "2a02:c7f::4 50123 2a01:4f8::7 65535")" = "-p 65535 tester@2a01:4f8::7" ]
  # Just outside the special-purpose ranges.
  for address in 1.0.0.1 9.255.255.255 11.0.0.1 100.63.255.255 100.128.0.1 126.255.255.255 128.0.0.1 169.253.0.1 \
    169.255.0.1 172.15.255.255 172.32.0.1 191.255.255.255 192.0.1.1 192.0.3.1 192.88.98.1 192.88.100.1 192.167.0.1 192.169.0.1 \
    198.17.255.255 198.20.0.1 198.51.99.1 198.51.101.1 203.0.112.1 203.0.114.1 223.255.255.255 2000::1 2001:200::1 \
    2001:db7::1 2001:db9::1 3ffe::1 3fff:1000::1 2A01:4F8::7; do
    check [ "$(login_of "81.2.69.160 50123 $address 22")" = "tester@$address" ]
  done
  # Private, shared, link-local, loopback, documentation, benchmarking, multicast or reserved: behind NAT, the person
  # reached another address.
  for address in 0.0.0.0 10.0.0.7 10.255.255.255 100.64.0.1 100.127.255.255 127.0.0.1 169.254.1.1 169.254.200.1 \
    172.16.0.1 172.31.255.255 192.0.0.8 192.0.2.1 192.88.99.2 192.168.1.2 198.18.0.1 198.19.255.255 198.51.100.7 203.0.113.7 224.0.0.1 \
    240.0.0.1 255.255.255.255 ::1 :: fe80::1 fd00::7 1fff::1 4000::1 200::1 2001::1 2001:1ff::1 2001:db8::7 \
    2001:0db8::7 3fff::1 3fff:fff::1 ::ffff:52.14.0.7; do
    check [ -z "$(login_of "81.2.69.160 50123 $address 22")" ]
  done
  # A session from this server itself, or from a private or loopback address.
  for connection in "52.14.0.7 50123 52.14.0.7 22" "127.0.0.1 50123 52.14.0.7 22" "::1 50123 2a01:4f8::7 22" \
    "10.0.0.4 50123 52.14.0.7 22" "192.88.99.2 50123 52.14.0.7 22"; do
    check [ -z "$(login_of "$connection")" ]
  done
  # Anything but the four fields, each well formed.
  for connection in "" "81.2.69.160 50123 52.14.0.7" "81.2.69.160 50123 52.14.0.7 22 x" \
    "81.2.69.160  50123 52.14.0.7 22" " 81.2.69.160 50123 52.14.0.7 22" "81.2.69.160 50123 52.14.0.7 22 " \
    "81.2.69.160 50123 52.14.0.7 0" "81.2.69.160 50123 52.14.0.7 65536" "81.2.69.160 50123 52.14.0.7 022" \
    "81.2.69.160 0 52.14.0.7 22" "81.2.69.160 50123 52.14.0.07 22" "81.2.69.160 50123 52.14.0.256 22" \
    "81.2.69.160 50123 52.14.0 22" "81.2.69.160 50123 52.14.0.7.1 22" "81.2.69 50123 52.14.0.7 22" \
    "81.2.69.160 50123 2a01:4f8:::7 22" "81.2.69.160 50123 2a01:4f8::7::1 22" \
    "81.2.69.160 50123 2a01:4f8:0:0:0:0:0:0:7 22" "81.2.69.160 50123 2a01:4f8:0:0:0:0:0::7 22" \
    "81.2.69.160 50123 2a01:4f8:00007::1 22" "81.2.69.160 50123 2a01:4f8:0:0:0:0:7 22" \
    "81.2.69.160 50123 2a01:4f8:7 22" "81.2.69.160 50123 2a01:4f8:: 22x" "x 50123 52.14.0.7 22" \
    $'81.2.69.160 50123 52.14.0.7 22\n' $'81.2.69.160 50123 52.14.0.7\t22' $'81.2.69.160 50123 52.14.0.7 22\e[2J'; do
    check [ -z "$(login_of "$connection")" ]
  done
  # Another account than the one logged in, no way to tell which that was, or an account name it does not take.
  check [ -z "$(login_of "81.2.69.160 50123 52.14.0.7 22" SUDO_USER=someone)" ]
  check [ -z "$(login_of "81.2.69.160 50123 52.14.0.7 22" USER=someone)" ]
  check [ -z "$(login_of "81.2.69.160 50123 52.14.0.7 22" LOGNAME=someone)" ]
  check [ -z "$(login_of "81.2.69.160 50123 52.14.0.7 22" FAKE_LOGNAME=someone)" ]
  check [ -z "$(login_of "81.2.69.160 50123 52.14.0.7 22" FAKE_LOGNAME=)" ]
  for name in -oProxyCommand=x "a b" 1tester; do
    check [ -z "$(login_of "81.2.69.160 50123 52.14.0.7 22" FAKE_USERNAME="$name" USER="$name" LOGNAME="$name" FAKE_LOGNAME="$name")" ]
  done
}
job case_ssh_logins

case_password_set() {
  fresh "a password set already"
  FAKE_STATUS='{"demo":null,"enrolled":true,"passkey":false}' installer
  check [ "$status" = 0 ]
  contains "$work/out" "Password:  set already"
  lacks "$work/out" "create it"
  contains "$work/out" "ssh -N -L 9443:127.0.0.1:9443 you@your-server (with your usual login), then open https://localhost:9443"
}
job case_password_set

case_status_unknown() {
  local answer
  for answer in "" "not json" '{"enrolled":"maybe"}' fails; do
    fresh "the admin's status answering \"$answer\""
    if [ "$answer" = fails ]; then FAKE_STATUS_FAILS=1 installer; else FAKE_STATUS="$answer" installer; fi
    check [ "$status" = 0 ]
    contains "$work/out" "easi.run is running"
    contains "$work/out" "Password:  unknown (the admin's status did not answer); unless one is set, create it at https://localhost:9443"
  done
}
job case_status_unknown

case_failing_chrome() {
  fresh "Chrome failing to be added"
  FAKE_BUILD_STATUS=1 installer
  check [ "$status" != 0 ]
  check [ "$(cat "$work/builds")" = 3 ]
  contains "$work/out" "Chrome could not be added to the browser image (the lines above say why): easi.run's containers are as they were"
  lacks "$work/log" "docker tag"
  lacks "$work/log" " up -d"
  lacks "$work/out" "easi.run is running"
}
job case_failing_chrome

case_chrome_stopping_twice() {
  fresh "adding Chrome stopping twice, then working"
  FAKE_BUILD_FAILS=2 installer
  check [ "$status" = 0 ]
  check [ "$(cat "$work/builds")" = 3 ]
  contains "$work/out" "Adding Chrome stopped; trying again (3 of 3)"
  contains "$work/out" "easi.run is running"
}
job case_chrome_stopping_twice

# A pull that fails: on every try (a pinned digest the registry does not serve, or content that does not match it),
# or twice and then not.
case_failing_pull() {
  for error in "Error response from daemon: manifest unknown" \
    "Error response from daemon: manifest verification failed for digest sha256:cccc"; do
    fresh "a pull failing: $error"
    FAKE_PULL_FAILS="*easi.run-controller@*" FAKE_PULL_ERROR="$error" installer
    check [ "$status" = 1 ]
    check [ "$(grep -c "^docker pull --quiet $registry-controller@" "$work/log")" = 3 ]
    contains "$work/out" "$error"
    contains "$work/out" "could not pull $registry-controller@sha256:"
    contains "$work/out" "trying again (3 of 3)"
    lacks "$work/out" "(4 of 3)"
    contains "$work/out" "easi.run's containers are as they were; run bash install.sh again"
    lacks "$work/log" "docker tag"
    lacks "$work/log" " build "
    lacks "$work/log" " up -d"
  done
}
job case_failing_pull

case_pull_stopping_twice() {
  fresh "a pull stopping twice, then working"
  FAKE_PULL_FAILS="*easi.run-proxy@*" FAKE_PULL_FAILURES=2 installer
  check [ "$status" = 0 ]
  check [ "$(grep -c "^docker pull --quiet $registry-proxy@" "$work/log")" = 3 ]
  contains "$work/out" "Pulling $registry-proxy@sha256:$(printf 'a%.0s' {1..64}) stopped; trying again (3 of 3)."
  contains "$work/out" "easi.run is running"
}
job case_pull_stopping_twice

case_admin_not_answering() {
  for state in "restarting 3" "exited 0" "none"; do
    fresh "the admin not answering while it is $state"
    FAKE_ADMIN_DOWN=1 FAKE_STATE="$state" installer
    check [ "$status" = 1 ]
    contains "$work/out" "admin: ADMIN_MAX_SESSIONS must be 1-16"
    contains "$work/out" "the controller or the admin stopped (their last lines are above)"
    contains "$work/log" "logs --no-color --tail 5 controller admin"
    lacks "$work/out" "easi.run is running"
  done
}
job case_admin_not_answering

case_admin_restarting() {
  fresh "the admin restarting while the installer waits"
  FAKE_ADMIN_DOWN=1 FAKE_RESTARTS=1 installer
  check [ "$status" = 1 ]
  contains "$work/out" "the controller or the admin stopped"
  check [ "$(grep -c '^curl ' "$work/log")" = 2 ]
}
job case_admin_restarting

case_config_readable() {
  fresh "a configuration directory that others could read"
  mkdir -m 755 "$work/home/.config" "$work/home/.config/easi-run"
  installer
  check [ "$status" = 0 ]
  check [ "$(stat -c %a "$work/home/.config/easi-run")" = 700 ]
}
job case_config_readable

case_config_symlink() {
  fresh "a configuration directory that is a symbolic link"
  mkdir -p "$work/home/.config" "$work/elsewhere"
  ln -s "$work/elsewhere" "$work/home/.config/easi-run"
  installer
  check [ "$status" = 1 ]
  contains "$work/out" "is a symbolic link"
  lacks "$work/log" "docker compose --project-directory"
}
job case_config_symlink

case_mcp_down() {
  fresh "the MCP server not answering"
  FAKE_MCP_DOWN=1 installer
  check [ "$status" = 0 ]
  contains "$work/out" "MCP:       not answering"
  lacks "$work/log" "claude mcp add"
}
job case_mcp_down

case_override() {
  fresh "an override file in the checkout"
  installer
  touch "$work/home/easi.run/compose.override.yaml"
  again
  check [ "$status" = 0 ]
  contains "$work/log" "--file $work/home/easi.run/compose.yaml --file $work/home/easi.run/compose.override.yaml up -d"
}
job case_override

case_port_moved() {
  fresh "the admin's port moved in .env"
  installer
  printf 'ADMIN_PORT=9444\n' >"$work/home/easi.run/.env"
  again
  check [ "$status" = 0 ]
  contains "$work/log" "ss -Hl --tcp -n sport = :9444"
  contains "$work/log" "ss -Hl --udp -n sport = :9444"
  contains "$work/log" "--resolve localhost:9444:127.0.0.1"
  contains "$work/out" "Admin:     https://localhost:9444"
  contains "$work/out" "adding -N -L 9444:127.0.0.1:9444."
  contains "$work/out" "ssh -N -L 9444:127.0.0.1:9444 you@your-server (with your usual login), then open https://localhost:9444"
  contains "$work/out" "Password:  create it at https://localhost:9444 (open it first"
  contains "$work/out" "add port 9444 in their Ports view; either way, the local port must be 9444 too"
  FAKE_PORT_BUSY=1 FAKE_SAME_IMAGES=1 installer
  check [ "$status" = 1 ]
  contains "$work/out" "port 9444 is in use"
}
job case_port_moved

# The admin's port in .env as Compose reads it: in quotes or not, with a comment after a space; empty, Compose's default.
case_port_in_env() {
  fresh "the admin's port in .env as $1"
  installer
  printf '%s\n' "$1" >"$work/home/easi.run/.env"
  port="$(printf '%s\n' "$1" | tr -cd '0-9')"
  again
  check [ "$status" = 0 ]
  contains "$work/out" "Admin:     https://localhost:${port:-9443}"
}
for line in "ADMIN_PORT='9445'" 'export ADMIN_PORT = "9446"' "ADMIN_PORT=9447 # mine" "ADMIN_PORT=" "ADMIN_PORT=\"\""; do
  job case_port_in_env "$line"
done

case_port_refused() {
  fresh "the admin's port in .env as $1, refused"
  installer
  printf 'ADMIN_PORT=%s\n' "$1" >"$work/home/easi.run/.env"
  again
  check [ "$status" = 1 ]
  contains "$work/out" "ADMIN_PORT in $work/home/easi.run/.env is not a number from 1 to 65535 such as ADMIN_PORT=9444: ADMIN_PORT=$1"
  lacks "$work/log" "docker compose --project-directory"
}
for port in 94a4 70000 "9444#x" 0944; do job case_port_refused "$port"; done

# Sessions at once on a server of $1 kB, with the line $2 in .env (none when empty): the summary says $3, or, with $4
# set, the installer refuses with that.
case_sessions() {
  fresh "sessions at once with $1 kB and .env \"$2\""
  printf 'MemTotal:        %s kB\n' "$1" >"$work/meminfo"
  installer
  if [ -n "$2" ]; then
    printf '%s\n' "$2" >"$work/home/easi.run/.env"
    again
  fi
  if [ -n "${4:-}" ]; then
    check [ "$status" = 1 ]
    contains "$work/out" "$4"
  else
    check [ "$status" = 0 ]
    contains "$work/out" "$3"
    # The number Compose passes on, to the admin and the controller.
    local n="${3#Sessions:  }"
    contains "$work/log" "compose env ADMIN_MAX_SESSIONS=${n%% *}"
  fi
}
job case_sessions 8000000 "" "Sessions:  2 at once (from this server's memory"
job case_sessions 67108864 "" "Sessions:  16 at once"
job case_sessions 4000000 "" "Sessions:  1 at once"
job case_sessions 8000000 "BROWSER_MEMORY_MIB=1024" "Sessions:  3 at once"
job case_sessions 8000000 'ADMIN_MAX_SESSIONS="5" # mine' "Sessions:  5 at once (set in"
job case_sessions 8000000 "ADMIN_MAX_SESSIONS=20" "" "/home/easi.run/.env is not a number from 1 to 16 such as ADMIN_MAX_SESSIONS=2"

# A newer installer: another commit and other images.
case_newer_release() {
  fresh "a newer release"
  installer
  git -C "$origin" -c user.name=t -c user.email=t@t commit -q --allow-empty -m two
  second="$(git -C "$origin" rev-parse HEAD)"
  : >"$work/log"
  PINNED="$second" IMAGES="$(digests b)" installer
  check [ "$status" = 1 ]
  contains "$work/out" "this install.sh installs release ${second:0:12}. Run bash install.sh --update"
  check [ "$(git -C "$work/home/easi.run" rev-parse HEAD)" = "$first" ]
  lacks "$work/log" "docker pull"
  PINNED="$second" IMAGES="$(digests b)" FAKE_SAME_IMAGES=1 installer --update
  check [ "$status" = 0 ]
  check [ "$(git -C "$work/home/easi.run" rev-parse HEAD)" = "$second" ]
  contains "$work/out" "live browser sessions end"
  contains "$work/out" "release ${second:0:12}"
  # The new images, tagged; the session images changed, so the controller and the admin start afresh; and the earlier
  # release's images are gone, once the admin answers.
  for image in $(digests b); do
    contains "$work/log" "docker pull --quiet $registry-$image"
    [ "${image%%@*}" = browser ] || contains "$work/tags" "easi-run/${image%%@*}:dev $registry-$image"
  done
  contains "$work/log" "--force-recreate controller admin"
  contains "$work/out" "A session image changed"
  for image in $(digests a); do
    contains "$work/log" "docker image rm $registry-$image"
    lacks "$work/pulled" "$registry-$image"
  done
  check [ "$(grep -c . "$work/pulled")" = 6 ]
  check [ "$(at '^curl ')" -lt "$(at '^docker image rm ')" ]
  lacks "$work/out" "Images:"
}
job case_newer_release

case_newer_chrome() {
  fresh "a newer Chrome"
  installer
  old="$(tail -n 1 "$work/built")"
  : >"$work/log"
  installer
  check [ "$status" = 0 ]
  new="$(tail -n 1 "$work/built")"
  check [ "$old" != "$new" ]
  contains "$work/tags" "easi-run/browser:dev $new"
  contains "$work/out" "A session image changed (a newer Chrome, say)"
  contains "$work/log" "--force-recreate controller admin"
  # The browser image the last run built goes once the admin answers; the pulled images stay.
  contains "$work/log" "docker image rm $old"
  check [ "$(at '^curl ')" -lt "$(at "^docker image rm $old")" ]
  check [ "$(grep -c '^docker image rm' "$work/log")" = 1 ]
}
job case_newer_chrome

case_locked() {
  fresh "another run under way"
  mkdir -p "$work/home/.config/easi-run"
  exec 8>"$work/home/.config/easi-run/lock"
  flock 8
  installer
  exec 8>&-
  check [ "$status" = 1 ]
  contains "$work/out" "another run of install.sh is under way on this server"
  check [ ! -e "$work/home/easi.run" ]
  lacks "$work/log" "sudo install"
  installer
  check [ "$status" = 0 ]
}
job case_locked

case_update_image_in_use() {
  fresh "an update with an earlier image still in use"
  installer
  IMAGES="$(digests b)" FAKE_IMAGE_IN_USE="*easi.run-display@*" installer --update
  check [ "$status" = 0 ]
  contains "$work/out" "Kept images of an earlier release, which a container still uses (the next run removes them): $registry-display@sha256:$(printf 'a%.0s' {1..64})"
  lacks "$work/pulled" "$registry-admin@sha256:a"
  # Once no container uses it, the next run removes it.
  IMAGES="$(digests b)" again
  check [ "$status" = 0 ]
  lacks "$work/pulled" "$registry-display@sha256:a"
  lacks "$work/out" "Kept images"
}

case_browser_image_in_use() {
  fresh "an earlier browser image still in use"
  installer
  first_built="$(tail -n 1 "$work/built")"
  FAKE_IMAGE_IN_USE="$first_built" installer
  check [ "$status" = 0 ]
  contains "$work/out" "Kept images of an earlier release, which a container still uses (the next run removes them): $first_built"
  # Two runs later, with another browser image built between, it is still found by its label, and goes.
  installer
  check [ "$status" = 0 ]
  lacks "$work/built" "$first_built"
  check [ "$(grep -c . "$work/built")" = 1 ]
}
job case_browser_image_in_use

case_older_installer() {
  fresh "an older installer than the installation"
  git -C "$origin" -c user.name=t -c user.email=t@t commit -q --allow-empty -m two
  second="$(git -C "$origin" rev-parse HEAD)"
  PINNED="$second" installer
  check [ "$status" = 0 ]
  : >"$work/log"
  installer --update
  check [ "$status" = 1 ]
  contains "$work/out" "is at ${second:0:12}, newer than release ${first:0:12}, which this install.sh installs: run the newest one"
  check [ "$(git -C "$work/home/easi.run" rev-parse HEAD)" = "$second" ]
  lacks "$work/log" "docker pull"
}
job case_older_installer
job case_update_image_in_use

case_local_changes() {
  fresh "a checkout with local changes"
  installer
  echo changed >"$work/home/easi.run/tracked"
  git -C "$work/home/easi.run" add tracked
  installer --update
  check [ "$status" = 1 ]
  contains "$work/out" "has local changes"
}
job case_local_changes


case_rule_behind_accept() {
  fresh "the rule behind another one, an ACCEPT"
  { echo "-A PREROUTING -j ACCEPT"; echo "-A PREROUTING -i er-+ -m addrtype --dst-type LOCAL -j DROP"; } >"$work/rules"
  installer
  check [ "$status" = 0 ]
  check [ "$(head -n 1 "$work/rules")" = "-A PREROUTING -i er-+ -m addrtype --dst-type LOCAL -j DROP" ]
  check [ "$(grep -c 'er-+' "$work/rules")" = 1 ]
}
job case_rule_behind_accept

case_rule_not_first() {
  fresh "a rule that cannot be put first"
  FAKE_IPTABLES_STUCK=1 installer
  check [ "$status" = 1 ]
  contains "$work/out" "the rule is not first in the raw table's PREROUTING"
  lacks "$work/log" "docker compose --project-directory"
}
job case_rule_not_first

case_other_mcp_server() {
  fresh "another easi server in Claude Code"
  printf 'easi:\n  Command: npx\n  Args: something-else\n' >"$work/claude-easi"
  installer
  check [ "$status" = 0 ]
  contains "$work/out" "Claude Code has another server named easi, left alone"
  lacks "$work/log" "claude mcp add"
}
job case_other_mcp_server

case_dry_run() {
  fresh "a dry run"
  installer --dry-run
  check [ "$status" = 0 ]
  contains "$work/out" "Checks passed."
  check [ ! -e "$work/home/easi.run" ] && check [ ! -e "$work/home/.config/easi-run" ]
  lacks "$work/log" "sudo"
  lacks "$work/log" "docker compose --project-directory"
  lacks "$work/log" "docker pull"
  for image in $(digests a); do contains "$work/out" "      $registry-$image"; done
}
job case_dry_run

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
job FAKE_UID=0 refused "root" "not as root"
job FAKE_ARCH=aarch64 refused "another architecture" "needs Linux on x86-64"
job FAKE_DOCKER_VERSION=27.5.1 refused "an older Docker" "Docker Engine 28 or later"
job FAKE_NO_BUILDX=1 refused "Docker without Buildx" "Docker Buildx (the docker buildx plugin), which adds Chrome to the browser image, is missing"
job FAKE_SECURITY='["name=seccomp,profile=builtin","name=rootless"]' refused "rootless Docker" "rootless Docker is not supported"
job FAKE_OS="Docker Desktop" refused "Docker Desktop" "Docker Desktop is not supported"
job FAKE_DISK_KB=1000000 refused "a small disk" "needs 10 GB free"
job FAKE_PORT_BUSY=1 refused "a busy port" "port 9443 is in use"
job FAKE_UNIT=not-found refused "Docker without docker.service (the snap package)" "not run by docker.service"
job FAKE_INACTIVE=1 refused "docker.service not running the Docker that answers" "not run by docker.service"
job DOCKER_HOST=tcp://10.0.0.5:2375 refused "another Docker host" "DOCKER_HOST is tcp://10.0.0.5:2375"
job FAKE_CONTEXT=unix:///run/user/1000/docker.sock refused "a Docker context on another socket" "the Docker context reaches unix:///run/user/1000/docker.sock"
job FAKE_PORT_BUSY=1 FAKE_OTHER=def456 refused "the port held by another checkout's admin" "port 9443 is in use"
job FAKE_SUDO_PASSWORD=1 refused "sudo needing a password, with no terminal" "there is no terminal to ask it in"
job FAKE_IPTABLES=/home/tester/bin/iptables refused "iptables outside the system's directories" "outside the system's own directories"
job FAKE_UID=12345 refused "a configuration directory of another user" "belongs to another user"

case_dropin_not_loaded() {
  fresh "the drop-in not loaded by systemd"
  FAKE_NOT_LOADED=1 installer
  check [ "$status" = 1 ]
  contains "$work/out" "docker.service does not put the rule in place before it starts"
  lacks "$work/log" "docker compose --project-directory"
}
job case_dropin_not_loaded

case_other_repository() {
  fresh "a checkout of another repository"
  installer
  git -C "$work/home/easi.run" remote set-url origin https://example.com/other.git
  installer
  check [ "$status" = 1 ]
  contains "$work/out" "is a checkout of another repository"
}
job case_other_repository

job PINNED=753c55b42c32 refused "a pinned commit that is not a whole commit id" "this install.sh names no commit to install"
job IMAGES="" refused "no pinned images" "this install.sh does not name an image for each of admin controller display agent proxy browser"
job IMAGES="$(digests a | cut -d' ' -f1-5)" refused "a role without a pinned image" "does not name an image for each of"
job IMAGES="$(digests a | sed 's/^admin@/display@/')" refused "the roles' images in another order" \
  "names display@sha256:$(printf 'a%.0s' {1..64}) where it should name admin@sha256:<digest>"
job IMAGES="$(digests A)" refused "a digest that is not lowercase hexadecimal" "where it should name admin@sha256:<digest>"
job IMAGES="$(digests a | sed 's/^admin@sha256:a/admin@sha256:/')" refused "a digest that is too short" \
  "where it should name admin@sha256:<digest>"
job IMAGES="$(digests a | sed 's/^admin@sha256:/admin:latest@sha256:/')" refused "a tag beside the digest" \
  "where it should name admin@sha256:<digest>"

case_compose_variables() {
  fresh "Compose variables in the caller's shell"
  COMPOSE_FILE=elsewhere.yaml COMPOSE_PROJECT_NAME=other COMPOSE_PROFILES=bench installer
  check [ "$status" = 0 ]
  lacks "$work/log" "compose with the shell"
}
job case_compose_variables

case_image_changed_after_interruption() {
  fresh "a build that changed a session image after an interrupted run"
  installer
  FAKE_UP_STATUS=1 installer
  again
  check [ "$status" = 0 ]
  contains "$work/log" "--force-recreate controller admin"
  contains "$work/out" "A session image changed (a newer Chrome, say): the controller and the admin start afresh; live browser sessions end."
}
job case_image_changed_after_interruption

case_controller_recreated() {
  fresh "the controller recreated by a change of its own"
  installer
  FAKE_NEW_CONTROLLER=1 again
  check [ "$status" = 0 ]
  contains "$work/log" "--force-recreate admin"
  lacks "$work/log" "--force-recreate controller"
}
job case_controller_recreated

case_uninstall() {
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
  for image in $(digests a); do contains "$work/log" "docker image rm $registry-$image"; done
  check [ ! -s "$work/pulled" ] && check [ ! -s "$work/tags" ] && check [ ! -s "$work/built" ]
  contains "$work/log" "docker ps -aq --filter label=run.easi.instance=easirun"
  lacks "$work/log" "docker compose"
  contains "$work/out" "easi.run is removed from this server."
  contains "$work/out" "Kept: $work/home/easi.run, the checkout"
  lacks "$work/out" "not this installation's"
  # Every session's container is gone before the rule that keeps its pages off this server.
  first_rule="$(grep -n "^iptables -w -t raw -D" "$work/log" | head -n 1 | cut -d: -f1)"
  last_container="$(grep -n "^docker container rm" "$work/log" | tail -n 1 | cut -d: -f1)"
  check [ "$last_container" -lt "$first_rule" ]
}
job case_uninstall

case_uninstall_without_terminal() {
  fresh "an uninstall without --yes, and no terminal to ask in"
  installer
  : >"$work/log"
  FAKE_OURS=abc123 installer --uninstall
  check [ "$status" = 1 ]
  contains "$work/out" "there is no terminal to ask in: run it in a terminal, or add --yes"
  check [ -e "$dropin" ]
  lacks "$work/log" "sudo"
  lacks "$work/log" " rm "
}
job case_uninstall_without_terminal

# Runs the uninstall on a terminal of its own, answering its question with $1.
prompted() { FAKE_OURS=abc123 in_env script -qec "bash $tmp/run --uninstall" /dev/null <<<"$1"; }
case_uninstall_prompted() {
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
}
job case_uninstall_prompted

case_uninstall_beside_other_mcp_server() {
  fresh "an uninstall beside another easi server in Claude Code"
  installer
  printf 'easi:\n  Command: ssh\n  Args: -T me@elsewhere docker exec -i easi-run-admin-1 admin mcp\n' >"$work/claude-easi"
  installer --uninstall --yes
  check [ "$status" = 0 ]
  check [ -e "$work/claude-easi" ]
  lacks "$work/log" "claude mcp remove"
  contains "$work/out" "Kept: Claude Code's server easi, which is not this installation's"
}
job case_uninstall_beside_other_mcp_server

case_uninstall_own_instance() {
  fresh "an uninstall of a controller with an instance of its own"
  installer
  FAKE_OURS=abc123 FAKE_CONTROLLER_ENV="CONTROLLER_INSTANCE=mine" installer --uninstall --yes
  check [ "$status" = 0 ]
  contains "$work/log" "docker ps -aq --filter label=run.easi.instance=mine"
  lacks "$work/log" "label=run.easi.instance=easirun"
}
job case_uninstall_own_instance

case_uninstall_beside_other_installation() {
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
}
job case_uninstall_beside_other_installation

case_uninstall_image_in_use() {
  fresh "an uninstall with an image still in use"
  installer
  FAKE_OURS=abc123 FAKE_IMAGE_IN_USE="*" installer --uninstall --yes
  check [ "$status" = 1 ]
  contains "$work/out" "docker image rm easi-run/admin:dev failed: remove the container that uses it"
  check [ -e "$dropin" ]
  contains "$work/rules" "$rule"
}
job case_uninstall_image_in_use

case_uninstall_nothing_installed() {
  fresh "an uninstall with nothing installed"
  installer --uninstall --yes
  check [ "$status" = 0 ]
  lacks "$work/log" "daemon-reload"
  lacks "$work/log" "docker container rm"
  lacks "$work/log" "docker image rm"
  contains "$work/out" "easi.run is removed from this server."
}
job case_uninstall_nothing_installed

job FAKE_UID=0 refused "an uninstall as root" "not as root" --uninstall --yes
job DOCKER_HOST=tcp://10.0.0.5:2375 refused "an uninstall on another Docker host" "DOCKER_HOST is tcp://10.0.0.5:2375" --uninstall --yes
job FAKE_SUDO_PASSWORD=1 refused "an uninstall where sudo needs a password, with no terminal" \
  "there is no terminal to ask it in" --uninstall --yes
job FAKE_IPTABLES=/home/tester/bin/iptables refused "an uninstall with iptables outside the system's directories" \
  "outside the system's own directories" --uninstall --yes
for args in "--yes" "--update --dry-run" "--uninstall --update" "--uninstall --yes --update" "--remove"; do
  # shellcheck disable=SC2086
  job FAKE_OURS=abc123 refused "the arguments $args" "usage: bash install.sh [--update | --dry-run | --uninstall [--yes]]" $args
done

case_containerd_moved() {
  fresh "containerd's root moved by its configuration"
  mkdir -p "$work/etc-containerd" "$work/srv-containerd" && printf "version = 2\n  root = '%s'\n" "$work/srv-containerd" >"$work/etc-containerd/config.toml"
  FAKE_CONTAINERD_CONFIG="$work/etc-containerd/config.toml" FAKE_MOVED_KB=1000000 installer
  check [ "$status" = 1 ]
  contains "$work/out" "($work/srv-containerd)"
}
job case_containerd_moved

case_no_containerd_root() {
  fresh "no containerd root of its own"
  rmdir "$work/containerd"
  FAKE_CONTAINERD_KB=1000000 installer
  check [ "$status" = 0 ]
}
job case_no_containerd_root

case_little_containerd_room() {
  fresh "little room in containerd's store"
  FAKE_CONTAINERD_KB=1000000 installer
  check [ "$status" = 1 ]
  contains "$work/out" "needs 10 GB free where Docker keeps its images ($work/containerd)"
}
job case_little_containerd_room

case_docker_image_store() {
  fresh "Docker's own image store"
  FAKE_DRIVER='[["Backing Filesystem","extfs"]]' FAKE_CONTAINERD_KB=1000000 installer
  check [ "$status" = 0 ]
}
job case_docker_image_store

case_little_memory() {
  fresh "little memory"
  printf 'MemTotal:        2000000 kB\n' >"$work/meminfo"
  installer
  check [ "$status" = 1 ]
  contains "$work/out" "needs 4 GB of memory"
}
job case_little_memory

settings_now() { tr '\n' ' ' <"$work/sysctl"; }
before="net.core.default_qdisc=fq_codel net.ipv4.tcp_congestion_control=cubic net.ipv4.tcp_slow_start_after_idle=1"

case_tcp_settings() {
  fresh "the TCP settings with BBR"
  installer
  check [ "$status" = 0 ]
  sysctl_file="$work/fakeroot/etc/sysctl.d/90-easi-run.conf"
  contains "$sysctl_file" "# previous: $before"
  for setting in net.core.default_qdisc=fq net.ipv4.tcp_congestion_control=bbr net.ipv4.tcp_slow_start_after_idle=0; do
    contains "$sysctl_file" "$setting"
    contains "$work/sysctl" "$setting"
  done
  check [ "$(tail -n 1 "$work/fakeroot/etc/modules-load.d/easi-run.conf")" = tcp_bbr ]
  current="the TCP settings kept by a second run"
  again
  check [ "$status" = 0 ]
  contains "$sysctl_file" "# previous: $before"
  current="the TCP settings put back by an uninstall"
  installer --uninstall --yes
  check [ "$status" = 0 ]
  check [ ! -e "$sysctl_file" ] && check [ ! -e "$work/fakeroot/etc/modules-load.d/easi-run.conf" ]
  check [ "$(settings_now)" = "$before " ]
}
job case_tcp_settings

case_tcp_settings_unwritable() {
  fresh "the TCP settings that cannot be written"
  mkdir -p "$work/fakeroot/etc"
  : >"$work/fakeroot/etc/sysctl.d"
  installer
  check [ "$status" != 0 ]
  check [ "$(settings_now)" = "$before " ]
  lacks "$work/log" "sysctl -q -w"
}
job case_tcp_settings_unwritable

case_tcp_settings_not_ours() {
  fresh "a settings file of that name that is not easi.run's"
  mkdir -p "$work/fakeroot/etc/sysctl.d"
  echo "vm.swappiness=10" >"$work/fakeroot/etc/sysctl.d/90-easi-run.conf"
  mkdir -p "$work/fakeroot/etc/modules-load.d"
  echo tcp_bbr >"$work/fakeroot/etc/modules-load.d/easi-run.conf"
  installer
  check [ "$status" = 1 ]
  contains "$work/out" "90-easi-run.conf is there and not easi.run's"
  check [ "$(cat "$work/fakeroot/etc/sysctl.d/90-easi-run.conf")" = "vm.swappiness=10" ]
  check [ "$(settings_now)" = "$before " ]
  current="a foreign settings file left by an uninstall"
  installer --uninstall --yes
  check [ "$status" = 0 ]
  check [ "$(cat "$work/fakeroot/etc/sysctl.d/90-easi-run.conf")" = "vm.swappiness=10" ]
  check [ "$(cat "$work/fakeroot/etc/modules-load.d/easi-run.conf")" = tcp_bbr ]
}
job case_tcp_settings_not_ours

case_tcp_without_bbr() {
  fresh "the TCP settings without BBR"
  FAKE_NO_BBR=1 installer
  check [ "$status" = 0 ]
  check [ ! -e "$work/fakeroot/etc/sysctl.d/90-easi-run.conf" ]
  lacks "$work/log" "sysctl -q -w"
  check [ "$(settings_now)" = "$before " ]
  lacks "$work/out" "BBR"
}
job case_tcp_without_bbr

case_udp_address() {
  fresh "the UDP address from the SSH session"
  summary_in_session "81.2.69.160 50123 52.14.0.7 22"
  check [ "$status" = 0 ]
  contains "$work/log" "compose env ADMIN_UDP_URL=https://52.14.0.7:9443 ADMIN_UDP_BIND=0.0.0.0"
  contains "$work/out" "Smoother on lossy networks: if your provider has a firewall for this server, let 9443/udp through it."
  current="the UDP address kept for a run outside SSH"
  again
  contains "$work/log" "compose env ADMIN_UDP_URL=https://52.14.0.7:9443"
  current="no UDP address from an IPv6 one, which the IPv4 publication does not reach"
  : >"$work/log"
  rm -f "$work/home/.config/easi-run/udp-url"
  summary_in_session "2a01:4f9::7 50123 2a01:4f9::1 22"
  lacks "$work/log" "ADMIN_UDP_URL=https"
  lacks "$work/out" "9443/udp"
  current="the UDP address the checkout's .env sets"
  printf 'ADMIN_UDP_URL=https://100.64.1.2:9443\n' >"$work/home/easi.run/.env"
  : >"$work/log"
  summary_in_session "81.2.69.160 50123 52.14.0.7 22"
  check [ "$status" = 0 ]
  lacks "$work/log" "ADMIN_UDP_URL=https://52.14.0.7"
  contains "$work/log" "compose env ADMIN_UDP_URL= ADMIN_UDP_BIND=0.0.0.0"
  contains "$work/out" "let 9443/udp through it"
  current="no UDP address from a private one"
  fresh "no UDP address from a private one"
  summary_in_session "81.2.69.160 50123 10.0.0.7 22"
  check [ "$status" = 0 ]
  lacks "$work/log" "ADMIN_UDP_URL=https"
  lacks "$work/log" "ADMIN_UDP_BIND=0.0.0.0"
  lacks "$work/out" "9443/udp"
}
job case_udp_address

case_port_ours() {
  fresh "the port held by this installation"
  FAKE_PORT_BUSY=1 FAKE_OURS=abc123 installer
  check [ "$status" = 0 ]
}
job case_port_ours

case_versions() {
  current="versions"
  # shellcheck disable=SC1091
  source "$root/install.sh"
  check major_at_least 28.0.1 28
  check major_at_least v2.39.1 2
  ! major_at_least 27.5.1 28 || fail "27 < 28"
  ! major_at_least "" 2 || fail "empty"
  ! major_at_least x.1 2 || fail "not a number"
}
job case_versions

# What each case said, in order, and the failures of all of them: a case that did not reach its end (it stopped at an
# error, or the run was interrupted) is one more.
report() {
  for i in "${!cases[@]}"; do
    [ -f "$tmp/$i.done" ] || printf 'FAIL %s: did not reach its end\n' "${cases[$i]% }" >>"$tmp/$i.log"
    [ ! -s "$tmp/$i.log" ] || cat "$tmp/$i.log" >&2
    failures=$((failures + $(grep -c '^FAIL ' "$tmp/$i.log" || true)))
  done
  if [ "$failures" -gt 0 ]; then
    echo "install.sh: $failures failed" >&2
    exit 1
  fi
  echo "install.sh: every case passed"
}
# Interrupted, it stops the cases still running, each a process group of its own (installers included), and says
# what every case said.
stop() {
  local group
  for group in $(jobs -p); do kill -- "-$group" 2>/dev/null || true; done
  wait
  report
}
trap stop INT TERM

# Every case at once, as many at a time as there are processors, each in a work directory and a process group of its
# own ($tmp/N), with its output in $tmp/N.log; one that reaches its end leaves $tmp/N.done.
set -m
running=0
for i in "${!cases[@]}"; do
  if [ "$running" -ge "$(nproc)" ]; then
    wait -n || true
    running=$((running - 1))
  fi
  (
    work="$tmp/$i"
    eval "${cases[$i]}"
    : >"$tmp/$i.done"
  ) >"$tmp/$i.log" 2>&1 &
  running=$((running + 1))
done
wait
report
