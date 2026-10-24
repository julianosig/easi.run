# Security model

What easi.run protects, from whom, what holds each boundary, and which test proves it. The requirements are in
[design §4](design.md#4-security-requirements); the exact uids, mounts, and limits are in [`topology.md`](topology.md).
To report a vulnerability, see [`SECURITY.md`](../SECURITY.md).

## Assets

- The person's control of the browser, and whatever they type or paste.
- The host and its Docker engine.
- Other browser sessions on the same installation.
- The agent's integrity and its view of the page.

## Adversaries

1. A malicious page, up to a compromised Chrome renderer or browser process, running as uid A.
2. A hostile Wayland client on either compositor socket.
3. A malicious or modified viewer: someone with the admin password but without the lease, or a lease holder with a
   modified client (in the public demo, any visitor), including the camera pictures and microphone packets it sends a
   recording.
4. A network attacker between the person and the admin application.
5. Another local user of the host.

## Boundaries

**Browser to everything else.** Chrome keeps its own sandbox, under the browser seccomp profile (Docker's default
plus the namespace calls the sandbox needs, without the 32-bit x86 ABI or `socketcall`, through which a socket's
address family would escape the profile's check). Its container has no capability, no device, a read-only root, no shell,
and **no network at all**. Its only egress is a loopback forwarder to the proxy's Unix socket, and the proxy accepts
only uid A on that socket. Without DevTools, Chrome and the forwarder run in a user and network namespace of the
launcher's whose only link is a veth pair reaching nothing but itself, so Chrome reads online; that namespace maps only
uid and gid A and the launcher drops every capability in it before Chrome starts
(`security::socket_inventories_show_only_the_intended_endpoints`, `security::the_browser_reaches_nothing_but_the_proxy`).
Pages see an ordinary storage quota: the profile's tmpfs is 16 GiB, Chrome holds each site to about 192 MiB, and the
browser's memory limit bounds what all sites store together
(`security::a_site_sees_an_ordinary_quota_but_stores_only_a_bounded_amount`). The proxy resolves names itself and
connects to the address it resolved, so DNS rebinding cannot redirect it. It refuses private, loopback, link-local,
and other special addresses on every resolution, and it connects only to ports 80 and 443. One host rule keeps
sessions off the host's own services, its public addresses included: a packet from a session's bridge to one of them
is delivered to the host itself, through `INPUT`, and every session bridge's interface is named `er-<session>`, so
`iptables -I INPUT -i er-+ -j DROP` drops all of them (IPv4 is all there is to drop: the session networks have no
IPv6). The host's firewall configuration keeps it across reboots, and Docker's default userland proxy
(`"userland-proxy": true`, as Docker ships it) receives what sessions send the host's published ports on the host, so
the same rule covers them. Docker's DNS resolver lives in each container's own namespace and is unaffected.

**Egress through an upstream.** A deployment may name upstream proxies (`ADMIN_EGRESS_FILE`, `compose.egress.yaml`),
and the person picks one per session; the browser then says it is where the operator states that egress leaves (its
time zone and languages), and Chrome's own location service places it by the upstream's address. The admin reads the
file once, without following a link or blocking on a FIFO, only if it is a regular file owned by its uid or root that
its group may at most read and others not at all, and never quotes it in an error. The session's proxy receives
its upstream on standard input from the controller, never in its environment (which `docker inspect` shows); the
session is created only once the proxy accepted it, and a proxy without a valid one exits. Every connection then goes
through the upstream and none directly, even when the upstream fails; names resolve through it too, by DNS over HTTPS
to 1.1.1.1 (its certificate verified), so no name is resolved from the deployment's network; and every answer and
literal is vetted as before, so pages still reach only public addresses. An `https`
upstream receives its credentials over TLS verified against the public roots
(`security::a_session_leaves_only_through_its_egress_and_says_it_is_there`).

**Browser to compositor.** Chrome reaches only its own Wayland socket, which offers no privileged protocol. The
compositor never maps client memory: it reads `wl_shm` pools with `pread` and accepts only regular files on tmpfs or hugetlbfs
no shorter than declared. The agent's virtual-keyboard keymaps must be such descriptors too, but are never read or
compiled: keys go through the seat's own keymap, and a few hundred bytes of XKB can make libxkbcommon allocate
gigabytes or abort.
It bounds:

- pool size and count, buffers, surfaces, pixel copies and live objects, across all of a socket's clients;
- buffer geometry;
- damage (past its bound, pending a commit or between frames, it collapses into the whole surface);
- commits a synchronized subsurface makes before its parent's commit applies them;
- rectangles per region and types per data source;
- popup positioners' coordinates and sizes, and a popup's chain of parents at each placement (every parent alive
  and already a popup or the toplevel at the root, at most the surface bound deep), before smithay adds them up;
- unused descriptors;
- keyboards a client creates (4), and the events it leaves unread (512 KiB and 128 descriptors);
- requests per turn, and live protocol objects of every interface;
- the toplevel handles made for the agent, which change at most once per composition;
- clients per socket, and the display's descriptors (it sets its own limit, sized for every client's bound).

A client that crosses a bound is disconnected. Every pixel a viewer receives is copied into compositor memory before
it is encoded.

**Browser's sound.** PulseAudio runs in the browser container as uid A, so it holds nothing Chrome lacks: three
modules, no module loading by clients, no shared memory, no real-time scheduling, and its socket in a directory private
to the container. A compromised Chrome can take it over and gains nothing. What reaches the display is a FIFO in the
Wayland directory, which the browser mounts read-only, so it can write to the FIFO but never replace it; the display
creates it and checks it is its own FIFO. The display parses nothing in it: every byte sequence is valid sound. It
reads at most real time and discards a bounded amount per turn, so a writer that floods the FIFO (or grows its buffer
to the 1 MiB an unprivileged writer may) costs the display bounded work and is not heard
(`audio::a_writer_flooding_the_fifo_costs_the_display_bounded_work_and_is_not_heard`). Sound goes only from the browser
to viewers: the page gets no microphone (a person's microphone goes only into their own recording, mixed on the
display), and audio capture stays blocked by policy, since the sound server's monitor of
the browser's own output would otherwise let a page with the permission record other tabs. Video capture may be asked
for, as in any Chrome, but there is no camera. Notifications and geolocation are asked for, as in a fresh profile; the prompt shows above the page and only the person or the agent answers it. Chrome keeps its own Google API keys, so a position a page was allowed comes from Google's location service, which places the browser by the address its traffic leaves from (that request goes through the session's proxy like any other); Chrome sends it only for a page granted the permission, and component updates, which would also use those keys, stay off by policy. The browser's time zone and languages are the deployment's (`ADMIN_TIMEZONE`, `ADMIN_LANGUAGES`), or, for a session that leaves through an egress, that egress's; they are checked by the admin at start, by the controller for every session, and by the launcher.

**Clipboard.** What the page copies reaches only the connection holding control, at the epoch of the copy, checked by
the display and again by the admin as it leaves; nothing copied while the agent holds control is collected. It lands on
the person's clipboard automatically only when they turned that on, so a hostile page cannot silently replace what the
person copied (`control::a_long_paste_arrives_whole_and_what_the_page_copies_reaches_the_holder_alone`). The public
demo is the exception: it copies automatically, because its pages are the operator's own walled garden. The
client never reads the person's clipboard; a paste comes from their own paste event.

**Downloads.** The display never opens a path the browser can write: the browser's launcher streams each download over
one end of a Unix stream socket pair, which the display checks (a Unix stream socket) and reads only with
`MSG_DONTWAIT`, a flag of each call, so the launcher cannot make the compositor block. The files global is granted
once, to the launcher, before Chrome starts, and never on the agent's socket. What the launcher tells the holder
unasked (a download offered or gone, a file chooser opened or closed) is dropped while the holder's queue is full,
never ending the holder's connection, and so never its control. A download goes only to the connection
holding control, at its lease epoch, checked by the display and again by the admin; the admin gives that viewer a
single-use 256-bit ticket, and its download endpoint answers only the same login while that viewer still holds control
at that epoch (the cookie is `SameSite=Strict`, and a cross-site `Sec-Fetch-Site` is refused), checks the lease and the
login again before every piece, and errors instead of ending cleanly when anything is cut, so the person's browser
never keeps a partial file as complete. The response is an `attachment` of `application/octet-stream`, `no-store`,
never rendered by the admin's origin (`files::a_download_reaches_the_holder_byte_exact_and_nobody_else`,
`files::a_download_fails_when_control_moves_and_is_not_offered_to_the_next_holder`). The person gets exactly the bytes
of Chrome's final file or a failed download, never a mix of two of its writing sessions. The size comes from the
launcher's own DevTools pipe (`--remote-debugging-pipe`, descriptors only Chrome inherits; it enables nothing but
the `Page` domain of each page, and never changes Chrome's download handling); a wrong size can only fail that
download, since the response's last piece waits for the confirmed end. Inspecting pages needs
`DeveloperToolsAvailability` 1; the DevTools frontend stays refused by policy
(`security::devtools_shortcuts_open_nothing`), and without DevTools mode no other client has an endpoint. The browser's tmpfs stays the
memory bound; the forwarder only paces Chrome's network to the person
(`files::a_download_far_larger_than_the_browsers_memory_streams_at_the_persons_pace`). Chrome still blocks dangerous
file types.

**Uploads and the file chooser.** The file chooser portal belongs to the browser's launcher: owned before Chrome starts
without replacement, callable only by the browser's uid, and answering only callers the bus attests by process
descriptor to be Chrome's processes, so the DevTools agent (the same uid, another container) can neither ask the
person for files nor cancel Chrome's requests. Choosing a directory and saving several files at once are refused; a
single save is answered with a free name in the downloads directory, so it becomes a download. Uploads come only from the
connection holding control, at its epoch, in order and within a window the display acknowledges, so the display holds
at most a window per upload and never a file; the launcher stores them in its private tmpfs, created exclusively and
never through a link, and only a file that arrived whole is given to the page. A drop offers only paths the launcher
stored for this holder, and waits for the page to accept before dropping; the drag-over before it offers placeholder
paths where nothing exists, and no input can drop it
(`files::a_file_given_to_the_pages_chooser_or_dragged_and_dropped_on_it_reaches_the_page_byte_exact`).

**Recording inputs.** A recording belongs to the connection holding control, view only included (a recording needs
the lease, not input), so any lease holder, and in the public demo any visitor, can send the display camera pictures as
JPEG (`record-camera`) and microphone packets as Opus (`record-microphone`) with a client of its own. The display
decodes them in its own process: pictures with zune-jpeg 0.5.15 (Rust, pinned), packets with libopus 1.5.2 (C, built
from source and statically linked, called through `display/src/stream/opus.rs`). The admin relays them only from the
holder at the current epoch of a session with recording on, and the display checks the lease again. What bounds them:

- a picture is 1 to 60 KiB and a packet 1 to 1,275 bytes, or the message is refused as it is parsed;
- one recording at a time per session, taking pictures only if it was started with a camera and packets only if with
  a microphone, through a 128-entry queue to its worker that drops what does not fit;
- a picture within 50 ms of the last one the worker took, or stamped more than 2 s ahead, is dropped undecoded, and
  one whose header gives a side over 640 pixels is refused before its pixels are allocated
  (`display/src/stream/camera.rs`);
- a packet decodes to at most one 20 ms frame, and a longer one is refused;
- the worker runs at a lower priority than the compositor and the live encoders, within the display's 2 CPUs and
  512 MiB.

The size bounds are proven by the protocol's `vectors::refuses_malformed_messages` and the packet bound by
`opus::tests::a_packet_decodes_to_one_frame_and_nonsense_is_refused` (unit tests); the fuzz targets `camera_picture`
and `microphone_packet` include the display's own decode functions, and libopus itself is fuzzed upstream (OSS-Fuzz).
Both decoders run in the display, which has no network and reaches only its own session's directories.

**Agent.** The privileged socket (foreign-toplevel lists, screencopy, virtual pointer and keyboard) and `cua-inject`
live in a directory only the display and the agent can enter. The lease gates the agent's input. While a person holds
control, the agent is blocked. What the agent may still do then cannot hold the compositor: its virtual pointer
presses only mouse buttons (another code disconnects it), and a screen copy is made at a composition, one per
composition and at most one waiting per client (`hostile::a_screencopy_flood_keeps_the_compositor_responsive`). A
takeover waits up to 5 seconds for the in-flight call to end. If the call does not end, the agent container is killed
and proven stopped. If that also fails, the session ends.

DevTools mode ([`agent-compatibility.md`](agent-compatibility.md)) is a per-session choice, off by default, for the
driver's `browser_*` tools: the agent then runs as uid A in its own container, sharing the browser's PID and network
namespaces. Its main process and the driver run execute-only binaries, so the kernel marks both non-dumpable and
Chrome's uid gets `EACCES` on their `/proc/<pid>/{root,fd,environ}` (`security::chromes_uid_cannot_reach_into_the_agent_through_proc`).

Chrome's windows (a person's `Ctrl+N`, a page's popup after a click) stack maximized, the newest on top and focused,
and closing one returns to the one below. The driver's input by PID names no window, so it is refused while more than
one window is shown (`control::a_new_window_shows_on_top_and_closing_it_returns_to_the_page`); desktop-scope input
reaches the window shown.

**Viewers.** Input is accepted only from the lease holder's connection at the current epoch. The admin and the display
each enforce this independently. A watcher's input, a stale epoch, and an unknown message are refused or close the
stream.

**Admin application.**

- HTTPS only, published on the host's loopback by `compose.yaml`; the public demo's admin publishes no port and
  listens on an internal network that only the tunnel joins (see [Public demo](#public-demo)).
- One enrolled password, hashed with Argon2id, with backoff on failed logins.
- A `__Host-` cookie (`Secure`, `HttpOnly`, `SameSite=Strict`) with a 30-minute idle and a 12-hour absolute expiry.
- An exact `Origin` check plus a CSRF header on every state change a login makes; enrolling and logging in, which come
  before there is a login, need the exact `Origin`; a WebSocket upgrade needs the exact `Origin` and a
  live login cookie (browsers send no custom header on an upgrade).
- Logout and expiry close the login's streams and revoke its lease.
- No Docker access.

**Agent tokens.** The administrator may create up to 32 bearer tokens for agents (never in the demo). Each is 32
random bytes; the admin keeps only its SHA-256, in `tokens.json` (mode 0600, replaced atomically), and compares a
presented token with every stored hash in constant time. A token reaches only the agent API (`/api/agent/`) and the MCP
endpoint (`/mcp`), and through them only the sessions it started: every session records the principal that started it
(the administrator, a token, or a demo visitor; `admin/src/app.rs`), and only the administrator reaches every one. The
viewer stream, downloads, takeovers, settings, the network simulator and token management stay behind the login. A
bearer request with an `Origin` other than the admin's own is refused. Its tool calls go through the same call service
as the administrator's (`admin/src/service.rs`): one at a time, blocked while a person holds control, and redacted
(field names and driver errors included).
Revoking a token writes the revocation before its sessions are stopped; a session still starting for it is stopped
instead of listed, and a call waiting for its turn is refused once admitted. A token carries 256 random bits, so
guessing one is out of reach however fast it is tried; a failed bearer check is answered `401`, and failures never
slow the administrator's login or a valid token.

The controller serves only the admin's uid. It refuses any operation, image, mount, namespace, capability, device, or
identity outside its fixed templates, and any session past `CONTROLLER_MAX_SESSIONS` (1 to 16, default 16), so a
faulty or compromised admin runs no more sessions than that cap, each within its template's CPU, memory, and process
limits.

**What the agent learns of the person.** After a takeover, what the person typed or pasted, as it stood when they
left the value (Enter, Tab, a click, Escape or a move), is replaced by `[redacted]` in every later result the agent's
caller receives; matching is case-sensitive, on four or more characters. Password fields' values never reach the
agent's caller. The set holds at most 256 KiB; once it is full, every later agent result is replaced by a notice that
holds nothing of the page. From the first takeover on, no screenshot reaches the agent's caller. The redaction set lives in `admin/src/redact.rs`: the viewer
relay (`admin/src/relay.rs`) records what the lease holder types and pastes, and the one agent call service
(`admin/src/service.rs`), which the administrator's page, the agent API and the MCP endpoint all use, filters every
agent result through it. A new input path must record its text there too, and a new way to call the agent must go
through that service.

## Public demo

The live demo at easi.run runs this stack in demo mode (`ADMIN_DEMO_SECONDS`; its deployment is
[`demo/compose.yaml`](../demo/compose.yaml)), and it is in scope ([`SECURITY.md`](../SECURITY.md)). What it changes:

- **Logins without a password, with a deadline.** Opening the demo starts a visit whose login needs no password. The
  login ends at the visit's deadline (`ADMIN_DEMO_SECONDS`, 300 s there) or on logout, and the visit's session is
  stopped then; a visit whose browser nobody has watched for 30 seconds ends early. Everything an administrator does
  (more sessions, settings, egresses, the network simulator, agent calls, other visitors' sessions) is refused
  ([`admin/src/demo.rs`](../admin/src/demo.rs)).
- **One place per client, told apart by the tunnel.** A client is an IPv4 address, or an IPv6 /64 network, taken from
  the `CF-Connecting-IP` header Cloudflare's tunnel sets at its edge; a request without it is refused. Each client
  holds at most one place (a visit starting, under way, or being cleaned up after) and starts at most 10 visits in
  10 minutes. The admin trusts that header, so it is reachable only through the tunnel: it publishes no port, and its
  only network, `origin`, is internal and joined by nothing but the tunnel.
- **Framing.** Only the origins in `ADMIN_FRAME_ANCESTORS` (easi.run's landing) may frame the demo, through the
  `frame-ancestors` directive of its Content-Security-Policy. Outside demo mode the admin refuses that variable, and no
  page may frame it.
- **The walled garden.** With `CONTROLLER_BENCH_FIXTURE_ONLY=1`, every session proxy sends every destination, whatever
  name or address the page asks for, to the one internal pages server ([`demo/nginx.conf`](../demo/nginx.conf)), and
  the session networks are internal, so not even a proxy fault reaches out; the controller refuses to start unless the
  pages server's network is internal too. Chrome accepts that server's self-signed certificate by its pinned key
  (`CONTROLLER_BENCH_SPKI_FILE`).
- **An age limit the admin cannot lift.** `CONTROLLER_MAX_SESSION_SECONDS` (360 s there) stops any older session,
  whatever the admin does, and at start the controller removes everything an earlier run of the installation left.
  The controller accepts that variable only in the walled garden.

## Evidence

Every item is proven by a test against the running stack (`scripts/e2e.sh`, see [CONTRIBUTING](../CONTRIBUTING.md))
unless noted.

| Requirement (design §4) | Test |
|---|---|
| 1, 2: no root, capability, device, or writable root; distinct uids; only the Wayland directories shared, read-only for the browser | `security::containers_run_unprivileged_with_distinct_identities` |
| 3: socket inventories; no UDP in `browser` or `display` | `security::socket_inventories_show_only_the_intended_endpoints` |
| 4: no direct egress from the browser; private and loopback refused through the proxy, also by a rebinding name | `security::the_browser_reaches_nothing_but_the_proxy`; `proxy` unit tests |
| 5: the controller's closed set; no Docker access for the admin | `security::the_controller_refuses_everything_outside_its_closed_set`; `controller` unit tests |
| 6: uid A cannot reach the privileged socket; Chrome's socket has no privileged global | `security::the_privileged_socket_is_out_of_the_browsers_reach`; `hostile::a_connection_from_another_uid_is_refused` |
| 7: every decoded byte comes from compositor-owned pixels or the sound's samples; a flooded sound FIFO costs bounded work | by construction (`display/src/shm.rs`, `display/src/compose.rs`); `hostile::a_pool_truncated_before_the_frame_disconnects`; `audio::a_tone_in_the_page_reaches_the_viewer_as_opus_and_stops_after_it`; `audio::a_writer_flooding_the_fifo_costs_the_display_bounded_work_and_is_not_heard` |
| 8: a modified watcher injects nothing | `control::a_modified_watcher_injects_nothing_and_a_revoked_lease_is_refused` |
| 9: only the holder at the current epoch injects | the same test; `control::the_lease_is_revoked_when_its_holder_disconnects` |
| 10: hostile clients; one hour of fuzzing per parser | `tests/tests/hostile.rs`; the fuzzing run below |
| 11: Chrome's sandbox; internal schemes blocked by policy | `security::chromium_keeps_its_sandbox_and_blocks_internal_schemes` |
| 12: enrollment, CSRF, expiry, logout closing streams, crash cleanup | `admin` unit tests; `security::logout_closes_live_streams`; `security::a_restarted_admin_removes_what_the_last_run_left` |
| Agent tokens: their own sessions only, nothing behind the login, takeover and redaction, revocation | `admin` unit tests (`tokens`, `app`, `mcp`); `agents::a_token_reaches_neither_another_s_sessions_nor_anything_behind_the_login`; `agents::a_person_s_takeover_pauses_a_token_s_tools_until_they_give_back`; `agents::revoking_a_token_stops_its_sessions_and_one_still_starting` |

### Fuzzing

The targets live in `tests/fuzz` and run with libFuzzer through `cargo fuzz` (`tests/fuzz/run.sh SECONDS`). Every target
ran for 3,600 seconds, in parallel, with no crash, timeout, or out-of-memory: the first five on 2026-10-20, the
recording's two on 2026-10-24:

| Target | Parser | Runs |
|---|---|---|
| `client_message` | the wire protocol, client to server | 1,189,667,781 |
| `server_message` | the wire protocol, server to client (including `config` and its codec) | 1,126,324,129 |
| `link_frames` | admin-display link frames and the bounded frame reader | 39,764,158 |
| `tile_records` | tiles messages: the records stream inflated under its declared length, then decoded | 71,947,572 |
| `websocket_frames` | WebSocket framing as the admin reads it | 19,847,567 |
| `camera_picture` | a recording's camera pictures, decoded as the display does (zune-jpeg) | 72,327,539 |
| `microphone_packet` | a recording's microphone packets, decoded as the display does (libopus) | 137,739,249 |
