# Design and acceptance criteria

This document states what easi.run must do, how it is built, and how it is judged. The README summarizes it;
[`architecture.md`](architecture.md) walks through the code. Section numbers (§1–§8) and milestone letters (A–E) are
referenced from tests and comments throughout the repository.

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

- **`browser`** (uid A): Chrome as an ordinary maximized window (tab strip, omnibox) with its sandbox (`--no-sandbox` is never used), its D-Bus session bus,
  its accessibility (AT-SPI) bus, and a PulseAudio daemon as uid A, with three modules and no client module loading,
  that writes Chrome's mixed sound to the display's FIFO. Read-only root, no capability, `no-new-privileges`, a seccomp
  profile (Docker's default plus the namespace calls Chrome's sandbox needs, without the 32-bit x86 ABI or
  `socketcall`; [security](security.md#boundaries)), noexec
  tmpfs storage, 2 GiB, 2 CPUs (both configurable; [measured](benchmark.md#browser-limits)). It joins **no** network: its only egress is a loopback forwarder to the proxy's Unix
  socket. (On any Docker network the engine places a DNS resolver socket in the container; with no network there is no
  gateway, no DNS, and no UDP at all.) So that Chrome reads online, as on a desktop, its launcher moves itself and Chrome
  into their own user and network namespace whose only link is a veth pair reaching nothing but itself (not in DevTools
  mode, which keeps the link-less namespace).
- **`proxy`** (uid C): an HTTP proxy on the egress network that accepts only uid A on its Unix socket. It resolves names
  itself, connects to the address it resolved (no second resolution, so no DNS rebinding), refuses loopback, private,
  link-local, CGNAT, multicast, broadcast, and reserved destinations and any port but 80 and 443, and logs each tunnel.
  Production policy has no exception; only the `bench` profile and the public demo's walled garden admit their fixture
  host, by exact name and address.
  When the person picks one of the deployment's egresses for the session, the proxy connects only through that
  upstream (HTTP `CONNECT`, over TLS for `https`, or SOCKS5), resolves names through it by DNS over HTTPS, and vets every
  answer the same way; it receives the upstream once on standard input and the session starts only after it accepted
  it. The browser's time zone and languages are then the egress's.
- **`display`** (uid B): the compositor and encoder (§3.2–§3.3), on no network. It serves one stream endpoint on a Unix
  socket in a volume shared only with the admin application.
- **`agent`** (uid E): the Cua Driver, sharing only the browser's PID namespace and the bus sockets, with no network.
  With DevTools enabled for a session (a per-session choice, off by default; see
  [`agent-compatibility.md`](agent-compatibility.md)), it runs as uid A instead, non-dumpable, in the browser's network
  namespace, because the driver proves ownership of Chrome's DevTools endpoint through `/proc/<chromium>/fd`.
- **`admin`** (uid D): the web application (§6). It publishes only its HTTPS port on loopback, reaches the display only
  through the stream socket, and has no Docker access.
- **`controller`** (uid F): the only holder of the Docker socket, on no network, serving the admin's uid alone on a Unix
  socket. It creates containers only from fixed templates (pinned images, the users, hardening, limits, and networks
  above, volumes it created itself, and, for each display, its own session's subpath of the installation's `streams`
  volume; never a bind mount, host namespace, device, capability, or privileged flag), acts only on resources it
  created, and refuses and logs anything else.

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
  - Bounded per socket: shm pools (count and size), buffers, surfaces, subsurfaces, pixel copies and live protocol
    objects, shared by all of a socket's clients (8 on the browser socket, 8 on the agent's). Bounded per client:
    unused file descriptors, object ids, requests per turn, keyboards created (4), and events it leaves unread
    (512 KiB and 128 descriptors). Bounded per object: commits pending a parent's, damage
    and region rectangles, and offered types. Buffer geometry is at most the output.
  - The pointer reaches a surface only inside its input region.
  - The display sets its own descriptor limit, sized so every client's bound fits.
  - Client memory is never mapped. Damaged regions are read with `pread` from pools on tmpfs or hugetlbfs into compositor-owned
    memory before any use; a pool that is truncated or unreadable disconnects its client, never the server.
  - Unknown, out-of-order, or over-limit requests disconnect the client.
- `unsafe` in the project's own code is allowed only with a stated justification and a test; today it is confined to the
  four functions of the C shim (the AV1 encoder's open, encode and close, and the heap trim) and seven libopus
  functions (the sound encoder, and the decoder of a person's microphone in their recording).

### 3.3 Encoding: hybrid, damage-aware, ordered

- **Static content**: lossless tiles, sent for the damaged cells of a 64x64 grid whose pixels changed since the viewer
  last got them: only the rectangle around the changed pixels, each as its smallest
  lossless record (one color, a palette of up to 256 colors, or RGB rows), the records of a message raw-deflated
  together. Flat motion (a moving interface, a slide transition) stays lossless while its tiles fit the video's byte
  and time budget.
- **Motion** (scrolling, video, animation; detected from the damaged area and rate over a short window) that does not
  fit in tiles: a real-time video stream of the output in **AV1** (SVT-AV1, real-time low-delay mode, screen-content tools). Software only; no
  GPL encoder. An **H.264** Constrained Baseline encoder (OpenH264) is an opt-in build feature, off in the published
  images for patent reasons (§5); in such a build, *Auto* continues a motion episode in H.264 when AV1 encodes cannot
  keep the frame rate on the available CPU or the client presents under 80% of the frames, and the next episode tries
  AV1 again. A client that decodes no available codec gets lossless tiles only.
- **Ordering and recovery**, per viewer:
  - Every media message (`config`, `video`, `tiles`, `cursor`) carries a configuration **epoch** (bumped on
    resolution, codec, or layout change), and `video` and `tiles` a monotonic **revision**. The client discards
    anything of an older epoch and composites strictly by revision, so a late video frame never overwrites newer
    lossless content.
  - Entering video starts with a keyframe; leaving it ends with a lossless refresh at a newer revision.
  - Backpressure acts before encoding: a viewer whose queue exceeds its budget skips frames instead of losing encoded
    ones. A client that loses decodability sends `keyframe-request` and gets a keyframe and a full lossless refresh.
- **Sound**: the browser's PulseAudio writes its mix as raw 48 kHz stereo 16-bit samples, paced by the clock, to a FIFO
  the display owns in the Wayland directory, which the browser mounts read-only. The display parses nothing there (any
  bytes are valid samples), reads at most real time (a writer that runs ahead is discarded, a bounded amount per turn,
  and not heard), sends nothing for silence after a 100 ms tail, and encodes 20 ms Opus packets once for every
  configured viewer. Sound goes out ahead of a viewer's other media; a viewer with more than 16 KiB of sound waiting (about two seconds) gets none until it drains. Nothing flows the other way:
  the page gets no microphone (a person's microphone goes only into their own recording, on the display), and audio
  and screen capture stay off by Chrome's policy.
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
- A small, closed, versioned binary protocol with golden vectors ([`protocol/`](../protocol)): `hello`, `config`,
  `video`, `tiles`, `audio`, `cursor`, `keyframe-request`, statistics and clock messages, `lease`, and, under control only,
  `pointer`, `wheel`, `key` (with composed text), `clipboard` (a paste), `upload-start`, `upload-data`, `file-choose`,
  `file-drop`, `file-drag`, and `file-drag-end`, and, to the holder only, `copied` (what the page copied), `file-offer`, `file-link` (a download's
  ticket), `file-end`, `file-gone`, `file-request`, `file-request-end`, `upload-ack`, and `upload-done`; from and to the
  lease holder, view only included (a recording needs the lease, not input), `record-start`, `record-camera`,
  `record-microphone`, `record-layout`, and `record-stop` from it and `record-state` to it; between the display and the
  admin application only, `file-read` (credit), `file-cancel`, and `file-data`. Unknown or malformed messages close the
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
- **Clipboard, both ways**: a paste is the person's text as a selection the compositor owns, written to the browser in
  full on a bounded writer, then pasted once. What the page copies while a person holds control (only the focused
  client can set the clipboard, and that is the browser) is read through a pipe, at most 1 MiB within 2 s, and
  sent to the holder's connection alone at the epoch of the copy; the display and the admin each check the lease again
  before it leaves, so a lease change drops it, and nothing copied while the agent holds control is collected. The
  client places it on the person's clipboard automatically only if the person turned that on; otherwise it offers a
  button, so a page cannot silently replace what the person copied. The public demo copies automatically, since
  its pages are the operator's own walled garden. The client never reads the person's clipboard: a paste comes from the
  person's own paste event.
- **Downloads**: a file the page downloads reaches the person holding control as soon as it starts, of any size, in
  their own browser's download manager. The browser container's launcher, which binds a files global on the browser's
  Wayland socket before Chrome starts (the display grants it once, and never on the agent's socket), follows its
  private download directory and streams each download while Chrome writes it: in order, through a socket pair the
  display reads without blocking, punching what it handed on out of the tmpfs; it fails the download rather than mix
  two of Chrome's writing sessions (a close-after-write fence, a drain of queued events after every read, and a hash
  of the prefix written before the target was known). Its proxy forwarder holds Chrome's network back while the
  person falls behind, so the tmpfs (256 MiB, the safety bound) holds a few tens of MiB of it. The display
  offers each download to the holder at the current lease epoch (one not started also to the next holder) and sends it
  only as far as its receiver's credit; the admin application gives the holder a single-use ticket and, when the
  person's browser fetches it from the download endpoint (same login, still holding control at that epoch, checked
  before every piece), is that receiver, so at most 4 MiB of it wait there. A response that ends early is an error, so a
  cut download never shows as complete. The display never opens a path the browser can write, and the browser and
  display still share only the Wayland directory. Chrome keeps blocking dangerous file types
  (`DownloadRestrictions`). The size is announced when Chrome knows it (an undecoded body
  with a Content-Length, read from the launcher's private DevTools pipe), so the person's download manager shows
  percentage and time left; the response's last piece waits for the confirmed end.
- **Uploads and drops**: a file chooser the page opens reaches the person as their own file picker, and files they drag
  onto the screen reach the page where they let go. The launcher serves the file chooser portal
  (`org.freedesktop.portal.FileChooser`) on the browser's session bus, owned before Chrome starts and callable only by
  the browser's uid, and admits only callers the bus attests (by process descriptor) to be Chrome's processes, since
  the DevTools agent shares the uid; it opens files, and answers a save (the PDF viewer's download, a page's save
  dialog) with a name it reserves in the download directory, whose file then arrives as any download; choosing several
  saves at once and directories are refused. The display asks the holder (`file-request`); the person's files come over
  the stream in order, at most 256 KiB ahead of the display's acknowledgement, at most 512 MiB each and four at once,
  and go through a pipe to the launcher, which stores each in its private `/uploads` tmpfs (created exclusively, never
  through a link) and answers with the path once exactly its size arrived. The portal then answers Chrome with those
  files; a drop is a compositor-owned drag offering them as `text/uri-list`, which waits for the page to accept before
  it drops. While the person still drags the files over the screen (`file-drag`: the point and the media types, all a
  page sees before a drop), the display drags placeholders of those types over the page, at paths where nothing
  exists, so the page shows its drag-over state; it holds where the person let go while the files upload, input cannot
  release it, and the drop replaces it (`file-drag-end` when the drag leaves or the uploads fail).
  A drag the browser starts itself (an image, a link, or an element dragged within the page) is carried by the
  pointer as on a desktop, the browser's drag icon drawn under it, and drops where the pointer is released. A lease
  change cancels the chooser and abandons every upload.
- **Give back** starts a fresh epoch, and the agent resumes (restarted if it was killed).
- **Redaction**: after a takeover the application keeps what the person typed or pasted, as it stood when they left
  the value (Enter, Tab, a click, Escape or a move), at most 256 KiB, in memory only, and replaces it by `[redacted]`
  in every later result the agent's caller receives; matching is case-sensitive, on four or more characters
  ([security](security.md#boundaries)). From the first takeover on, no screenshot reaches the agent's caller, and
  password field values never do. When the set is full, every later agent result is replaced by a notice that holds
  nothing of the page.

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
10. The hostile-client tests pass, and every parser (protocol, tiles, WebSocket framing, link frames, and a recording's
    camera JPEG and microphone Opus decoders) is fuzzed for at least one hour per target without a crash.
11. Chrome keeps its sandbox, and its policy blocks `chrome://` (but its own new-tab and toolbar pages), `devtools://`,
    `file://`, and `view-source:`; its DevTools frontend stays refused.
12. The admin application resists enrollment theft and CSRF, ends live streams on logout and expiry, and cleans up after
    a crash.

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

- **Enrollment**: until a password is set, every start logs a new one-time enrollment token (single use, 15 minutes;
  a restart makes another), which guards setting the password, hashed with Argon2id.
- **Sessions**: a `__Host-` cookie (`Secure`, `HttpOnly`, `SameSite=Strict`), 30-minute idle and 12-hour absolute
  expiry; logout and expiry close the login's streams and revoke its lease.
- **CSRF**: every state change a login makes needs the exact `Origin` and a CSRF token; enrolling and logging in, which
  come before there is a login, need the exact `Origin`; WebSocket upgrades need the exact `Origin` and a valid session.
- **Functions**: start and stop browser sessions; watch, take control, give back; a live statistics panel; codec,
  bitrate, and frame-rate settings; a network-condition simulator; calls to the agent.
- **Crash cleanup**: session resources carry labels, and on start the application removes orphans and says what it
  removed.

## 7. Milestones

The project was built and proven in this order:

- **A, agent compatibility**: the agent lists the window, binds the tab, navigates, clicks, types, and reads the page.
- **B, capture**: Chrome composited headlessly with correct damage, and the hostile-client tests passing.
- **C, stream**: the ordered hybrid stream to the WebCodecs client through the authenticated path, with per-viewer
  backpressure and recovery tests.
- **D, control**: the lease, quiescence, epochs, and redaction, with a scripted agent.
- **E, proof**: the §4 tests, fuzzing, SBOMs, and the §8 benchmark.

## 8. Acceptance: the benchmark against KasmVNC

**Fairness**: one host, the same Google Chrome build and flags, 1280x800, and the same total CPU for the screen path:
KasmVNC gets one container with 2 CPUs (Chrome and the X server); this project gets `browser` 1 CPU (the bench profile
lowers it from the default 2) and `display` 1 CPU (in a bench lane; a session outside one gives it 2).

**Fixtures**, served over HTTPS by a host that exists only in the `bench` profile: a static article; smooth scroll
through a long page; a full-motion composited animation; a 1080p video; a page of text and figures scrolling, with
exact reference frames for quality; text typed in a form under control. Two stress fixtures are reported too but carry
no gate: a full-screen animation that Chromium must repaint every frame, which Chromium itself cannot render faster than
about 30 fps on one CPU, and flat colored blocks that all change every frame, where chroma subsampling alone caps any
4:2:0 codec below SSIM 0.97 even when lossless. Each fixture runs 60 seconds, 5 headed and 5 headless repetitions.

**Instrumentation**: presented frames counted from what the client actually drew (distinct time stamps read back from
the canvas each animation frame); capture-to-draw from a time stamp the page draws, with the client and server clocks
compared by a round-trip exchange; input-to-draw from a square the page flips on each key press; quality as SSIM of drawn
frames against the fixture's exact reference frames.

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
