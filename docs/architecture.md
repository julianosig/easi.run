# Architecture

A walk through the code, following a frame from Chrome to the viewer's canvas and a key press back. The
[design](design.md) states the requirements; [`topology.md`](topology.md) lists the containers, their uids, mounts,
networks, and CPU, memory and process limits; [`security.md`](security.md) explains why the boundaries sit where they do.

## Repository layout

| Path | Language | What it is |
|---|---|---|
| [`display/`](../display) | Rust, a small C shim | The headless Wayland compositor and the per-viewer encoder |
| [`protocol/`](../protocol) | Rust | Wire protocol between the viewer and the display, the admin-display link framing, golden vectors |
| [`client/`](../client) | TypeScript | The viewer (WebCodecs, WebGL2 or Canvas 2D), the admin page, the benchmark page |
| [`admin/`](../admin) | Rust | The HTTPS application: enrollment, login, sessions, the viewer relay, the lease, agent calls, redaction, agent tokens, the agent API and the MCP endpoint |
| [`controller/`](../controller) | Rust | The only Docker-socket holder: a closed set of operations on fixed templates |
| [`proxy/`](../proxy) | Rust | The browser's egress proxy, which refuses private and special destinations |
| [`egress/`](../egress) | Rust | An upstream proxy a session may leave through, as admin, controller, and proxy all check it, and the destination rules the controller and the proxy share |
| [`locale/`](../locale) | Rust | A session's time zone and languages, as admin, controller, and launcher all check them |
| [`browser/`](../browser) | Rust, config | The Chrome image: its launcher, managed policy, session-bus policy, seccomp profile |
| [`agent/`](../agent) | Rust | The Cua Driver image and its launcher |
| [`affinity/`](../affinity) | Rust | Keeps a process on as many neighboring physical cores as its CPU quota, rounded up (used by `display` and the browser launcher) |
| [`bench/`](../bench) | Rust, HTML, Python, JS | The fixture host, the fixtures, the KasmVNC baseline, the measuring runner, and results |
| [`tests/`](../tests) | Rust | End-to-end tests against the running stack, hostile Wayland clients, probes, and fuzz targets |
| [`scripts/`](../scripts) | Python, shell | The checks and the development image, the encoders' builds (SVT-AV1, libopus), the license report and SBOMs, the end-to-end runner, the pinned tool snapshot, and the demo's host lists |
| [`site/`](../site) | Svelte, TypeScript | The easi.run landing page and docs.easi.run, with their nginx configuration and release script |
| [`demo/`](../demo) | Compose, HTML | The public demo at easi.run/demo/: its own installation and the walled garden of demo pages |
| [`third_party/`](../third_party) | Rust | A patched crate (`wayland-backend`), used through `[patch.crates-io]` |

## The picture path

```text
Chrome ──wl_shm──▶ display ──link frames──▶ admin ──WebSocket──▶ viewer
 (uid A)     Unix     (uid B)    Unix socket   (uid D)    HTTPS      (client/)
```

### 1. Wayland in: `display/src/relay.rs`, `shm.rs`, `limits.rs`

Each Wayland client is served through a socket pair. In each event-loop turn, the relay forwards at most a bounded
number of request bytes and descriptors to Smithay. A fast client therefore cannot starve the others, and a client
that hoards descriptors is disconnected. `shm.rs` implements `wl_shm` without mapping client memory. It accepts only
pools on tmpfs or hugetlbfs and reads damaged rows with `pread` into compositor-owned memory. `limits.rs` holds every
per-socket and per-client bound and the display's descriptor limit.

### 2. Composition: `frame.rs`, `compose.rs`, `shell.rs`

A commit is composed as soon as it arrives, at most once per `DISPLAY_COMPOSE_GAP_MS`. After each composition, the
compositor sends frame callbacks and `wp_presentation` feedback, so Chrome paces itself by the advertised refresh.
`compose.rs` paints the damaged region of the top toplevel and its popups into an `Arc<Vec<u32>>` framebuffer.
Encoders share that framebuffer rather than copy it; when one still holds the last frame, the next frame is written
into a spare buffer. `shell.rs` keeps every browser toplevel maximized at the origin and stacked, the newest (or the one
raised) on top and focused; a page may go fullscreen (a video, F11) and back.

### 3. Per-viewer encoding: `display/src/stream/`

Every viewer connection on the stream socket gets three threads (`viewer.rs`): a reader, an encoder, and a writer.
The encoder (`encoder.rs`) receives `Snapshot`s, each a composed frame plus its damage. For each snapshot it chooses:

- **Tiles** (`tiles.rs`) for ordinary damage. The output is cut into 64x64 cells. Of a damaged cell whose pixels
  differ from what this viewer last received, only the rectangle around the changed pixels is sent. Each rectangle
  becomes its smallest lossless record: one color, a palette of up to 256 colors with
  an index per pixel, or RGB rows. The records of one message are raw-deflated together.
- **Video** while `motion.rs` sees motion, meaning many frames changing a large share of the output within a short
  window, and the motion does not fit in tiles: each motion frame first goes as tiles, and the episode turns to video
  at the first frame whose tiles (headers included) take more than 1.5 times the video's per-frame bytes or hold more
  than 256 KiB of RGB records (content too rich for a palette, given up before deflating it), or when the median
  encode of a group of five lossless frames takes more than 60% of the frame interval. The codec is AV1 (`av1.rs`
  and `av1.c`, SVT-AV1's real-time preset, statically linked), from the BT.709 I420 picture of `yuv.rs`. A build with the `h264` feature adds OpenH264 (`h264.rs`); under *Auto*, an episode then
  continues in H.264 when AV1 encodes exceed 60% of the frame interval (the median of 20 encodes) or, two seconds in a
  row, the client draws under 80% of the video frames sent while at least 20 a second are. The video encoder starts with the first frame that goes as video and is released
  after 5 s without one.

  AV1 covers only where the picture moves for good (`region.rs`): the bounding box, in whole cells, of the cells in
  sustained motion, changing (compared with their last hash; damage is only a hint) with no pause over 250 ms for at
  least 500 ms and lately, at least 64 pixels on each side; an episode's first region, before anything has moved that
  long, is the box of whatever changed in the last 500 ms. Around it, changes go as tiles, so a hover, a ticking
  counter, or a new page beside a playing video arrives exact and never moves the region, and what a lossless trial
  left unsent outside it is sent as tiles when video starts. The region grows as soon as sustained motion appears
  outside it, and shrinks once that has fitted in a smaller one for 10 s (what moves in a video or a game, or on a
  scrolling page, shifts from place to place), or for 1 s when what it would leave has not changed at all meanwhile
  (a page that changed around a video goes back to exact tiles soon); it moves at most once a second, since each move restarts the encoder
  with a keyframe at the new place, after lossless tiles over whatever the old region leaves (a revision below the
  keyframe's and above every old frame's). A part of the region that changed as one (a tenth of its tiles, at least
  four, within 250 ms, as when a page or a panel between two animations changes) and then stayed still for 500 ms
  gets a keyframe, since inter frames may leave traces of what it showed before there; one that stopped after
  sustained motion (a long fade) only after 3 s. The few tiles a figure crossing a video leaves behind count only once
  still for 10 s, and no repainting keyframe comes within 2 s of the last keyframe, which under constant bitrate would
  get few bits. Video ends once no pixel
  changed for 500 ms, whatever the damage says.

  The encoder's constant bitrate is spent over the frame rate it was started for, whatever the frames' timing: 30
  when the compositor gives the region at most 36 frames a second (a video at 24, 25, or 30) over a whole second and
  the video keeps up with them, else the frame-rate cap; one started for 30 goes back to the cap only from 45 frames a
  second, so a source near the threshold does not flap. A region whose rate changes restarts the encoder at the other
  rate after 2 s, at most once per 5 s. Only these two rates, so a compositor slowed by the display's own load
  moves the encoder one step at most, and a video that falls behind stays at the cap: fewer frames given more bits each
  would fall further behind. H.264 covers the whole output (browsers would read a small crop that
  states no colors as BT.601).
- **Cursor** (`cursor.rs`): a named shape, or a custom image read from the client's cursor surface.

Backpressure acts before encoding. A viewer whose queue exceeds its budget skips frames rather than dropping encoded
ones. Leaving video ends with a lossless refresh.

### 4. The link: `protocol/src/frame.rs`

The admin application and the display exchange length-prefixed frames over the Unix socket. Each frame carries a
wire protocol message (`protocol/src/lib.rs`). The protocol is closed: an unknown message type, a malformed field, or a
value outside its range closes the stream. `protocol/vectors/` holds the golden encodings that both the Rust code and
the TypeScript mirror (`client/src/protocol.ts`) are tested against.

### 5. Admin relay: `admin/src/relay.rs`

The admin application opens its own display connection for each viewer WebSocket. Display-to-viewer messages pass
through unchanged, or through the network simulator when one is set. Viewer-to-display messages are decoded and
checked first, and input is forwarded only from the lease holder at the current epoch.

### 6. The viewer: `client/src/`

- `viewer.ts` owns the WebSocket and the canvas.
- `order.ts` applies the ordering rules: it discards older epochs and composites by revision, so a late video frame
  never covers newer lossless tiles.
- `decoder.ts` wraps WebCodecs, `tile.ts` inflates tiles under a hard bound, and `qoi.ts` decodes custom cursor
  images.
- `renderer.ts` draws with WebGL2, or with Canvas 2D where WebGL2 would run on a software rasterizer. It presents at
  most once per animation frame. WebGL2 draws each video frame into the screen as it is decoded; Canvas 2D, whose
  conversion is asynchronous, queues the converted frames and drains a backlog after two.
- `audio.ts` decodes the sound with WebCodecs and schedules it on an `AudioContext`, 60 ms ahead and never more than
  300 ms; a gap in the timestamps plays on after what is scheduled, with the same decoder (the display's encoder
  goes on across it too).

## The sound path

```text
Chrome ──PulseAudio──▶ pulseaudio ──FIFO, raw samples──▶ display ──Opus, audio messages──▶ admin ──▶ viewer
 (uid A)   private socket  (uid A)      Wayland volume      (uid B)
```

- `browser/launch/src/main.rs` starts PulseAudio before Chrome, as uid A, with three modules: the native protocol on a
  socket in `/home/browser/pulse` (private to the container, outside the bus volume the agent shares), a pipe sink that
  writes the mix to `/run/browser/wayland/audio` paced by the clock, and suspend on idle, which stops the samples one
  second after the last stream.
- `display/src/stream/audio.rs` creates that FIFO (`0660`, the Wayland group) before the browser's socket and opens it
  read-write, so it never sees end of file and its thread sleeps in `poll` while nothing plays. Each turn (at most
  every 10 ms) it takes at most real time's worth of whole stereo samples; a backlog over 100 ms is discarded, 64 KiB
  at most per turn, and restarts the stream. Silence sends nothing after a five-packet tail.
- `display/src/stream/opus.rs` encodes 20 ms packets (libopus, 64 kbit/s, complexity 5) once, and `audio.rs` sends
  each to every viewer that has its configuration and less than 16 KiB of sound queued. Nothing is encoded while no viewer is
  connected.

## The input path

`client/src/input.ts` and `keys.ts` map DOM events to protocol `pointer`, `wheel`, `key`, and `clipboard` messages,
and only while this connection holds the lease. The admin relay checks the lease and records typed text for
redaction (`admin/src/redact.rs`). The display checks the lease epoch again and injects the input into the focused
surface (`display/src/input.rs`).

Text the page copies goes the other way: `display/src/copy.rs` reads the browser's new clipboard selection once the
loop is idle (Smithay records it after its callback) and sends it to the holder alone, at the copy's lease epoch;
`admin/src/relay.rs` checks the lease again as it leaves (`holder_only`), and `client/src/viewer.ts` once more before it
offers it or, if the person turned that on, writes it to their clipboard.

## The download path

```text
Chrome ─▶ /downloads ─▶ browser-launch ──socket pair, easi_files_v1──▶ display ──file-offer──▶ admin relay ──file-link──▶ viewer
 (uid A)    (private)      (uid A)  ▲        browser's Wayland socket      (uid B) ◀──file-read──    (credit)        │
                                    │                                       ──file-data, file-end──▶ │              │ GET ticket
                       proxy forwarder waits at the gate                                       download endpoint ◀─┘ (native download)
```

- `browser/launch/src/files.rs` binds `easi_files_v1` (`protocol/wayland/easi-files-v1.xml`) before Chrome
  starts, and `browser/launch/src/downloads.rs` follows every file created in `/downloads` by inode (inotify: create,
  both halves of a move, open, close-after-write). From a download's first move to `<name>.crdownload` it streams it:
  reads it in order up to its size, writes it to its end of a Unix stream socket pair, and punches the pages handed on
  out of the tmpfs. The fence (a close-after-write freezes the stream; only the move to its plain name completes it),
  the drain of queued events after every read, and the hash check of the bytes written before the first move make the
  person get exactly Chrome's final file or a failed download. Anything else is delivered only complete.
- `browser/launch/src/gate.rs` holds the proxy forwarder (`forward.rs`) back while downloads being received fall
  behind (their backlog past 48 MiB lets only drained bytes through, past 96 MiB nothing) or a file not yet followed
  grows past 4 MiB, so a download of any size goes at the person's pace inside a few tens of MiB of the tmpfs.
- `display/src/files.rs` grants the global once, to a browser-socket client, accepts only a Unix stream socket (read
  only with `MSG_DONTWAIT`), offers each download to the lease holder (one nobody started receiving also to the next
  holder), and sends it as `file-data` pieces of exactly 64 KiB (the last shorter) as far as the receiver's credit
  (`file-read`, 4 MiB ahead) allows, through the viewer's writer as bulk; `file-end` follows the last piece in the same
  queue.
- `admin/src/relay.rs` gives the holder a single-use ticket (`file-link`) per offer, and receives the bytes itself once
  `admin/src/api.rs`'s download endpoint is fetched with it (same login, the viewer still holding control at that
  epoch, checked again before every piece); the response ends cleanly only after `file-end` with the exact size.
- `browser/launch/src/cdp.rs` reads each download's size from Chrome over the launcher's private DevTools pipe
  (descriptors 3 and 4, set up only for Chrome's spawn): it attaches to every page and frame, enables only their
  `Page` domain, and matches `Page.downloadProgress` totals to streams by name, giving none when ambiguous. The
  admin sends it as `Content-Length` and holds the last piece until `file-end`.
- `client/src/files.ts` hands each ticket to the person's browser as a native download, and shows its state.

## The upload path

```text
viewer ──upload-start, upload-data──▶ display ──pipe, easi_files_v1──▶ browser-launch ─▶ /uploads (private)
          ◀──upload-ack, upload-done──       ◀──stored(path)──             │
Chrome ──portal OpenFile──▶ browser-launch ──open_chooser──▶ display ──file-request──▶ viewer
         ◀──Response(uris)──                ◀──chosen, chooser_done──   ◀──file-choose──
```

- `browser/launch/src/portal.rs` owns `org.freedesktop.portal.Desktop` before Chrome starts (no replacement; the bus
  policy in `browser/session.conf` lets only the browser's uid own or call it), checks each caller's bus-attested process
  descriptor descends from the Chrome it started, answers on the caller's own request object, and lets only that
  caller close it.
- `display/src/uploads.rs` receives the holder's uploads in order within the window, writes them into one pipe per
  upload without blocking (a loop source waits for room), acknowledges every 64 KiB, closes the pipe at exactly the
  size, and answers the chooser with the stored paths once each is stored; `display/src/drop.rs` drags placeholders of
  the person's dragged files over the page, then the stored uploads into it; `display/src/state.rs` carries a drag the
  browser starts with the pointer (Smithay's DnD grab) and `display/src/shell.rs` draws its icon above the scene,
  outside what input hits. `browser/launch/src/files.rs` stores each pipe's bytes and reports the path.
- `client/src/uploads.ts` sends one file at a time within the window; `client/src/viewer.ts` opens the person's file
  picker for a file request (directly while their click still counts as a gesture, else through an admin page button)
  and turns files dragged over the screen into the page's drag (`file-drag`) and files dropped on it into a drop.

## Sessions, the lease, and the agent

- `admin/src/app.rs` holds the browser sessions, their viewers, and the lease.
- `admin/src/api.rs` serves the HTTP API and the viewer WebSocket.
- `admin/src/auth.rs` handles enrollment (the setup link and `admin setup-link`), `admin init`, login, and cookies; `admin/src/tokens.rs` the agent tokens;
  `admin/src/local.rs` the local MCP socket and `admin mcp`, its stdio relay.
- `admin/src/service.rs` holds the session operations every way in shares: starting, describing and stopping a
  session, and one agent call with its admission, takeover blocking and redaction. The administrator's page
  (`api.rs`), the agent API (`agent_api.rs`, under `/api/agent/`) and the MCP endpoint (`mcp.rs`, at `/mcp` and on the local socket) each
  authenticate their caller as one principal (`app::Owner`) and call it; `admin/src/tools.rs` pins the driver's desktop
  tools a token may call (`tools.json`).
- To start a session, the admin asks the controller (`admin/src/controller.rs`, one JSON line per request over a Unix
  socket) to create its four containers. `controller/src/template.rs` builds them from fixed templates, and
  `controller/src/guard.rs` re-checks each container body against the closed set immediately before creating it.
- The agent is the Cua Driver's MCP server. `admin/src/agent.rs` reaches it through the controller's `agent_attach`
  operation: one fixed exec of `agent-mcp`, as the agent's user, over its standard input and output. The controller
  accepts no other command. Its privileged Wayland protocols are `display/src/wlr_toplevels.rs`, `screencopy.rs`,
  `virtual_keyboard.rs`, and smithay's `ext_foreign_toplevel_list_v1` and virtual pointer; `display/src/inject.rs` serves the `cua-inject v1` socket.
- Taking control blocks new agent calls and waits for the in-flight one. If it does not end in time, the admin asks
  the controller to kill the agent container and proves it stopped.

## Process placement

On hosts with many CPUs, the kernel may move a busy thread between sockets, which costs cache locality.
`affinity::stay_on_cores()` pins `display` and the browser launcher (and therefore Chrome) to the physical core of
the CPU each starts on and, when the container's CPU quota is above one CPU, to the next cores of the same socket: the
quota rounded up. On a two-socket, 48-CPU test host, with 1 to 16 sessions encoding AV1 at once, an encoder kept on
those neighbouring cores ran 30 fps at one session and 21 at sixteen, against 25 and 19 when pinned to its whole socket
and 23 and 19 with no pinning.
