<div align="center">

<a href="https://easi.run"><img src="site/public/favicon.svg" width="72" alt="easi.run"></a>

# easi.run

**Isolated browser sessions for your agents. Self-hosted. Yours.**

Give every agent its own real Chrome on your own server. Watch it live, and take over with one click when it hits
a login, a 2FA code or a CAPTCHA. Then hand it back.

[![CI](https://github.com/julianosig/easi.run/actions/workflows/ci.yml/badge.svg)](https://github.com/julianosig/easi.run/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-2457f5)](LICENSE)
[![GitHub stars](https://img.shields.io/github/stars/julianosig/easi.run?style=flat&color=f06445)](https://github.com/julianosig/easi.run/stargazers)
[![Last commit](https://img.shields.io/github/last-commit/julianosig/easi.run)](https://github.com/julianosig/easi.run/commits/main)
[![Built with Rust](https://img.shields.io/badge/built_with-Rust-b7410e?logo=rust)](https://www.rust-lang.org)
[![MCP](https://img.shields.io/badge/MCP-built--in-6e56cf)](https://docs.easi.run/#/agents)

[**▶ Try the live demo**](https://easi.run/demo/) · [Documentation](https://docs.easi.run) ·
[Quick start](#quick-start) · [Compare](https://easi.run/#compare)

<a href="https://easi.run/demo/"><img src="docs/assets/hero.png" alt="easi.run: a live Chrome session in your browser, with drag and drop, takeover, uploads, downloads, sound, typing, and copy and paste" width="860"></a>

</div>

## What is it?

Agents that use the web need a browser. Most teams rent one by the hour from a cloud service, or glue together a
headless Chrome and a VNC server. easi.run is a third way: **one command starts it on your own Linux server**, and
from then on every agent gets a fresh, isolated Google Chrome, and you get a live window into each one.

When the agent gets stuck at something only a person can do, you click **Take control**. The agent waits. You type
the password, approve the 2FA prompt, or solve the puzzle. You click **Give back**, and the agent carries on. What
you typed (four characters or more) reaches the agent as `[redacted]`.

## Why people use it

- 💸 **No per-hour bills.** It runs on your server, so you pay for the server and nothing else. No plan limits on
  how long a session runs.
- 🙋 **A person can step in.** One click to take over, and the agent waits until you hand it back.
- 🔒 **Your passwords stay yours.** Text you type or paste while you are in control (four characters or more) reaches
  the agent as `[redacted]`, and the agent gets no screenshots after a takeover.
- 🧱 **Every browser in its own sandbox.** Each session gets its own containers; the browser has no network of its
  own and no way into your private network.
- 🕵️ **A real Chrome that sites treat as a real Chrome.** Google Chrome itself, clean on BrowserScan, sannysoft and
  rebrowser's bot detector ([what sites see](docs/stealth.md)).
- 📁 **Files, sound and clipboard just work.** Drag a file onto the page, pick one with the page's own upload
  button, get downloads straight into your Downloads folder, hear the page, and copy and paste both ways.
- 🎥 **Record a session** on the server in high quality, with your camera and microphone if you like.
- 🤖 **Plugs into your agent.** A built-in [MCP server](https://docs.easi.run/#/agents) that works with Claude Code,
  and a plain REST API, each with its own token per agent.
- 🌍 **Your own proxy per session**, with the browser's time zone and languages set to match where it connects from.
- ⚡ **Fast and light.** Crisp text, smooth video, and a fraction of the bandwidth of a VNC stream (numbers below).

## How it compares

| | **easi.run** | Browserbase | Browserless | KasmVNC |
|---|:---:|:---:|:---:|:---:|
| Runs on your own server | ✅ | – | ✅ | ✅ |
| No usage fees | ✅ | – | – | ✅ |
| Take over, and the agent waits | ✅ | – | ✅ | – |
| Your typing hidden from the agent | ✅ | – | – | – |
| An MCP server for your agents | ✅ | ✅ | ✅ | – |
| Drag a file straight onto the page | ✅ | – | – | – |
| Downloads land on your computer | ✅ | – | – | – |
| Live sound | ✅ | – | – | – |
| Open source | ✅ AGPL-3.0 | – | SSPL | ✅ GPL-2.0 |

✅ has it · – doesn't, partly, not in their docs, or not applicable. The [full comparison](https://easi.run/#compare)
covers 21 criteria and 19 alternatives, each cell [with its source](docs/comparison.md) (October 2026).

## Quick start

You need a Linux x86-64 server with Docker Engine 28 or newer. No GPU needed.

```sh
git clone https://github.com/julianosig/easi.run && cd easi.run
sudo iptables -I INPUT -i er-+ -j DROP        # pages reach the internet, never the server's own services
DOCKER_GID=$(stat -c %g /var/run/docker.sock) docker compose up -d --build
docker compose logs admin | grep -i enroll    # a one-time enrollment token, valid for 15 minutes
```

Save the firewall rule in your host's persistent firewall configuration, so it holds after a reboot. It is written for
Docker's default userland proxy.

1. Open `https://localhost:9443`. On a remote server, reach it through SSH first:
   `ssh -L 9443:127.0.0.1:9443 you@your-server`. The certificate is self-signed, and the admin logs its SHA-256
   fingerprint so you can check it.
2. Enroll a password of at least 12 characters with the token. (Token expired? `docker compose restart admin` and run
   the last line again.)
3. Start a session with a public web address, watch it live, and press **Take control** whenever you want.

The [documentation](https://docs.easi.run) covers configuration, upstream proxies, limits per session and operations.

## Connect your agent

Create a token under **Agent tokens** on the admin page, then add easi.run to Claude Code:

```sh
# Claude Code checks certificates: trust the admin's own one first.
docker compose cp admin:/var/lib/admin/tls-cert.pem easi-run.pem
export NODE_EXTRA_CA_CERTS=$PWD/easi-run.pem

claude mcp add --transport http easi https://localhost:9443/mcp \
  --header "Authorization: Bearer easi_…"
```

Ask it to open a session and browse. Each session it starts comes with a `watch_url`, where you can watch and take
over. Prefer plain HTTP? The same tools are under `/api/agent/` ([Agents guide](https://docs.easi.run/#/agents)).

## Performance

Against KasmVNC on the same host and CPU budget, medians of the headless runs:

| | easi.run | KasmVNC |
|---|---|---|
| Smooth scroll | **55 fps**, 1.8 Mbit/s | 24 fps, 12.4 Mbit/s |
| Full-motion animation | **38 fps**, 2.0 Mbit/s | 24 fps, 25.4 Mbit/s |
| 1080p video | **36 fps**, 2.1 Mbit/s | 24 fps, 18.1 Mbit/s |
| Scrolling text | **1.9 Mbit/s**, SSIM 0.990 | 16.4 Mbit/s, SSIM 0.977 |
| Typing, key press to drawn frame | **28 ms** | 69 ms |
| Static page | no media traffic | no media traffic |

Every metric, the method and the raw results are in [the benchmark](docs/benchmark.md).

## How it works

```text
              ┌───────────── session (one per browser) ──────────────┐
 Internet ◀── │ proxy ◀─unix── browser ──wayland──▶ display ◀── agent  │
              └──────────────────────────────────────│───────────────┘
                                                unix │ link frames
                                                     ▼
 person's browser ◀──── HTTPS / WebSocket ──── admin ──── controller ──▶ Docker
```

A small compositor written in Rust (`display`) draws Chrome's windows into its own memory. Text and still content
travel as lossless tiles, motion as AV1 video, and the page's sound as Opus, all over one HTTPS port. Only the viewer
who holds control can type and click, and taking control stops the agent first, and proves it stopped.

- [Security model](docs/security.md): every boundary, and the test that proves it
- [Architecture](docs/architecture.md): where each piece of code lives, following a frame and a key press
- [Design](docs/design.md) and [topology](docs/topology.md): the reasoning, the containers, their limits

<details>
<summary><b>Repository layout</b></summary>

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
| `site/` | the easi.run landing and its documentation pages, and how they are published |
| `demo/` | the public demo's installation: the walled-garden pages and their Compose file |
| `third_party/` | a patched dependency ([`third_party/README.md`](third_party/README.md)) |

</details>

## Contributing

Issues and pull requests are welcome. [`CONTRIBUTING.md`](CONTRIBUTING.md) covers building, testing and review, and
`scripts/dev/run.sh scripts/check.sh` runs the checks a change needs in the same image CI uses. Please report security
problems privately, as [`SECURITY.md`](SECURITY.md) describes.

## License

easi.run is free software under the [GNU Affero General Public License, version 3 only](LICENSE). Run it, change it,
and share it; if you offer a modified version to people over a network, offer them its source too. Every dependency
keeps its own license, listed in [`docs/licenses.md`](docs/licenses.md). The published images encode video only as
AV1, which is royalty-free; an H.264 encoder is an opt-in build feature (`DISPLAY_FEATURES=h264`).

<div align="center">

**If easi.run saves your agents a headache, a ⭐ helps others find it.**

</div>
