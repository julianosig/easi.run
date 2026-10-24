# easi.run

A remote screen for a browser that an agent drives and a person can take over. A headless Wayland compositor,
written in Rust, shows one sandboxed Google Chrome to a web page. Static content goes as lossless tiles, motion goes
as AV1 video, and the page's sound as Opus. The person can take control from the agent at any moment and hand it back,
copy and paste both ways, receive the page's downloads, and give the page files through its own file chooser or by
dragging them onto the screen. A session's traffic can leave through an upstream proxy the deployment names (a
residential one, say), the browser's time zone and languages matching where it leaves.

It is built to be safer and lighter than running a VNC server next to the browser:

- **Isolation first.** Chrome runs in a container with no network, no capability, and a read-only root. The
  compositor never maps Chrome's memory, and Chrome never produces the bytes the viewer decodes. By default every
  role has its own uid, and only one small controller holds the Docker socket.
- **Exact control.** Input comes only from the connection that holds the control lease, at the current lease epoch.
  The admin application and the compositor each check that independently. A takeover stops the agent first, and
  proves it stopped.
- **Efficient on the wire.** A static, silent page sends no media, only statistics and keepalives. Text and other static
  content stay bit-exact. Motion is encoded once per viewer at a few Mbit/s, with backpressure before encoding.
- **No hidden surface.** The only listener is one HTTPS port on loopback. There is no UDP and no outbound connection
  from the compositor, and the wire protocol is closed and versioned.

## Performance

Against KasmVNC on the same host and CPU budget, the medians of five headless runs ([method and every
number](docs/benchmark.md)):

| | easi.run | KasmVNC |
|---|---|---|
| Smooth scroll | 55 fps, 1.8 Mbit/s | 24 fps, 12.4 Mbit/s |
| Full-motion animation | 38 fps, 2.0 Mbit/s | 24 fps, 25.4 Mbit/s |
| 1080p video | 36 fps, 2.1 Mbit/s | 24 fps, 18.1 Mbit/s |
| Scrolling text, bitrate and SSIM | 1.9 Mbit/s, SSIM 0.990 | 16.4 Mbit/s, SSIM 0.977 |
| Flat blocks changing every frame | 54 fps, lossless | 24 fps, SSIM 0.993 |
| Typing, key press to drawn frame | 28 ms | 69 ms |
| Static page | no media traffic | no media traffic |

Capture-to-draw latency, from the page drawing a frame to the viewer drawing it, stays under 50 ms for scrolling and
is 63–68 ms for full-motion animation and video on this GPU-less test host, against KasmVNC's 83–100 ms. KasmVNC uses
31–59 MiB less memory in video motion, where easi.run keeps an AV1 encoder running, and 7–17 MiB less otherwise,
and slightly less CPU on three motion fixtures, where it presents 44–65% as many frames. [The benchmark](docs/benchmark.md#every-fixture-headless-medians)
lists every metric, and explains [where the latency goes](docs/benchmark.md#what-limits-capture-to-draw).

> **Status: early.** easi.run runs on one Linux x86-64 host with Docker. It has been tested with Chrome
> clients only. The benchmark against KasmVNC and its open items are in [`docs/benchmark.md`](docs/benchmark.md).

## Quick start

You need Linux on x86-64 with Docker Engine 28 or newer.

```sh
git clone https://github.com/julianosig/easi.run && cd easi.run
DOCKER_GID=$(stat -c %g /var/run/docker.sock) docker compose up -d --build
docker compose logs admin | grep -i enroll    # a one-time enrollment token, valid for 15 minutes
# expired? docker compose restart admin, then run the line above again: the last token is the new one
```

Then:

1. Open `https://localhost:9443`. The certificate is self-signed; the admin logs its SHA-256 fingerprint so you can
   check it.
2. Enroll a password of at least 12 characters with the token.
3. Start a session with a URL.

Watch the page live, or press **Take control** to use it yourself, then **Give back**.

`DOCKER_GID` gives the controller the group of the Docker socket. `BROWSER_CPUS` (default 2) and
`BROWSER_MEMORY_MIB` (default 2048), set in the environment or a `.env` file next to `compose.yaml`, change each
browser session's limits; the defaults are [the measured balance](docs/benchmark.md#browser-limits). Only the `admin` and `controller` containers run all
the time. Each browser session gets its own `display`, `proxy`, `browser`, and `agent` containers, created from fixed
templates. The controller resolves image tags once, when it starts, so restart it after rebuilding a session image:
`docker compose restart controller`.

Pages reach only public addresses, and the host's own public address is one: a page could reach a host service
through it. Drop what the session bridges (`er-<session>`) send the host itself, and keep the rule across reboots. The
rule is written for Docker's default userland proxy (`"userland-proxy": true`, as Docker ships it):

```sh
sudo iptables -I INPUT -i er-+ -j DROP
```

### The agent and DevTools

Each session runs the [Cua Driver](https://github.com/trycua/cua) as its agent. It sees the page through the
accessibility tree and acts through the compositor's virtual input devices. Chrome's DevTools endpoint is off by
default. It can be turned on per session for the driver's navigation tools, as
[`docs/agent-compatibility.md`](docs/agent-compatibility.md) describes.

An agent of your own gets a token from the admin page and then starts and drives its own sessions, over MCP at `/mcp`
or plain HTTP under `/api/agent/`, while you watch and take over from the admin page (see the
[Agents page](https://docs.easi.run/#/agents) of the documentation).

## How it works

```text
              ┌───────────── session (one per browser) ──────────────┐
 Internet ◀── │ proxy ◀─unix── browser ──wayland──▶ display ◀── agent  │
              └──────────────────────────────────────│───────────────┘
                                                unix │ link frames
                                                     ▼
 person's browser ◀──── HTTPS / WebSocket ──── admin ──── controller ──▶ Docker
```

The compositor, `display`, composes Chrome's surfaces into its own memory. Each viewer connection gets its own
encoder thread, which sends the changed parts of damaged tiles losslessly and switches to video while the page moves. The admin
application relays each viewer's stream and checks every message from the viewer. The person's input travels back
along the same path, but only from the connection that holds the lease.

- [Documentation](https://docs.easi.run/): installing, configuring, and operating easi.run, its agent API, and custom
  clients.
- [Architecture](docs/architecture.md): where each piece of code lives, following a frame and a key press.
- [Design](docs/design.md): the requirements and the reasoning behind them.
- [Topology](docs/topology.md): every container, uid, mount, network, and limit.
- [Security model](docs/security.md): the threat model, each boundary, and the test that proves it.
- [Benchmark](docs/benchmark.md): the method and the results against KasmVNC.
- [What sites see](docs/stealth.md): why the browser reads as an ordinary Chrome, and what bot detectors say.

## Repository layout

| Path | What |
|---|---|
| `display/` | the headless Wayland compositor (Smithay) and the per-viewer encoder |
| `protocol/` | the wire protocol, the admin-display link framing, and golden vectors |
| `client/` | the TypeScript viewer, the admin page, and the benchmark page |
| `admin/` | the HTTPS application: enrollment, login, sessions, the viewer relay, the lease, the agent, agent tokens, the agent API and MCP |
| `controller/` | the only Docker-socket holder: a closed set of operations on fixed templates |
| `proxy/` | the browser's egress proxy, which refuses private and special destinations |
| `browser/`, `agent/` | the Chrome image and the Cua Driver image |
| `egress/`, `locale/` | an upstream proxy and a session's time zone and languages, as every holder checks them |
| `affinity/` | keeps a process on the physical core it starts on |
| `bench/` | the fixtures, the KasmVNC baseline, the measuring runner, and results |
| `tests/` | end-to-end tests, hostile Wayland clients, and fuzz targets |
| `scripts/` | the checks, the development image, SBOMs, and the license report |
| `site/` | the easi.run landing and its documentation pages (static Svelte), and how they are published (`site/deploy/`) |
| `demo/` | the public demo's installation: the walled-garden pages and their Compose file |
| `third_party/` | a patched dependency ([`third_party/README.md`](third_party/README.md)) |

## Contributing

[`CONTRIBUTING.md`](CONTRIBUTING.md) covers building, testing, and review. `scripts/dev/run.sh scripts/check.sh` runs
the checks a change needs, in the same development image CI uses. Report security problems privately, as
[`SECURITY.md`](SECURITY.md) describes.

## License

easi.run is licensed under the [GNU Affero General Public License, version 3 only](LICENSE) (`AGPL-3.0-only`). If you
run a modified version for people over a network, offer them its source. The vendored `third_party/wayland-backend`
keeps its MIT license, and every dependency keeps its own: they are listed in [`docs/licenses.md`](docs/licenses.md),
which is generated from the images' SBOMs.

Copyright licenses do not settle codec patents. The published images encode only AV1, which AOMedia's royalty-free
patent license covers. H.264 is patent-encumbered (its AVC pool patents run until at least late 2027 in the US), and
Cisco's patent coverage for OpenH264 applies only to Cisco's own prebuilt binary, not to a build from source, so the
H.264 encoder is an opt-in build feature (`DISPLAY_FEATURES=h264`) for deployments with their own patent position.
KasmVNC (GPL-2.0) is used only by the benchmark profile and is never part of a runtime image.
