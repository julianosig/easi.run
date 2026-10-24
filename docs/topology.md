# Topology

[`design.md`](design.md) states the requirements; this page fixes the concrete identities, mounts, and templates the
implementation uses, including the per-session DevTools choice explained in [`agent-compatibility.md`](agent-compatibility.md).

## Identities

| uid | Who | Groups (supplementary) |
|---|---|---|
| 10001 (A) | Chrome and its D-Bus session and accessibility buses (`browser`); the agent in DevTools mode | `10011` wayland, `10012` bus, `10016` proxy clients (browser only) |
| 10002 (B) | the compositor (`display`) | `10011` wayland, `10013` agent-sockets, `10014` streams |
| 10003 (C) | the egress proxy (`proxy`) | `10016` proxy clients |
| 10004 (D) | the admin web application (`admin`) | `10014` streams, `10015` controller |
| 10005 (E) | the Cua Driver in the default mode (`agent`) | `10012` bus, `10013` agent-sockets |
| 10006 (F) | the controller (`controller`), the only Docker-socket holder | the host's docker group, `10015` controller |

In DevTools mode the agent runs as 10001:10001 with supplementary `10012` and `10013`; Chrome never holds `10013`.

## Long-lived services (`compose.yaml`)

- `admin` (D): publishes only HTTPS on `127.0.0.1:${ADMIN_PORT:-9443}`; mounts the `streams` volume at `/run/streams`
  and the `controller` volume at `/run/controller`. No Docker access.
- `controller` (F): no network (`network_mode: none`); the Docker socket; serves `/run/controller/controller.sock`.
- Volumes: `streams` (a tmpfs created `2770 10004:10014` by its volume options, so new directories keep the group), `controller`
  (`0770 10006:10015`).

## Per-session resources (created by the controller from fixed templates)

Every resource carries the labels `run.easi.instance=<instance>`, `run.easi.session=<id>` and `run.easi.role=<role>`;
names start with `<instance>-<id>-` (`CONTROLLER_INSTANCE`, default `easirun`). A session id is 12 lowercase hex characters.
Cleanup only ever matches its own instance, so two installations (a development stack and the public demo) can share
one Docker Engine without touching each other's sessions.

Networks:
- `easirun-<id>-egress`: an ordinary bridge with outbound access, IPv6 off, its host interface named `er-<id>` (the
  host rule in [`security.md`](security.md#boundaries) drops what it sends the host). Only `proxy`.
- The browser joins **no** network (`network_mode: none`): its namespace has only loopback. Chrome's proxy is
  `http://127.0.0.1:3128`, the browser launcher's forwarder, which carries each connection unchanged to the proxy's Unix
  socket, which accepts only uid 10001. The browser deliberately joins no Docker network: on any Docker network the engine
  places its embedded DNS resolver (a UDP and a TCP socket on 127.0.0.11) in the container, so the browser would hold a
  UDP socket and a resolver it could query; with no network there is no gateway, no DNS, and no UDP at all.
  Without DevTools, the launcher then runs itself and Chrome in their own user namespace (only uid and gid 10001,
  unchanged) and network namespace, whose links are loopback and a veth pair `eth0`/`eth1` with `192.168.1.100/24` on
  `eth0`, no IPv6 address but loopback's, and no route beyond that subnet: Chrome sees an online link (otherwise it reads
  offline, `navigator.onLine` false), and nothing in that namespace reaches anything but the forwarder. The buses start
  before, in the container's namespace, so their peers' uids stay what they are.

Volumes (local driver, `type=tmpfs`, `noexec,nosuid,nodev`):
- `easirun-<id>-wayland`: `uid=10002,gid=10011,mode=0750`. The browser's Wayland socket and the sound FIFO `audio`
  (`0660`). `display` rw; `browser` ro (it can connect and write samples but never create, replace, or remove).
- `easirun-<id>-agent`: `uid=10002,gid=10013,mode=0750`. The privileged Wayland socket and `cua-inject`. `display` rw;
  `agent` ro.
- `easirun-<id>-bus`: `uid=10001,gid=10012,mode=0750`. The session and accessibility bus sockets. `browser` rw; `agent` ro.
- `easirun-<id>-proxy`: `uid=10003,gid=10016,mode=0750`. The proxy's socket. `proxy` rw; `browser` ro.
- The stream socket lives in the `streams` volume, mounted into `display` with `subpath=<id>` (a directory the admin
  creates `0770 10004:10014` before asking for the session), so a display sees only its own directory.

Containers (all: read-only root, `cap_drop: ALL`, `no-new-privileges`, no devices, pinned image digests, `init`):
- `display` (B): `network_mode: none`; 2 CPUs (1 in a benchmark lane), 512 MiB (a recording encodes beside the live stream); `/tmp` tmpfs.
- `proxy` (C): `easirun-<id>-egress` only; listens on its Unix socket; 0.5 CPU, 128 MiB. With an egress, its standard
  input is open (`PROXY_UPSTREAM_STDIN=1`) until the controller writes the upstream there once; it then resolves names
  by DNS over HTTPS and connects through that upstream only.
- `browser` (A): no network; 2 CPUs and 2 GiB by default (`BROWSER_CPUS`, `BROWSER_MEMORY_MIB`); the browser
  seccomp profile; tmpfs `/tmp`, `/home/browser` (16 GiB, so pages see an ordinary storage quota; the memory limit
  bounds it), `/downloads`, and `/uploads` (1 GiB) with `noexec`; `--shm-size`
  512 MiB. PulseAudio runs as the same uid, its socket in `/home/browser/pulse`; the launcher serves the file chooser
  portal on the session bus and holds the browser socket's files global. Chrome is
  started as an ordinary maximized window at the screen's size (a profile seeded with that placement and the system
  title bar, so the compositor's empty server-side decorations replace Chrome's shadows), with its sandbox, the managed
  policy, `--proxy-server=http://127.0.0.1:3128`,
  `--proxy-bypass-list=<-loopback>`, and, only in DevTools mode, `--remote-debugging-port=9222` (loopback).
- `agent` (E, or A in DevTools mode): `pid: container:<browser>`; `network_mode: none` (default) or
  `container:<browser>` (DevTools mode); 0.5 CPU, 512 MiB; tmpfs `/tmp`. Its command, the execute-only (so
  non-dumpable) `agent-idle`, idles; the controller's only exec is `/usr/local/bin/agent-mcp` (which executes the
  execute-only `cua-driver mcp`, so the driver is non-dumpable).

## Controller operations (closed set)

`create_session`, `stop_session`, `inspect_session`, `agent_attach` (start the one MCP exec and relay its stdio),
`agent_terminate` (kill the agent container and prove it exited), `agent_restart`, `cleanup` (remove every labelled
orphan and report it). Anything else is refused and logged.
