# Design and acceptance criteria

This document states what easi.run must do, how it is built, and how it is judged. The README summarizes it;
[`architecture.md`](architecture.md) walks through the code. Its section numbers are referenced from tests and
comments throughout the repository.

## 1. Context

An agent drives a disposable Chrome inside a hardened container: non-root, read-only root, every capability
dropped, a strict seccomp profile, `no-new-privileges`, noexec tmpfs storage, 2 GiB of memory, 2 CPUs, and no direct
network (every request leaves through a proxy that refuses private destinations). The agent works through the
[Cua Driver](https://github.com/trycua/cua) (`cua-driver mcp`) and the page's accessibility tree. A person must be able
to watch the browser live and take control of it (to sign in, pass a 2FA prompt or a CAPTCHA) and hand it back, from a
web application over HTTPS, possibly from outside the local network.

The usual way to show such a browser is a VNC server. KasmVNC, a capable one, carries costs this project removes: it
is an X server built on Xorg (a large C code base); it opens a UDP listener and serves an HTTP API on its WebSocket port;
it is GPL-2.0; its pointer and clipboard messages deviate from standard RFB; and it encodes JPEG and WebP inside the
same container as the untrusted pages, so the viewer's browser decodes image bytes a compromised Chrome could
influence.

## 2. Goals

A remote browser screen that is faster, lighter on the network, and safer than KasmVNC at the same total CPU, and ready
for a person's input:

- Up to 60 fps at 1280x800 for motion, lossless text for static content, the page's sound, and no media traffic for a
  static, silent page.
- Motion bandwidth of a few Mbit/s instead of tens.
- Latency low enough that typing and scrolling under control feel local on a LAN.
- No hidden listener or API, no UDP, and no image, video, or sound bytes produced inside the browser's sandbox ever
  reaching the viewer's decoder.
- A clean screen: the page and nothing else, no remote-desktop chrome.

## 3. Architecture

### 3.1 Containers, networks, and identities

Six roles, by default each with its own uid ([`topology.md`](topology.md) has the exact uids, groups, mounts, and limits):

- **`browser`** (uid A): Chrome as an ordinary maximized window (tab strip, omnibox) with its sandbox (`--no-sandbox`
  is never used), its D-Bus session and accessibility (AT-SPI) buses, and a PulseAudio daemon as uid A that writes
  Chrome's mixed sound to the display's FIFO. Read-only root, no capability, `no-new-privileges`, the browser seccomp
  profile, noexec tmpfs storage, 2 GiB, 2 CPUs (both configurable; [measured](benchmark.md#browser-limits)). It joins
  **no** network: its only egress is a loopback forwarder to the proxy's Unix socket, and outside DevTools mode Chrome
  reads online in a namespace of its launcher's that reaches nothing else ([security](security.md#boundaries)).
- **`proxy`** (uid C): an HTTP proxy on the egress network that accepts only uid A on its Unix socket, refuses every
  non-public destination (a rebinding name's included) and any port but 80 and 443, and logs each tunnel. Production
  policy has no exception; only the `bench` profile admits its fixture host, by exact name and address, and the public
  demo's walled garden sends every destination to its one pages server. When the person picks one of the
  egresses the administrator added in the dashboard (HTTP `CONNECT`, over TLS for `https`, or SOCKS5; each checked
  before a session starts through it) for the session, the proxy connects only
  through it, and the browser's time zone and languages are the egress's ([security](security.md#boundaries), Egress
  through an upstream).
- **`display`** (uid B): the compositor and encoder (§3.2–§3.3), on no network. It serves one stream endpoint on a Unix
  socket in a volume shared only with the admin application.
- **`agent`** (uid E): the Cua Driver, sharing only the browser's PID namespace and the bus sockets, with no network.
  With DevTools enabled for a session (a per-session choice, off by default; see
  [`agent-compatibility.md`](agent-compatibility.md)), it runs as uid A instead, non-dumpable, in the browser's network
  namespace, because the driver proves ownership of Chrome's DevTools endpoint through `/proc/<chromium>/fd`.
- **`admin`** (uid D): the web application (§6). It publishes only its HTTPS port on loopback, reaches the display only
  through the stream socket, and has no Docker access.
- **`controller`** (uid F): the only holder of the Docker socket, on no network, serving the admin's uid alone on a Unix
  socket. It creates containers only from fixed templates ([topology](topology.md) lists them), acts only on resources
  it created, and refuses and logs anything else.

The browser and the display share only a tmpfs directory holding the browser's Wayland socket, owned by uid B with mode
0750, so the browser can connect but never create, replace, or remove a socket.

### 3.2 The display server: a headless Wayland compositor

- Built on [Smithay](https://github.com/Smithay/smithay), headless: no DRM/KMS, no GPU, no device, no capability.
  Software composition of one 1280x800 output (configurable). Chrome's windows are maximized and stacked, the newest
  on top, as on a desktop where every window is maximized; a page may go fullscreen. Popups, menus, and native dialogs
  are composited too.
- Damage-driven: a static page produces no frame. Each commit is composed as soon as it arrives, and the output reports
  presentation times, so Chrome paces itself by the refresh without waiting for a compositor tick.
- **Hostile clients** (Chrome may be compromised; `wl_shm` hands the compositor client-controlled file descriptors
  and shared memory):
  - Everything a client can make the compositor hold is bounded: per socket, shared by all of its clients (8 on the
    browser socket, 8 on the agent's), per client, and per object; buffer geometry is at most the output.
    [Security](security.md#boundaries) (Browser to compositor) lists the bounds.
  - The pointer reaches a surface only inside its input region.
  - Client memory is never mapped: damaged regions are read into compositor-owned memory before any use, and a pool
    that is truncated or unreadable disconnects its client, never the server.
  - Unknown, out-of-order, or over-limit requests disconnect the client.

### 3.3 Encoding: hybrid, damage-aware, ordered

- **Static content**: lossless tiles, sent for the damaged cells of a 64x64 grid whose pixels changed since the viewer
  last got them: only the rectangle around the changed pixels, each as its smallest
  lossless record (one color, a palette of up to 256 colors, or RGB rows), the records of a message raw-deflated
  together. Flat motion (a moving interface, a slide transition) stays lossless while its tiles fit the video's byte
  and time budget.
- **Motion** (scrolling, video, animation; detected from the damaged area and rate over a short window) that does not
  fit in tiles: a real-time video stream of the moving region in **AV1** (SVT-AV1, real-time low-delay mode, screen-content tools). Software only; no
  GPL encoder. An **H.264** Constrained Baseline encoder (OpenH264) is an opt-in build feature, off in the published
  images for patent reasons (§5); in such a build, *Auto* continues a motion episode in H.264 when AV1 encodes cannot
  keep the frame rate on the available CPU or the client presents under 80% of the frames, and the next episode tries
  AV1 again. A client that decodes no available codec gets lossless tiles only.
- **Ordering and recovery**, per viewer:
  - Every media message (`config`, `video`, `tiles`, `cursor`) carries a configuration **epoch** (bumped on
    resolution, codec, or layout change), and `video` and `tiles` a monotonic **revision**. The client discards
    anything of an older epoch and composites strictly by revision, so a late video frame never overwrites newer
    lossless content.
  - Every epoch opens with a full refresh, unasked (the client clears its picture on each configuration): lossless
    tiles, or, while video goes on, a keyframe and the tiles around it. Entering video starts with a keyframe;
    leaving it ends with a lossless refresh at a newer revision.
  - Backpressure acts before encoding: a viewer whose queue exceeds its budget skips frames instead of losing encoded
    ones. A client that loses decodability sends `keyframe-request` and gets a keyframe and a full lossless refresh.
- **Sound**: the browser's PulseAudio writes its mix as raw 48 kHz stereo 16-bit samples, paced by the clock, to a FIFO
  the display owns, which costs the display bounded work whatever is written ([security](security.md#boundaries),
  Browser's sound). The display sends nothing for silence after a 100 ms tail, and encodes 20 ms Opus packets once
  for every configured viewer. Sound goes out ahead of a viewer's other media; a viewer with more than 16 KiB of sound waiting (about two
  seconds) gets none until it drains. Nothing flows the other way: the page gets no microphone, and audio and screen
  capture stay off by Chrome's policy.
- Every byte the viewer decodes is produced by `display`, from compositor-owned pixels or from the sound's samples,
  never by Chrome.

### 3.4 The web client

- TypeScript, one canvas: WebCodecs `VideoDecoder` for video, a bounded inflater for tiles, drawn with WebGL2, or
  with Canvas 2D where WebGL2 would run on a software rasterizer. Sound: WebCodecs `AudioDecoder` (Opus), scheduled on an
  `AudioContext` 60 ms ahead and never more than 300 ms; stale packets are dropped. It plays by default once the
  browser's autoplay rule allows (any click or key on the page), and can be muted, locally only.
- At `hello` the client reports which of AV1 and H.264 it decodes; the server picks the codec. A client with neither
  gets lossless tiles only, at a lower frame rate.
- The administrator's controls live outside the screen: Watch and Close on the sessions list, and Take control, Give
  back, sound, Record, and Close session above the screen. The public demo instead floats one bar over the screen,
  which hides itself after a few seconds and comes back from a handle at the top, as remote-desktop bars do.

### 3.5 Transport

- One WebSocket per viewer between the browser and the admin application, and one framed stream between the admin
  application and the display over the Unix socket. No UDP, no outbound connection from `display`, no other listener.
- A small, closed, versioned binary protocol with golden vectors ([`protocol/`](../protocol), whose crate lists every
  message): media and sound go to every viewer; input, clipboard and file messages only under control, and the page's
  copies and files only to the holder; a recording needs the lease, not input. Unknown or malformed messages close the
  stream.
- Bounds at every layer: message sizes, queues, buffers, and deadlines. "No traffic for a static, silent page" refers to
  media and sound; statistics and keepalives are counted separately.

### 3.6 Control and the agent

- The agent's privileged Wayland protocols (window listing, screen capture, virtual pointer and keyboard) and its input
  socket are offered only on a **second** socket, in a directory only the display and the agent can enter. Chrome's
  socket offers none of them.
- **Take control** is a lease. Before granting it, the server blocks every new agent command and waits, within a bound,
  for the in-flight one to end; if it does not, the agent's container is killed and proven stopped, and if even that
  fails the session stops. While a person holds the lease, only that connection's input is injected, fenced by an epoch
  that every grant and revocation advances; watchers never inject; a disconnect, ping timeout, logout, or expiry revokes
  the lease at once and clears the clipboard.
- **Clipboard, both ways**: a paste is the person's text as a selection the compositor owns, pasted once. What the page
  copies while a person holds control (at most 1 MiB within 2 s) reaches the holder's connection alone, at the epoch of
  the copy, and the person's clipboard automatically only if they turned that on; the client never reads the person's
  clipboard. [Security](security.md#boundaries) (Clipboard) gives the checks, and
  [architecture](architecture.md#the-input-path) the code path.
- **Downloads**: a file the page downloads reaches the person holding control as soon as it starts, of any size, in
  their own browser's download manager, with its size when Chrome knows it; a cut download never shows as complete.
  While the person falls behind, the browser's tmpfs (256 MiB, the safety bound) holds a few tens of MiB of it, and at
  most 4 MiB of it wait in the admin application. [Security](security.md#boundaries) (Downloads) gives the checks, and
  [architecture](architecture.md#the-download-path) the code path.
- **Uploads and drops**: a file chooser the page opens reaches the person as their own file picker, and files they drag
  onto the screen reach the page where they let go (a compositor-owned drag offering them as `text/uri-list`), the
  page showing its drag-over state while they drag; a drag the browser starts itself (an image, a link, an element) is
  carried by the pointer and drops where it is released. The person's files come at most 256 KiB ahead of the
  display's acknowledgement, at most 512 MiB each and four at once, and only a file that arrived whole is given to the
  page; a lease change cancels the chooser and abandons every upload. [Security](security.md#boundaries) (Uploads and
  the file chooser) gives the checks, and [architecture](architecture.md#the-upload-path) the code path.
- **Give back** starts a fresh epoch, and the agent resumes (restarted if it was killed).
- **Redaction**: after a takeover, text of four characters or more the person types or pastes reaches the agent's
  caller as `[redacted]`; [security](security.md#boundaries) (What the agent learns of the person) states what is kept
  and removed.

## 4. Security requirements

Each is proven by a test ([`security.md`](security.md) maps them to tests):

1. No process runs as root; no container has a capability, a device, or a writable root.
2. Chrome (A), the display (B), the proxy (C), the admin application (D), the agent (E, by default), and the controller
   (F) run as distinct uids; `browser` and `display` share only the Wayland socket directory, which `browser` cannot
   modify.
3. Socket inventories of every container show only the intended endpoints; no UDP socket exists in `browser` or
   `display`.
4. From `browser`, direct connections to the internet, the host, private ranges, and any UDP destination fail; through
   the proxy, private and loopback destinations are refused, including after DNS rebinding.
5. The controller refuses every operation outside its closed set; the admin application has no Docker access.
6. Uid A cannot connect to the privileged Wayland socket, and Chrome's socket offers no privileged protocol.
7. Every image, video, or sound byte the client decodes is produced by `display`, from compositor-owned pixels or the
   sound's samples; a writer flooding the sound FIFO costs the display bounded work and is not heard.
8. Watch mode: the server injects nothing, even from a modified client.
9. Control mode: only the lease holder's connection injects; a stale epoch, a second viewer, and a revoked lease are
   refused.
10. The hostile-client tests pass, and the parsers listed here (protocol, tiles, WebSocket framing, link frames, and a
    recording's camera JPEG and microphone Opus decoders) are each fuzzed for at least one hour without a crash.
11. Chrome keeps its sandbox, and its policy blocks `chrome://` (but its own new-tab and toolbar pages), `devtools://`,
    `file://`, and `view-source:`; its DevTools frontend stays refused.
12. The admin application sets its first password once and only from its exact origin, resists CSRF, ends live streams
    on logout and expiry, and cleans up after a crash.

## 5. Licenses and patents

- **Runtime images**: easi.run's own code and its client are AGPL-3.0-only. Its encoders, the agent's driver, and the
  crates and packages it links are permissively licensed; the base system and Chrome include identified LGPL
  libraries, dynamically linked as the distribution ships them; no third-party GPL program runs in them.
  [`licenses.md`](licenses.md) is generated from the images' SBOMs.
- **Benchmark baseline**: KasmVNC (GPL-2.0) exists only in the `bench` profile, never in a runtime image.
- **Codec patents**: copyright licenses do not settle patent rights. AV1 is covered by AOMedia's royalty-free patent
  license, and the published images encode only AV1. H.264 is patent-encumbered, and Cisco's patent coverage applies
  only to its prebuilt OpenH264 binary, not to this project's build from source, so H.264 is an opt-in build feature
  whose distribution needs its own patent position.

## 6. The admin web application

- **Enrollment, logins and CSRF**: while no password is set, the first screen creates it, and the person lands
  logged in; a login is a `__Host-` cookie with an idle and an absolute expiry, and
  logout and expiry close its streams and revoke its lease; every state change needs the exact `Origin`.
  [Security](security.md#boundaries) (Admin application) gives the rules.
- **Functions**: start and stop browser sessions; watch, take control, give back; a live statistics panel; codec,
  bitrate, and frame-rate settings; a network-condition simulator; calls to the agent.
- **Crash cleanup**: session resources carry labels, and on start the application removes orphans and says what it
  removed.

## 8. Acceptance: the benchmark against KasmVNC

One host, the same Google Chrome and the same total CPU for the screen path; each fixture runs 60 seconds, 5 headed
and 5 headless repetitions. The fairness rules, the fixtures (two of them stress fixtures with no target) and how each
metric is measured are in [the benchmark's method](benchmark.md#method).

| Metric (median of repetitions) | Target |
|---|---|
| Full-motion animation, presented fps | ≥ 55 and ≥ KasmVNC |
| Smooth scroll, presented fps | ≥ 55 |
| Motion bitrate | ≤ 1/3 of KasmVNC's at equal or better SSIM |
| Static page media traffic | 0 bytes/s after the last damage |
| Static text | bit-exact lossless tiles |
| Capture-to-draw latency (LAN), watch | p50 ≤ 50 ms, p95 ≤ 100 ms |
| Input-to-draw latency (LAN), control | p50 ≤ 60 ms, p95 ≤ 120 ms |
| CPU at the shared budget | reported, with presented fps per CPU |
| Memory, `display` | ≤ 256 MiB |
| Image size, `display` | reported |

## Alternatives considered

- **WebRTC (Selkies-GStreamer, Neko)**: fast, but it adds a UDP media path, ICE and TURN machinery, and hundreds of MiB
  of GStreamer or a second product with its own users and lifecycle.
- **Chrome's DevTools screencast**: simplest, but page-only (no native dialogs), a second DevTools client beside the
  automation driver, and slow without a GPU.
- **xpra, Apache Guacamole**: heavier or GPL, with large surfaces and no clear gain over KasmVNC.
- **Kernel capture (DRM/KMS, dma-buf, vkms)**: needs devices and capabilities the containers must not have; the
  compositor already knows the damaged pixels at the source.
- **Compositors with RDP or VNC backends (Weston, GNOME, wayvnc)**: a remote-desktop protocol instead of a purpose-built
  damage and video stream, and less suited to a browser client.
