# Security model

What easi.run protects, from whom, what holds each boundary, and which test proves it. The requirements are in
[design §4](design.md#4-security-requirements); the exact uids, mounts, and limits are in [`topology.md`](topology.md).
To report a vulnerability, see [`SECURITY.md`](../SECURITY.md).

## Assets

- The person's control of the browser, and whatever they type or paste.
- The logins, site data, history and preferences their persistent sessions keep.
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
address family would escape the profile's check; `tests/tests/seccomp.rs`, such as
`seccomp::no_call_or_abi_reaches_sockets_around_the_rules`). Its container has no capability, no device, a read-only
root, no shell, and **no network at all** (`security::socket_inventories_show_only_the_intended_endpoints`,
`security::the_browser_reaches_nothing_but_the_proxy`). Its only egress is a loopback forwarder to the proxy's Unix
socket, and the proxy accepts only uid A on that socket (`proxy::tests::the_unix_listener_serves_only_its_client_uid`).
Without DevTools, Chrome and the forwarder run in a user and network namespace of the launcher's whose only
interfaces are its loopback and a veth pair reaching nothing but itself, so Chrome reads online; that namespace maps
only uid and gid A and the launcher drops every capability in it before Chrome starts (`browser/launch/src/netns.rs`).
The profile, downloads and uploads live in the browser container's tmpfs, inside its memory limit, which leaves it no
swap, and go with the session; a persistent session also saves its profile, encrypted, when it is stopped (see
**Persistent sessions** below). Every session container, the admin, the controller and `init` run with a core limit
of 1, soft and hard, which the kernel takes as no core dump at all: it writes no core file, and pipes nothing to a
collector the host's core pattern names, so no crashed process's memory (a page's cookies, what the person typed)
reaches the host's disk (`security::containers_run_unprivileged_with_distinct_identities` checks it for the session
containers, the admin and the controller; `compose.yaml` sets it for `init` too). easi.run's own programs (the admin,
the controller, the proxy, the agent's idle process, and the browser launcher before it starts Chrome) are also
non-dumpable.
Pages see an ordinary storage quota while each site stores a bounded amount: the profile's tmpfs is 16 GiB, Chrome
holds each site to 192 MiB, and the browser's memory limit bounds what all sites store together
(`security::a_site_sees_an_ordinary_quota_but_stores_only_a_bounded_amount` checks a quota of at least 10 GiB and a
site stopped between 64 and 256 MiB). The proxy resolves names itself and
connects to the address it resolved, so DNS rebinding cannot redirect it. It refuses private, loopback, link-local,
and other special addresses on every resolution, and it connects only to ports 80 and 443.

**Host services, a setup step.** One host rule keeps sessions off the services on every address assigned to the host
itself, the ports Docker publishes on them included: `iptables -t raw -I PREROUTING 1 -i er-+ -m addrtype --dst-type
LOCAL -j DROP`. Every session bridge's interface is named `er-<session>`, and the rule, first in the raw table's
`PREROUTING`, drops whatever one of them sends such an address before Docker's port redirects (the session networks
are IPv4 only). `install.sh` puts it first in a `docker.service` drop-in, before every start of Docker (Docker
appends its own rules after it), or the operator adds it by hand ([Quick start](https://docs.easi.run/#/quick-start))
and saves it in the host's firewall configuration. Docker's DNS resolver lives in each container's own namespace and is unaffected.

**Egress through an upstream.** The administrator adds upstream proxies (egresses) in the dashboard, and the person
picks one per session; the browser then says it is where that egress leaves (its time zone and languages), and
Chrome's own location service places it by the upstream's address. Only the administrator manages them: every
`/api/egresses` route takes the password login, and a change the exact origin and the CSRF token too, and is refused in
the demo and to agent tokens, which may only start a session through an egress by its name. The admin keeps them in
`egresses.json` in its state volume (mode 0600, replaced atomically, read only as a regular file of its own, never
through a link), credentials included, so a backup of that volume holds them; no answer, log line or error holds a
credential or the URL it came in, the listing showing only whether an egress has a user name and a password, and its
answers are not cached (`admin/src/egresses.rs`). An upstream may be any address the session networks reach, the
host's own excepted (the host rule above), so whoever holds the password decides where sessions' traffic goes, as the
operator did before.

An egress is saved, or given a new proxy, only once a check passed, and a session starts through it only once a check
of the very upstream it will use passed, just before (or, for an upstream the same for every session, within the last
30 seconds), before any of the session's containers exists; a failed check says why (unreachable, timeout, auth,
refused, tls, protocol, unexpected). The controller runs the check as one more closed operation, `check_egress`, from
where a session's proxy would reach the upstream: a network made as a session's (its `er-` interface, so the host
rule holds) and one container from a fixed, guarded template, the proxy image in its check mode, with no volume,
listener or supplementary group, 0.25 CPU, 64 MiB and 64 processes, given the upstream on standard input. It reaches
the upstream as a session's proxy does, opens a tunnel through it to the DNS-over-HTTPS resolver, resolves
`speed.cloudflare.com` through it, and fetches the metadata Cloudflare's speed test reads
(`https://speed.cloudflare.com/meta`, its certificate verified) through it, which confirms the upstream works and says
the address its traffic leaves from, that address's country and, when known, its region and city; the country suggests
the time zone and languages, and a region or city is kept only as a short place name. Its answer is held to a fixed shape, and
its container and network are removed whatever the outcome; at most four checks run at once, what one could not remove
(or may still make, when Docker never answered its creation) counting among them (`proxy/src/check.rs`, `controller/src/ops.rs`). A check passes for that moment
only: an upstream may fail later, which the session then handles as below.

The session's proxy receives its upstream on standard input from the controller, never in its environment (which
`docker inspect` shows); the session is created only once the proxy accepted it, and a proxy without a valid one
exits. Every connection then goes through the upstream and none directly, even when the upstream fails; the names
pages ask for resolve through it too, by DNS over HTTPS to 1.1.1.1 (its certificate verified), so none of them is
resolved from the deployment's network; and every answer and literal is vetted as before, so pages still reach only
public addresses (`security::an_egress_is_checked_before_use_and_a_session_leaves_only_through_it`,
`proxy::tests::with_an_upstream_every_connection_goes_through_it_by_the_vetted_address`). An `https` upstream receives
its credentials over TLS verified against the public roots (`proxy/src/upstream.rs`).

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
- keyboards a client creates (4), and the events it leaves unread (512 KiB of memory and 128 descriptors);
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
reads at most real time, so a writer that floods the FIFO is held to real time and heard like any sound; one that grows
its buffer (to the 1 MiB an unprivileged writer may) to run further ahead has what waits beyond 100 ms discarded
unheard, a bounded amount per turn. Either way the display does bounded work
(`audio::a_writer_flooding_the_fifo_costs_the_display_bounded_work_and_is_not_heard`). Sound goes only from the browser
to viewers: the page gets no microphone (a person's microphone goes only into their own recording, mixed on the
display), and audio capture stays blocked by policy, since the sound server's monitor of
the browser's own output would otherwise let a page with the permission record other tabs. The browser has no camera.
Notification and geolocation prompts show above the page, and only the person or the agent answers them; Chrome's
location request goes through the session's proxy like any other, and component updates stay off by policy. The
browser's time zone and languages are the deployment's (`ADMIN_TIMEZONE`, `ADMIN_LANGUAGES`), or, for a session that
leaves through an egress, that egress's; they are checked by the admin at start, by the controller for every session,
and by the launcher.

**Clipboard.** What the page copies reaches only the connection holding control, at the epoch of the copy, checked by
the display and again by the admin as it leaves
(`control::a_long_paste_arrives_whole_and_what_the_page_copies_reaches_the_holder_alone`); nothing copied while the
agent holds control is collected. While the person holds control, what the page copies goes to their clipboard, as it
would in their own Chrome, where the browser allows the write; otherwise a button offers it
(`client/src/ui/Session.svelte`). The client never reads the person's clipboard; a paste comes from their own paste
event.

**Downloads.** The display never opens a path the browser can write: the browser's launcher streams each download over
one end of a Unix stream socket pair, which the display checks (a Unix stream socket) and reads only with
`MSG_DONTWAIT`, a flag of each call, so the launcher cannot make the compositor block. The files global is granted
once, to the launcher, before Chrome starts, and never on the agent's socket. What the launcher tells the holder
unasked (a download offered or gone, a file chooser opened or closed) is dropped while the holder's queue is full,
never ending the holder's connection, and so never its control. A download goes only to the connection
holding control, at its lease epoch, checked by the display and again by the admin; the admin gives that viewer a
single-use 256-bit ticket, and its download endpoint answers only the same login while that viewer still holds control
at that epoch (the cookie is `SameSite=Strict`, and a `Sec-Fetch-Site` other than `same-origin` is refused), checks
the lease and the login again before every piece, and errors instead of ending cleanly when anything is cut, so the person's browser
never keeps a partial file as complete (`files::a_download_reaches_the_holder_byte_exact_and_nobody_else`,
`files::a_download_fails_when_control_moves_and_is_not_offered_to_the_next_holder`). The response is an `attachment`
of `application/octet-stream`, `no-store`, never rendered by the admin's origin (`admin/src/api.rs`). The person gets
exactly the bytes of Chrome's final file or a failed download, never a mix of two of its writing sessions. The size comes from the
launcher's own DevTools pipe (`--remote-debugging-pipe`, descriptors only Chrome inherits). On it the launcher
enables only the `Page` domain of each page, for its download events, and opens the `https` page the display asks
for in a tab of its own, closing the others; it never changes Chrome's download handling. A wrong size can only fail
that download, since the response's last piece waits for the confirmed end. Inspecting pages needs
`DeveloperToolsAvailability` 1; the DevTools frontend stays refused by policy
(`security::devtools_shortcuts_open_nothing`), and without DevTools mode no other client has an endpoint. The browser's tmpfs stays the
memory bound; the forwarder only paces Chrome's network to the person
(`files::a_download_far_larger_than_the_browsers_memory_streams_at_the_persons_pace`). Chrome still blocks dangerous
file types.

**Uploads and the file chooser.** The file chooser portal belongs to the browser's launcher: owned before Chrome starts
without replacement, callable only by the browser's uid, and answering only callers the bus attests by process
descriptor to be Chrome's processes, so the DevTools agent (the same uid, another container) can neither ask the
person for files nor cancel Chrome's requests. Choosing a directory and saving several files at once are refused; a
single save is answered with a free name in the downloads directory, so it becomes a download. A page's own "Save as"
is answered with its single-file kind (MHTML), which Chrome writes in place into that name: it is delivered only once
it ends with the closing boundary its header declares, so a save cut short never reaches the person
(`files::a_page_saved_with_chromes_save_as_reaches_the_holder_as_one_file`). Uploads come only from the
connection holding control, at its epoch, in order and within a window the display acknowledges, so the display holds
at most a window per upload and never a file; the launcher stores them in its private tmpfs, created exclusively and
never through a link, and only a file that arrived whole is given to the page. A drop offers only paths the launcher
stored for this holder, and waits for the page to accept before dropping; the drag-over before it offers placeholder
paths where nothing exists, and no input can drop it
(`files::a_file_given_to_the_pages_chooser_or_dragged_and_dropped_on_it_reaches_the_page_byte_exact`).

**Recording inputs.** A recording belongs to the connection holding control, view only included (a recording needs
the lease, not input), so any lease holder, and in the public demo any visitor, can send the display camera pictures as
JPEG (`record-camera`) and microphone packets as Opus (`record-microphone`) with a client of its own. The display
decodes them in its own process: pictures with zune-jpeg 0.5.15 (Rust, pinned), packets with libopus (C, built
from source and statically linked, called through `display/src/stream/opus.rs`). The admin relays them only from the
holder at the current epoch of a session with recording on, and the display checks the lease again. What bounds them:

- a picture is 1 to 60 KiB and a packet 1 to 1,275 bytes, or the message is refused as it is parsed;
- one recording at a time per session, taking pictures only if it was started with a camera and packets only if with
  a microphone, through a 128-entry queue to its worker that drops what does not fit;
- a picture within 50 ms of the last one the worker took, or stamped more than 2 s ahead, is dropped undecoded, and
  one whose header gives a side over 640 pixels is refused before its pixels are allocated
  (`display/src/stream/camera.rs`);
- a packet decodes to at most one 20 ms frame, and a longer one is refused;
- the worker runs at a lower priority than the compositor and the live encoders, within the display's CPU and memory
  limits ([topology](topology.md)).

The size bounds are proven by the protocol's `vectors::refuses_malformed_messages`, and a packet's decoding to one
frame and the refusal of an oversized or empty one by
`opus::tests::a_packet_decodes_to_one_frame_and_nonsense_is_refused` (unit tests); the fuzz targets `camera_picture`
and `microphone_packet` include the display's own decode functions, and libopus itself is fuzzed upstream (OSS-Fuzz).
Both decoders run in the display, which has no network and reaches only its own session's directories.

**Agent.** The privileged socket (foreign-toplevel lists, screencopy, virtual pointer and keyboard) and `cua-inject`
live in a directory only the display and the agent can enter. The admin sends the few lines the driver cannot (text
beyond ASCII, a click by element or window pixel the driver cannot deliver here, [agent
compatibility](agent-compatibility.md#what-the-admin-does-around-the-driver)) on its own stream link, which the display
serves under the same rules within the agent's call. The lease gates the agent's input. While a person holds
control, the agent is blocked; a viewer opened while an agent uses the session (a call placed or running, or one in the
last 30 seconds) only watches: opening a session takes control only from an agent idle that long, and never stops one.
What the agent may still do then cannot hold the compositor: its virtual pointer
presses only mouse buttons (another code disconnects it), and a screen copy is made at a composition, one per
composition and at most one waiting per client, so a flood of them leaves the compositor responsive
(`hostile::a_screencopy_flood_keeps_the_compositor_responsive`). A
takeover waits up to 5 seconds for the in-flight call to end. If the call does not end, the agent container is killed
and proven stopped. If that also fails, the session ends.

DevTools mode ([`agent-compatibility.md`](agent-compatibility.md)) is a per-session choice, off by default, for the
driver's `browser_*` tools: the agent then runs as uid A in its own container, in the browser's network namespace (it
shares the browser's PID namespace in every mode). Its main process and the driver run execute-only binaries, so the
kernel marks both non-dumpable and Chrome's uid gets `EACCES` on their `/proc/<pid>/{root,fd,environ}` (`security::chromes_uid_cannot_reach_into_the_agent_through_proc`).

Chrome's windows (a person's `Ctrl+N`, a page's popup after a click) stack maximized, the newest on top and focused,
and closing one returns to the one below. The driver's input by PID names no window, so it is refused while more than
one window is shown (`control::a_new_window_shows_on_top_and_closing_it_returns_to_the_page`); desktop-scope input
reaches the window shown.

**Viewers.** Input is accepted only from the lease holder's connection at the current epoch. The admin and the display
each enforce this independently. A watcher's input, a stale epoch, and an unknown message are refused or close the
stream.

**Admin application.**

- HTTPS, published on the host's loopback by `compose.yaml`; the public demo's admin publishes no port and
  listens on an internal network that only the tunnel joins (see [Public demo](#public-demo)).
- The direct path's UDP port (`admin/src/udp.rs`), which the admin answers on when `ADMIN_UDP_URL` is set, and which
  `compose.yaml` publishes on loopback unless `ADMIN_UDP_BIND` says every address (install.sh sets both when it knows
  the IPv4 address the person reached the server at over SSH, or `.env` sets the first). It carries only a viewer's media, the sound and the viewer's acknowledgements and keyframe requests;
  everything else stays on the WebSocket. Before a session is admitted, what is reachable there is QUIC's handshake,
  TLS 1.3 and HTTP/3 (its setup and requests):
  - An address QUIC has not validated gets a stateless Retry, so a forged source address gets one packet no larger
    than its own and no state; at most 8 attempts are under way at once, quinn holds at most 16 connections not yet
    looked at (buffering 64 KiB for each after its first packet), and at most 64 connections are open (past that, a
    new one is refused); each attempt has 5 s from its validated first packet to its admission, however busy it keeps
    its connection, and the admin closes the connection of an attempt it refuses or that runs out of time; no 0-RTT.
    wtransport's own logs, which trace what a peer sends, are kept to warnings whatever `RUST_LOG` asks.
  - The certificate is self-signed (ECDSA P-256), made in memory at startup, valid for 13 days and an hour and
    replaced every 6 days; the browser accepts only the one whose SHA-256 the viewer's own signed-in WebSocket gave it.
  - The request is admitted only with a ticket of 32 random bytes that the admin gave a viewer over its signed-in
    WebSocket, at most one at a time per viewer: taken once (whatever follows), within 10 s, while that viewer is
    connected and its login live, from the admin's own origin. Anything else gets a 404 and its connection closes.
    The admitted session belongs to that viewer's relay, and ends with it (logout, expiry, the session's end).
  - After admission the client may open one stream, of acknowledgements and keyframe requests only, 64 KiB ahead at
    most (a frame of another message, a second stream, a bidirectional one, or a datagram closes the session).
  A session's browser cannot reach the port: the installer's rule drops whatever a session bridge sends the server's
  own addresses, UDP included.
- One password, hashed with Argon2id, with backoff on failed logins. The person creates it in the admin's first
  screen, from the exact origin, and setting it opens a login. `admin init` sets it instead (for scripts), reading it from stdin in a one-off container with
  no network and no log; neither way replaces a password that is set.
- An optional second step at sign-in (`admin/src/second_step.rs`), offered right after the password is created and
  under Security after login; never in the public demo. With it on, a right password opens no login: it answers with
  a pending step, a random token kept only as its SHA-256, which lasts 2 minutes, opens one login and is dropped after
  five wrong codes. A code answers it:
  - from an authenticator app (TOTP, RFC 6238: HMAC-SHA-1, 6 digits, 30-second steps, one step either side), whose
    160-bit secret is drawn by the admin and does nothing until a code from the app confirms it. A code is taken once:
    the step of the last one taken is written before the login opens, and it and every older one are refused, also
    after a restart.
  - or one of the ten recovery codes, of 80 random bits each, shown once when the second step is turned on (with
    the app or the first passkey); the admin keeps only their SHA-256, and takes each out as it is used.

  Wrong codes count toward the same backoff as wrong passwords, and only an opened login resets it, so knowing the
  password buys no extra guesses at a code. Passkeys (WebAuthn, verified by webauthn-rs) sign in on their own, in place
  of the password and a code: every ceremony requires user verification, the relying party is `ADMIN_ORIGIN` (its host
  is the RP ID, and only that exact origin is accepted; an origin at an IP address offers no passkeys), each challenge
  is kept in memory and used once, a refused sign-in counts toward the backoff, a signature counter that does not move
  past the one stored is refused (for a device that keeps one; one that always reports 0 keeps none), and a passkey
  removed after its ceremony began signs in no more. Turning the app on needs the login; replacing it, adding a passkey,
  removing either and new recovery codes also need the current password, checked as a login checks it and counted toward
  the same backoff. Every change ends every other login, and is refused to a login that ended while it was asked. The
  second step lives in `second-step.json` in the state volume (mode 0600, replaced atomically) and is read again at
  every use; each change, and the reset, holds an exclusive lock beside it; a file that cannot be read stops the start
  and refuses sign-ins rather than turning the second step off, and a code whose use cannot be written opens no login.
  `admin reset-second-step`, run in the admin's container (which takes access to the server), removes that file for
  someone who lost their phone or passkey and their recovery codes: the password alone signs in again, at once. The
  admin logs no secret, code or credential, and webauthn-rs, which traces the credentials it handles, logs warnings at
  most, whatever `RUST_LOG` asks.
- A `__Host-` cookie (`Secure`, `HttpOnly`, `SameSite=Strict`) with a 30-minute idle and a 12-hour absolute expiry.
- An exact `Origin` check plus a CSRF header on every state change a login makes; setting the password and logging in
  (the second step's routes included), which come before there is a login, need the exact `Origin`; a WebSocket
  upgrade needs the exact `Origin` and a live login cookie (browsers send no custom header on an upgrade).
- Logout and expiry close the login's streams and revoke its lease.
- No Docker access.

**Agent tokens.** The administrator may create up to 32 bearer tokens for agents (never in the demo). Each is 32
random bytes; the admin keeps only its SHA-256, in `tokens.json` (mode 0600, replaced atomically), and compares a
presented token with every stored hash in constant time. A token reaches only the agent API (`/api/agent/`) and the MCP
endpoint (`/mcp`), and through them only the sessions it started and those running from a saved session that allows it,
whoever started them: every session records the principal that started it (the administrator, a token, the local MCP
clients, or a demo visitor) and the saved session it runs from, if any (`admin/src/app.rs`); that saved session's
allowed list is read again at every request, and by a call waiting for its turn once it is admitted, so taking a token
or the local clients off it withdraws their reach to the runs someone else started, while the runs they started stay
their own, calls waiting in them included; and only the administrator reaches every one. The
viewer stream, downloads, takeovers, settings, the network simulator and token management stay behind the login. A
bearer request with an `Origin` other than the admin's own is refused. Its tool calls go through the same call service
as the administrator's (`admin/src/service.rs`): one at a time, blocked while a person holds control, and redacted
(field names and driver errors included).
Revoking a token refuses it at once and writes the revocation before its sessions are stopped; should that write fail,
the administrator is told so and it is written again every few seconds, but a restart before it succeeds accepts the
token again. A session still starting for it is stopped instead of listed, and a call waiting for its turn is refused
once admitted. A token carries 256 random bits, so
guessing one is out of reach however fast it is tried; a failed bearer check is answered `401`, and failures never
slow the administrator's login or a valid token.

**Local MCP.** `admin mcp`, run in the admin's container (`docker exec -i`, over `ssh -T` from another computer),
relays an MCP client's stdio to a Unix socket in the admin's state directory: mode 0600 in a 0700 directory, and the
admin answers only a peer with its own uid (`admin/src/local.rs`). Its clients share one principal, `Local`, with
exactly a token's rights: the agent tools, and only the sessions local clients started and those running from a saved
session that allows `local`. It needs no token, port or
certificate, and reaches the admin only for someone who may already run commands in its container. `ADMIN_LOCAL_MCP=0`
turns it off; the demo never serves it.

**Persistent sessions.** A session may be persistent: it starts from a saved session, which is a saved Chrome profile,
and its stop saves that again, so the person's logins on any site, site data, history and preferences carry over
(`admin/src/profiles.rs`, `admin/src/keys.rs`, `browser/launch/src/profile.rs`; never in the demo): what Chrome keeps
when it closes, so not a login a site keeps only until then (a session cookie), as when Chrome closes on a desktop,
and not Chrome's caches. The code and the administrator's API (`/api/profiles`) call a saved session a profile; the
agent API and MCP call it a saved session. What keeps it:

- *The keys.* The key-encryption key (KEK) is Argon2id (the password hash's parameters: 19 MiB, two passes) over the
  administrator's password and a random 16-byte salt of the installation's own (`key-salt`, beside the password hash),
  so it is independent of that hash. Each saved session has a random 256-bit data key, stored only wrapped by the KEK with
  XChaCha20-Poly1305, the profile's id as associated data. Its archive is sealed with its data key by
  XChaCha20-Poly1305 in the STREAM construction (64 KiB chunks, the last flagged, the profile's id as each chunk's
  associated data, a random nonce prefix drawn at every save), so a chunk changed, moved, dropped or added, or an
  archive cut anywhere, does not open (`keys` unit tests).
- *What is on disk*, in the admin's state volume: `key-salt`; `profiles.json` (each saved session's id, name, wrapped key,
  the tokens allowed to use it, and when it was created and last saved, at what size; mode 0600, replaced atomically);
  and `profiles/<id>.bin`, each sealed archive (mode 0600, written beside it and renamed over it once whole). Neither
  the KEK nor a data key is ever written in the clear or logged, nor what a saved session holds: a restore or a save that
  fails logs why, which may name the file it failed on.
- *When the saved sessions are unlocked.* Setting the password, a password login, and with the second step on, a login
  once its code is right (the KEK waits with the pending step until then) derive the KEK; so does unlocking them, below.
  A password confirmed for a security setting (a passkey, the second step) derives none. The admin holds it
  in its memory only (the admin is non-dumpable and runs without swap), from that login until the admin stops; logging out keeps it, so agents
  can resume saved sessions while the person is away. A passkey sign-in checks no password and derives none: after a
  restart, the saved sessions stay locked until the password is entered once to unlock them, checked as a login
  checks it and counted toward the same backoff. While locked, making a persistent session or resuming one is
  refused (`423`); everything else works (`auth` and `api` unit tests). A password set anew, after its file was
  removed, opens none of the saved sessions made under the one before: they say so, and are deleted and made again.
- *Who resumes one.* The administrator, every saved session; an agent token or the local MCP clients only the saved
  sessions that allow them (none at first): `list_saved_sessions` (`GET /api/agent/saved-sessions`) lists only those,
  and any other answers "no such saved session", the same as an id that is no saved session's (`profiles` and
  `agent_api` unit tests, `profiles::a_login_carries_over_through_a_saved_session_and_a_token_resumes_only_one_it_is_allowed`).
  Revoking a token takes it off every saved session. One running session at a time resumes a saved session, so
  Chrome never has two writers; the tokens and local clients that saved session allows reach that run as their own
  while they are allowed, whoever started it (`app` unit tests): they see it in `list_sessions`, drive it, waiting
  while the person holds control, and may stop it, which saves it. A run they started stays theirs either way.
- *Its way to the browser.* At the session's start, before Chrome starts, the admin opens the archive chunk by chunk
  and streams what it opens through the controller's `profile_restore`, which runs the launcher's `profile-restore` in
  the browser container as its own uid (no new mount, privilege or network; the controller's exec guard holds its
  command, user and streams); at the stop, `profile_save` runs `profile-save`, which has the launcher close Chrome with
  `Browser.close` on its private DevTools pipe (as a person quitting it, which writes its cookies and storage out; a
  Chrome without its pipe is sent SIGTERM, which leaves its last cookies unwritten, so that save fails and the profile
  keeps its last save), then streams the profile back, without Chrome's caches (a site's Cache API storage included,
  which its pages must fetch again), crash reports and locks, and the admin seals it as it arrives. In between, the
  plaintext is in the browser's tmpfs, inside its memory limit (no swap, no core dump); while it streams, it passes in
  64 KiB pieces through the memory of the admin and the controller, both non-dumpable and without swap
  (`memswap_limit` in `compose.yaml`), and of Docker's exec stream in the Docker daemon, whose memory the host's swap
  settings govern; it is written to no file.
- *Pages write the profile*, so `profile-restore` takes the archive as untrusted: it makes only directories and regular
  files, each created exclusively and never through a link (it makes none), at relative paths of plain names (no `..`,
  `.`, empty name or leading `/`), each directory before what it holds, at most 100,000 entries, 1,024-byte paths and
  32 names deep, into a directory of its own that it moves into place whole once the archive's end arrived; anything
  else fails the restore and the session (`browser-launch` unit tests on crafted archives).
- *Bounds and failures.* An archive holds at most 512 MiB (Chrome evicts what sites store beyond 256 MiB together; a
  profile that had opened sixteen major sites packed to 26 MiB); a larger save is refused and the saved session lists why. A
  save that fails (Chrome not closed within 20 seconds, a browser that ended, a full disk) keeps the archive from
  before, as does a session that ends without a save: one whose start failed, or that the admin's restart ended.

Chrome on Linux encrypts cookies with a key from the desktop's keyring, which the container has none of; it runs with
`--password-store=basic`, Chrome's fixed key, so a restored profile reads its cookies. Inside a session they are kept
by the session's isolation, as everything Chrome holds is; at rest, by the archive's encryption.

The controller serves only the admin's uid. It refuses any operation, image, mount, namespace, capability, device, or
identity outside its fixed templates, and any session past `CONTROLLER_MAX_SESSIONS` (1 to 16, default 16), so a
faulty or compromised admin runs no more sessions than that cap, each within its template's CPU, memory, and process
limits, and no more than four egress checks beside them, each within the check template's.

**What the agent learns of the person.** After a takeover, text of four characters or more the person typed or pasted
is replaced by `[redacted]` in every later result the agent's caller receives, matched exactly as written: what they
pasted, what they typed as the relay follows it through their edits until they leave the value (Enter, Tab, a click,
Escape or a move), and every four-character piece of it wherever an edit cannot be followed. In the accessibility
tree the driver returns, a password field's value is replaced by `[redacted]`. The set holds at most 256 KiB; once it
is full, every later agent result is replaced by a notice that holds nothing of the page. From the first takeover on
(a viewer that opened the session holding control counts as one: it may type at once),
every image item and every field whose name ends in `_b64`, where the driver returns its screenshots, is removed from
the agent's results. The redaction set lives in `admin/src/redact.rs`: the viewer
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
| 1, 2: no root, capability, device, or writable root; distinct uids; the browser shares only the Wayland directory with the display, read-only | `security::containers_run_unprivileged_with_distinct_identities`; for capabilities, devices and read-only roots, `controller/tests/docker.rs` against the real Docker Engine (`scripts/e2e.sh` runs it whenever the controller can be affected) |
| 3: socket inventories; no UDP in `browser` or `display` | `security::socket_inventories_show_only_the_intended_endpoints` |
| 4: no direct egress from the browser; private and loopback refused through the proxy, also by a rebinding name | `security::the_browser_reaches_nothing_but_the_proxy`; `proxy` unit tests |
| 5: the controller's closed set; no Docker access for the admin | `security::the_controller_refuses_everything_outside_its_closed_set`; `security::containers_run_unprivileged_with_distinct_identities`; `controller` unit tests |
| 6: uid A cannot reach the privileged socket; Chrome's socket has no privileged global | `security::the_privileged_socket_is_out_of_the_browsers_reach`; `hostile::a_connection_from_another_uid_is_refused` |
| 7: every decoded byte comes from compositor-owned pixels or the sound's samples; a flooded sound FIFO costs bounded work | by construction (`display/src/shm.rs`, `display/src/compose.rs`); `hostile::a_pool_truncated_before_the_frame_disconnects`; `audio::a_tone_in_the_page_reaches_the_viewer_as_opus_and_stops_after_it`; `audio::a_writer_flooding_the_fifo_costs_the_display_bounded_work_and_is_not_heard` |
| 8: a modified watcher injects nothing | `control::a_modified_watcher_injects_nothing_and_a_revoked_lease_is_refused` |
| 9: only the holder at the current epoch injects | the same test; `control::the_lease_is_revoked_when_its_holder_disconnects` |
| 10: hostile clients; one hour of fuzzing per parser | `tests/tests/hostile.rs`; the fuzzing run below |
| 11: Chrome's sandbox; internal schemes blocked by policy | `security::chromium_keeps_its_sandbox_and_blocks_internal_schemes` |
| Egresses: the administrator's alone, credentials never answered, saved and used only after a passed check, held by their sessions; the check's template and bound | `admin` unit tests (`egresses`, `api`); `proxy` unit tests (`check`, `upstream`); `controller` unit tests (`ops`, `template`, `request`); `security::an_egress_is_checked_before_use_and_a_session_leaves_only_through_it` |
| 12: setting the first password once, CSRF, expiry, logout closing streams, crash cleanup | `admin` unit tests; `admin/tests/cli.rs`; `security::logout_closes_live_streams`; `security::exclusive::a_restarted_admin_removes_what_the_last_run_left` |
| 13: the UDP port admits only a live ticket of a connected viewer, from the admin's origin; the session's media, and the fallback to the WebSocket | `admin` unit tests (`udp`, `main`); `udp::the_port_admits_only_a_live_ticket_from_the_admins_origin`; `udp::media_take_the_direct_path_and_its_keyframe_requests_are_answered_there`; `udp::a_lost_session_brings_the_media_back_on_the_websocket_in_a_new_epoch`; the admin's one UDP socket in `security::socket_inventories_show_only_the_intended_endpoints` |
| The second step: TOTP vectors, codes used once, recovery codes hashed and used once, the shared backoff, the pending step's expiry, the password for every change but turning the app on, passkeys with user verification at the exact origin, `admin reset-second-step` | `admin` unit tests (`totp`, `second_step`, `auth`, `api`); `admin/tests/cli.rs`; `security::exclusive::the_second_step_signs_in_with_a_code_and_turns_off_with_the_password` |
| Agent tokens: their own sessions and the runs of the saved sessions that allow them only, nothing behind the login, takeover with no image item after it, revocation | `admin` unit tests (`tokens`, `app`, `mcp`); `agents::a_token_reaches_neither_another_s_sessions_nor_anything_behind_the_login`; `agents::a_person_s_takeover_pauses_a_token_s_tools_until_they_give_back`; `agents::exclusive::revoking_a_token_stops_its_sessions_and_one_still_starting` |
| Persistent sessions: the key hierarchy, locked until a password since the start, tokens only on the saved sessions that allow them, one running session per saved one, the archive's bound, safe unpacking of a crafted archive, a login carried over | `admin` unit tests (`keys`, `auth`, `profiles`, `api`, `agent_api`); `browser-launch` unit tests (`profile`); `controller` unit tests (`template`); `profiles::a_login_carries_over_through_a_saved_session_and_a_token_resumes_only_one_it_is_allowed` |
| Local MCP: the admin's uid only, its own sessions and the runs of the saved sessions that allow it only, off in the demo | by construction for the uid (`admin/src/local.rs`); `admin` unit tests (`local`, `app`, and `main`'s `the_local_mcp_socket_is_on_by_default_outside_the_demo_only`); `admin/tests/cli.rs`; `agents::a_local_mcp_client_starts_a_session_and_calls_its_tools_over_admin_mcp` |
| Redaction of what the person typed or pasted | `admin` unit tests (`redact`); `control::takeover_waits_for_the_in_flight_call_and_blocks_the_agent`; `control::the_holders_clipboard_pastes_once_and_the_agent_reads_it_redacted`; `control::give_back_follows_a_64_kib_paste_and_a_full_redaction_set_hides_what_the_agent_reads`; `control::a_fresh_session_opens_holding_control_and_takes_input_at_once` |

### Fuzzing

The targets live in `tests/fuzz` and run with libFuzzer through `cargo fuzz` (`tests/fuzz/run.sh SECONDS`). Every target
ran for 3,600 seconds, in parallel, with no crash, timeout, or out-of-memory: the first four on 2026-10-20 (`message` as
one target for each direction), the recording's two on 2026-10-24:

| Target | Parser | Runs |
|---|---|---|
| `message` | the wire protocol, client to server and server to client (including `config` and its codec) | 1,189,667,781 and 1,126,324,129 |
| `link_frames` | admin-display link frames and the bounded frame reader | 39,764,158 |
| `tile_records` | tiles messages: the records stream inflated under its declared length, then decoded | 71,947,572 |
| `websocket_frames` | WebSocket framing as the admin reads it | 19,847,567 |
| `camera_picture` | a recording's camera pictures, decoded as the display does (zune-jpeg) | 72,327,539 |
| `microphone_packet` | a recording's microphone packets, decoded as the display does (libopus) | 137,739,249 |
