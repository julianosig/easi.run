#!/usr/bin/env bash
# Tests install.sh without changing this machine: it sources the installer and runs it against stubs of sudo,
# systemctl, iptables, docker, ss, ps, df, curl, claude, id and uname in PATH, a fake root for the systemd files, and a
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
stub curl 'for a in "$@"; do [ "${next:-}" = 1 ] && { [ -s "$a" ] && exit 0 || exit 7; }; [ "$a" = --cacert ] && next=1; done; exit 7'
stub claude 'case "$1 $2" in
  "mcp get") [ -f "$WORK/claude-easi" ] && cat "$WORK/claude-easi" || exit 1 ;;
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
stub systemctl 'case "$1" in
  show) f="$FAKEROOT/etc/systemd/system/docker.service.d/easi-run-firewall.conf"
        [ -f "$f" ] && sed -n "s/^$4=//p" "$f" | sed "s/\\$\\$/\\$/g" ;;
esac'
stub sudo 'case "$1" in
  -n|-v) exit 0 ;;
  sh) echo /usr/sbin/iptables ;;
  */iptables) shift; exec iptables "$@" ;;
  install) shift; args=(); while [ $# -gt 0 ]; do case "$1" in -o|-g) shift 2 ;; *) args+=("$1"); shift ;; esac; done
           exec install "${args[@]}" ;;
  *) exec "$@" ;;
esac'
stub docker 'case "$1" in
  context) echo "unix://$FAKE_SOCKET" ;;
  version) echo "${FAKE_DOCKER_VERSION:-28.3.0}" ;;
  info) case "$*" in
      *DriverStatus*) echo "${FAKE_DRIVER:-[[\"driver-type\",\"io.containerd.snapshotter.v1\"]]}" ;;
      *) echo "${FAKE_OS:-Debian GNU/Linux 13 (trixie)}|${FAKE_SECURITY:-[\"name=seccomp,profile=builtin\"]}|$WORK" ;;
    esac ;;
  image) [ -f "$WORK/images" ] && cat "$WORK/images" || exit 1 ;;
  ps) echo "${FAKE_OURS:-}" ;;
  inspect) case "$*" in *IPAddress*) echo 10.201.0.2 ;; *) echo /easi-run-admin-1 ;; esac ;;
  exec) cat >/dev/null
    case "$*" in
      *setup-link) echo "${FAKE_SETUP_OUT-https://localhost:9443/#setup=one-time-token}"; exit "${FAKE_SETUP_STATUS:-0}" ;;
    esac
    [ -z "${FAKE_MCP_DOWN:-}" ] || exit 1
    echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-11-25\"}}" ;;
  compose) shift; [ "$1" = version ] && { echo 2.39.1; exit 0; }
    while [ "${1#--}" != "$1" ]; do shift 2; done
    case "$1" in
      build) [ -n "${FAKE_SAME_IMAGES:-}" ] || date +%N >>"$WORK/images"; exit "${FAKE_BUILD_STATUS:-0}" ;;
      up) exit "${FAKE_UP_STATUS:-0}" ;;
      cp) echo CERT >"$3" ;;
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
  rm -rf "${work:?}/home" "$work/fakeroot" "$work/rules" "$work/log" "$work/claude-easi" "$work/images"
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
installer() {
  status=0
  # shellcheck disable=SC2030
  HOME="$work/home" PATH="$work/bin:$base_path" LOG="$work/log" WORK="$work" FAKEROOT="$work/fakeroot" \
    FAKE_SOCKET="$work/docker.sock" FAKE_UID="${FAKE_UID:-$real_uid}" EASI_RUN_REPO="$origin" \
    bash "$work/run" "$@" >"$work/out" 2>&1 </dev/null || status=$?
}

fresh "a first install"
installer
check [ "$status" = 0 ]
contains "$work/out" "easi.run is running: release v0.1.0, in $work/home/easi.run."
check [ "$(git -C "$work/home/easi.run" rev-parse HEAD)" = "$first" ]
check [ "$(cat "$work/home/.config/easi-run/release")" = "release v0.1.0 $first" ]
check [ "$(stat -c %a "$work/home/.config/easi-run")" = 700 ]
# No password file: the person creates the password in the admin's first screen, from the setup link.
check [ ! -e "$work/home/.config/easi-run/password" ]
lacks "$work/log" " init"
contains "$work/log" "docker exec -i easi-run-admin-1 admin setup-link"
contains "$work/out" "Password:  create it at https://localhost:9443/#setup=one-time-token (one use, valid for up to 15 minutes; to print it again: docker exec -i easi-run-admin-1 admin setup-link)"
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
order="$(grep -nE '^(iptables -w -t raw -I|docker compose .* (build|up -d$)|curl |docker exec .* admin setup-link)' "$work/log" |
  cut -d: -f2- | uniq)"
check [ "$(echo "$order" | sed -n 1p)" = "iptables -w -t raw -I PREROUTING 1 -i er-+ -m addrtype --dst-type LOCAL -j DROP" ]
echo "$order" | sed -n 2p | grep -q " build$" || fail "build second: $order"
echo "$order" | sed -n 3p | grep -q " up -d$" || fail "up third: $order"
echo "$order" | sed -n 4p | grep -q "^curl " || fail "the admin answering fourth: $order"
echo "$order" | sed -n 5p | grep -q "admin setup-link$" || fail "the setup link last: $order"
lacks "$work/log" "systemctl restart"
contains "$work/log" "docker compose --project-directory $work/home/easi.run --project-name easi-run --file $work/home/easi.run/compose.yaml build"
contains "$work/log" "curl --noproxy * -fsS --max-time 5 --cacert $work/home/.config/easi-run/tls-cert.pem"
lacks "$work/log" "curl -k"
contains "$work/log" "claude mcp add --scope user easi -- docker exec -i easi-run-admin-1 admin mcp"
contains "$work/out" "MCP:       ready (Claude Code: easi)"
contains "$work/out" "claude mcp add --scope user easi -- ssh -T tester@box docker exec -i easi-run-admin-1 admin mcp"
contains "$work/out" "codex mcp add easi -- docker exec -i easi-run-admin-1 admin mcp"

check grep -q "compose.yaml up -d --no-deps --force-recreate controller admin" "$work/log"

current="a second run at the same release"
: >"$work/log"
FAKE_SAME_IMAGES=1 installer
lacks "$work/log" "--force-recreate controller"
check [ "$status" = 0 ]
lacks "$work/log" "claude mcp add"
check [ "$(grep -c 'er-+' "$work/rules")" = 1 ]

fresh "a password set already"
FAKE_SETUP_STATUS=3 FAKE_SETUP_OUT="" installer
check [ "$status" = 0 ]
contains "$work/out" "Password:  set already"
lacks "$work/out" "#setup="
contains "$work/out" "ssh -N -L 9443:127.0.0.1:9443 tester@box, then open https://localhost:9443"
check [ ! -e "$work/home/.config/easi-run/password" ]

for out in "" "admin setup-link: no setup link is ready" "not a link"; do
  for code in 1 0; do
    fresh "admin setup-link failing (exit $code, output \"$out\")"
    FAKE_SETUP_STATUS="$code" FAKE_SETUP_OUT="$out" installer
    check [ "$status" = 0 ]
    contains "$work/out" "easi.run is running"
    contains "$work/out" "Password:  create it at the link this prints: docker exec -i easi-run-admin-1 admin setup-link"
    contains "$work/out" "then open https://localhost:9443"
    [ -z "$out" ] || lacks "$work/out" "$out"
  done
done

fresh "a failing build"
FAKE_BUILD_STATUS=1 installer
check [ "$status" != 0 ]
lacks "$work/log" " up -d"
lacks "$work/out" "easi.run is running"

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
: >"$work/log"
FAKE_SAME_IMAGES=1 installer
check [ "$status" = 0 ]
contains "$work/log" "--file $work/home/easi.run/compose.yaml --file $work/home/easi.run/compose.override.yaml --file $work/home/easi.run/compose.egress.yaml up -d"

for line in "EGRESS_FILE=/srv/egress.json" "export EGRESS_FILE=/srv/egress.json" "  EGRESS_FILE = /srv/egress.json"; do
  fresh "an egress file named in .env: $line"
  installer
  printf '%s\n' "$line" >"$work/home/easi.run/.env"
  : >"$work/log"
  FAKE_SAME_IMAGES=1 installer
  contains "$work/log" "--file $work/home/easi.run/compose.egress.yaml up -d"
done

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
work_origin_tags="$(git -C "$origin" tag)"
for tag in $work_origin_tags; do git -C "$origin" tag -d "$tag" >/dev/null; done
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

refused() {
  fresh "$1"
  shift
  installer
  check [ "$status" = 1 ]
  contains "$work/out" "$1"
  lacks "$work/log" "sudo install"
}
FAKE_UID=0 refused "root" "not as root"
FAKE_ARCH=aarch64 refused "another architecture" "needs Linux on x86-64"
FAKE_DOCKER_VERSION=27.5.1 refused "an older Docker" "Docker Engine 28 or later"
FAKE_SECURITY='["name=seccomp,profile=builtin","name=rootless"]' refused "rootless Docker" "rootless Docker is not supported"
FAKE_OS="Docker Desktop" refused "Docker Desktop" "Docker Desktop is not supported"
FAKE_DISK_KB=1000000 refused "a small disk" "needs 20 GB free"
FAKE_PORT_BUSY=1 refused "a busy port" "port 9443 is in use"

fresh "a build that changed a session image after an interrupted run"
installer
date +%N >>"$work/images"
FAKE_SAME_IMAGES=1 FAKE_UP_STATUS=1 installer
: >"$work/log"
FAKE_SAME_IMAGES=1 installer
check [ "$status" = 0 ]
contains "$work/log" "--force-recreate controller admin"

fresh "the controller recreated by a change of its own"
installer
: >"$work/log"
FAKE_SAME_IMAGES=1 FAKE_NEW_CONTROLLER=1 installer
check [ "$status" = 0 ]
contains "$work/log" "--force-recreate admin"
lacks "$work/log" "--force-recreate controller"

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
