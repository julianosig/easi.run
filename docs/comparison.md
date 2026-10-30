# easi.run compared

The sources behind the comparison table on the easi.run page. The research on other products was verified on
2026-10-21 (running on your own server, usage fees, the live view, takeover, hidden typing, the private network,
stealth, sound, files, the clipboard and recording), 2026-10-23 (limits on browsers at once and on session length,
your own takeover screen, the agent API and MCP server, per-session switches, your own proxy, the one HTTPS port and
open source) and 2026-10-24 (saved logins encrypted with a key only you unlock). The statements about easi.run were
checked against its code on 2026-10-27.

Scope: the comparison table on the page covers cloud browsers for agents and self-hosted browser-streaming stacks,
researched from each product's own documentation, pricing pages, repositories and source code, with security and
trust pages for saved logins. "Not documented" and "not in their docs" mean the pages checked say nothing on the
point; that does not prove the feature is absent. "Not checked" means the research did not look. Prices are list
prices with the billing unit the vendor uses, and "(calc)" marks arithmetic on those prices. Each product has one
section below, whose bullets are named after the page's rows, in the table's order, and give the cell and the
sources behind it, with quotes and details in parentheses; "Live view" is context, not a row. Self-hosted free
software has no plan, so its "no plan limit" cells note the product's own structure or timeouts instead.

Two rows need a definition:

- **An API your agents can call:** Yes means a session starts over HTTP (REST, or an SDK on top of it) and is driven
  over HTTP or the CDP endpoint the API hands back. Partial means there is an HTTP API, but it can't start sessions,
  can't send input, or only reads them. No means no HTTP API.
- **An MCP server for your agents:** Yes means the vendor publishes an official MCP server, hosted or run yourself,
  that gives agents its browser. No means the vendor has an MCP offering, but none gives agents its browser or
  desktop. "Not in their docs" means no MCP server in their docs, README or repository. Community servers don't
  count; they are listed with the sources. The self-hosted tools have no agent, so their MCP cells are not
  applicable, as for takeover and hidden typing.

## The products

### easi.run

- **Runs on your own server:** Yes: one command on a Linux x86-64 server with Docker Engine 28 or newer —
  [README](../README.md)
- **No usage fees:** Yes: no browser-hour charge; free software, you pay only for your server — [README](../README.md),
  [LICENSE](../LICENSE)
- **No plan limit on browsers at once:** Yes: as many as the server's memory holds, up to 16 per server
  (`ADMIN_MAX_SESSIONS`, which the installer sets from the memory) — [Configuration docs](../site/src/docs/pages/Configuration.svelte),
  [site/src/docs/config.ts](../site/src/docs/config.ts), [install.sh](../install.sh) (`max_sessions`),
  [admin/src/main.rs](../admin/src/main.rs) (`MAX_SESSIONS`), [controller/src/main.rs](../controller/src/main.rs),
  [admin/src/app.rs](../admin/src/app.rs) (the place a starting session reserves)
- **No plan limit on session length:** Yes: a session runs until it is stopped; a session age limit exists only for the
  public demo's walled garden, and a login's idle expiry closes that login's streams, not the session —
  [site/src/docs/config.ts](../site/src/docs/config.ts) (`CONTROLLER_MAX_SESSION_SECONDS`),
  [controller/src/main.rs](../controller/src/main.rs), [admin/src/auth.rs](../admin/src/auth.rs) (`IDLE_EXPIRY`)
- **Live view:** Lossless tiles, pixel-exact, and AV1 on the region in motion; over WebTransport on UDP with BBR where it
  gets through, the WebSocket otherwise — [README](../README.md), [design](design.md), [benchmark](benchmark.md),
  [admin/src/udp.rs](../admin/src/udp.rs)
- **Take over, and the agent waits:** Yes: one click; the agent waits until you give it back — [security](security.md#boundaries) (Agent),
  [Sessions docs](../site/src/docs/pages/Sessions.svelte) (Control), [admin/src/agent.rs](../admin/src/agent.rs)
- **Build your own takeover screen:** Yes: a viewer SDK (`/sdk/viewer.js`) draws the screen and takes input; the bars,
  menus and prompts are yours — [Custom clients docs](../site/src/docs/pages/Clients.svelte), [client/src/sdk.ts](../client/src/sdk.ts)
- **An API your agents can call:** Yes: a REST API under `/api/agent/` with a token per agent, which reaches only the
  sessions it started and the runs of the saved sessions you allow it — [Agents docs](../site/src/docs/pages/Agents.svelte), [security](security.md#boundaries) (Agent
  tokens), [admin/src/agent_api.rs](../admin/src/agent_api.rs), [admin/src/tokens.rs](../admin/src/tokens.rs)
- **An MCP server for your agents:** Yes: a built-in MCP endpoint, `POST /mcp` on the admin's port with the same token,
  or on the server itself (`docker exec … admin mcp`, over SSH from elsewhere) with no token; works with Claude Code and
  Codex — [Agents docs](../site/src/docs/pages/Agents.svelte), [admin/src/mcp.rs](../admin/src/mcp.rs),
  [admin/src/local.rs](../admin/src/local.rs)
- **Your typing hidden from the agent:** Yes: text of four characters or more you type or paste after taking over
  reaches the agent as `[redacted]`, password fields' values never reach it, and screenshots leave its results from the
  first takeover on — [security](security.md#boundaries) (What the agent
  learns of the person), [admin/src/redact.rs](../admin/src/redact.rs)
- **Encrypted session vaults only your password unlocks:** Yes: logins, site data (caches aside), history and
  preferences kept between sessions, encrypted with a key your password unlocks; locked after a restart until you enter it; agents
  resume only the ones you allow — [security](security.md#boundaries) (Persistent sessions),
  [admin/src/profiles.rs](../admin/src/profiles.rs), [admin/src/keys.rs](../admin/src/keys.rs)
- **Private network off-limits:** Yes: Chrome has no network of its own, and its proxy refuses private and special
  addresses — [security](security.md#boundaries) (Browser to everything else), [egress/src/destination.rs](../egress/src/destination.rs)
- **Built-in stealth:** Yes: real Google Chrome, `navigator.webdriver` false, nothing spoofed from JavaScript; clean on
  BrowserScan, bot.sannysoft.com and rebrowser's bot detector (2026-10-21, from a datacenter address) —
  [stealth](stealth.md)
- **Live sound:** Yes: the page's sound, live, as Opus; pages get no audio capture — [security](security.md#boundaries) (Browser's sound),
  [design](design.md) (3.3), [browser policy](../browser/policy.json)
- **Drag a file straight onto the page:** Yes: drop files right on the page, with real progress where you dropped
  them — [display/src/drop.rs](../display/src/drop.rs), [client/src/ui/TransferCard.svelte](../client/src/ui/TransferCard.svelte)
- **Your own file picker for uploads:** Yes: the page's own file chooser opens your computer's picker, and the files
  go straight into the page — [security](security.md#boundaries) (Uploads and the file chooser), [browser/launch/src/portal.rs](../browser/launch/src/portal.rs),
  [client/src/uploads.ts](../client/src/uploads.ts)
- **Downloads land on your computer:** Yes: streamed into your browser's Downloads as they are written, any size, byte
  for byte, only to whoever holds control; a page saved with Chrome's Save as arrives whole, as one MHTML file —
  [security](security.md#boundaries) (Downloads), [design](design.md) (3.6), [client/src/files.ts](../client/src/files.ts)
- **Copy and paste both ways:** Yes: text both ways, with your usual shortcuts — [security](security.md#boundaries) (Clipboard), [design](design.md)
  (3.6), [client/src/copy.ts](../client/src/copy.ts)
- **Record the session:** Yes: on the server in high quality, with your camera and microphone, streamed into your
  Downloads as WebM — [Sessions docs](../site/src/docs/pages/Sessions.svelte) (Recording),
  [client/src/recorder.ts](../client/src/recorder.ts), [display/src/recording.rs](../display/src/recording.rs),
  [display/src/stream/record.rs](../display/src/stream/record.rs)
- **Turn off input, clipboard, files or sound per session:** Yes: input, clipboard, uploads, downloads, sound and
  recording, each off for a session's life (`features` on `POST /api/sessions`), enforced on the server whatever the
  client sends — [Custom clients docs](../site/src/docs/pages/Clients.svelte), [admin/src/app.rs](../admin/src/app.rs)
  (`Features`), [admin/src/relay.rs](../admin/src/relay.rs)
- **Your own proxy per session, location matched:** Yes: an upstream proxy (HTTP, HTTPS or SOCKS5) the deployment
  lists and a session picks, checked before use; the browser takes that proxy's time zone and languages, suggested from
  the country its check finds when it is added and yours to change; name lookups go through it (DNS over HTTPS); a saved
  session remembers its last proxy, which the dashboard offers again and agents reuse unless they name another —
  [Egress docs](../site/src/docs/pages/Egress.svelte), [admin/src/egresses.rs](../admin/src/egresses.rs),
  [egress/src/lib.rs](../egress/src/lib.rs), [stealth](stealth.md)
- **Works over one HTTPS port, no TURN:** Yes: one HTTPS port serves the page and every viewer's WebSocket, which carries
  everything; where UDP gets through, the media also take a WebTransport session on the admin's UDP port; no WebRTC,
  so no TURN server — [README](../README.md), [design](design.md) (3.5), [compose.yaml](../compose.yaml),
  [topology](topology.md)
- **Open source you can audit:** Yes: easi.run's own code is AGPL-3.0-only, everything around the browser; Chrome
  itself is Google's, built into the image from Google's signed package — [LICENSE](../LICENSE), [README](../README.md),
  [licenses](licenses.md)

### Browserbase

- **Runs on your own server:** No, cloud only; self-host and air-gapped use are marked "No" — https://docs.browserbase.com/platform/browser/getting-started/remote-browser-versus-local-browser
- **No usage fees:** No: Developer plan $20/month with 100 browser-hours, then $0.12/h, billed per minute; proxy 1 GB included, then $12/GB — https://www.browserbase.com/pricing, https://docs.browserbase.com/account/billing/plans.md
- **No plan limit on browsers at once:** No: 3 free, 25 on Developer, 100 on Startup, 250+ on Scale — https://docs.browserbase.com/account/billing/plans.md , https://www.browserbase.com/pricing
  ("| **Concurrency** | 3 | 25 | 100 | 250+ |"; session creation is also rate-limited per minute.)
- **No plan limit on session length:** No: 15 minutes free; 6 hours on Developer and Startup ("6+ hrs" on Scale) — https://docs.browserbase.com/account/billing/plans.md , https://docs.browserbase.com/reference/api/create-a-session.md
  ("| **Session Duration** | 15 mins | 6 hrs | 6 hrs | 6+ hrs |"; `timeout` "Minimum 60 seconds, maximum 21600 seconds (6 hours)".)
- **Live view:** Iframe live view (`debuggerFullscreenUrl`); transport not documented — https://docs.browserbase.com/platform/browser/observability/session-live-view.md
- **Take over, and the agent waits:** Partial: the live view is interactive; no pause of the agent documented — https://docs.browserbase.com/platform/browser/observability/session-live-view.md
- **Build your own takeover screen:** Partial: an iframe of their viewer; the navbar can be hidden — https://docs.browserbase.com/platform/browser/observability/session-live-view.md , https://docs.browserbase.com/reference/api/session-live-urls.md
  ("Add the live view link to an iframe in your frontend to embed it."; `navbar=false`.)
- **An API your agents can call:** Yes: REST API and Node and Python SDKs — https://docs.browserbase.com/reference/api/create-a-session.md , https://docs.browserbase.com/reference/sdk/overview.md
- **An MCP server for your agents:** Yes: official MCP server, hosted or run locally — https://docs.browserbase.com/integrations/mcp/introduction.md , https://github.com/browserbase/mcp-server-browserbase
  ("The hosted Streamable HTTP endpoint is served on Browserbase infrastructure"; local STDIO is also supported.)
- **Your typing hidden from the agent:** Not documented — https://docs.browserbase.com/platform/browser/observability/session-live-view.md
- **Encrypted session vaults only your password unlocks:** Not documented: Contexts "live indefinitely on Browserbase's infrastructure" and "are uniquely encrypted at rest"
  (AES-256-CBC, with a public key Browserbase provides); who holds the decrypting key is not documented; Enterprise
  BYOS can store contexts in your own S3 bucket — https://docs.browserbase.com/platform/browser/core-features/contexts.md,
  https://docs.browserbase.com/reference/api/create-a-context.md, https://docs.browserbase.com/account/enterprise/byos-setup-guide.md
- **Private network off-limits:** Not documented — https://docs.browserbase.com/platform/identity/proxies.md
- **Built-in stealth:** Yes: Basic stealth on Developer, Advanced Stealth (custom Chromium) on Scale — https://www.browserbase.com/pricing, https://www.browserbase.com/changelog/advanced-stealth
- **Live sound:** Not documented — https://docs.browserbase.com/platform/browser/observability/session-live-view.md
- **Drag a file straight onto the page:** Not documented — https://docs.browserbase.com/platform/browser/files/uploads, https://docs.browserbase.com/platform/browser/observability/session-live-view.md
- **Your own file picker for uploads:** No: the page's chooser opens a remote picker that "can't access files on your local machine"; the documented bridge is developer-built — https://docs.browserbase.com/platform/browser/files/uploads
- **Downloads land on your computer:** No: downloads go to cloud storage and come back through an API — https://docs.browserbase.com/platform/browser/files/downloads, https://docs.browserbase.com/reference/api/get-download.md
- **Copy and paste both ways:** Not documented: only the iframe's `allow="clipboard-read; clipboard-write"` — https://docs.browserbase.com/platform/browser/observability/session-live-view.md
- **Record the session:** Yes: HLS video segments, MP4 download after the session — https://browserbase.com/blog/session-recordings, https://docs.browserbase.com/platform/browser/observability/recording-downloads.md
- **Turn off input, clipboard, files or sound per session:** Partial: recording per session; view-only is CSS on your iframe — https://docs.browserbase.com/reference/api/create-a-session.md , https://docs.browserbase.com/platform/browser/observability/session-live-view.md
  (`browserSettings.recordSession`: "Enable or disable session recording. Defaults to `true`." The read-only example is `style="pointer-events: none;"` on the iframe. No upload, download, clipboard or audio switch in the create-session schema.)
- **Your own proxy per session, location matched:** Partial: own proxy per session; the time zone follows only the built-in proxy — https://docs.browserbase.com/features/proxies.md , https://docs.browserbase.com/platform/browser/techniques/timezones.md
  (Bring your own with `type: "external"` and server, user name and password at session creation. The time zone is aligned automatically "when you use a Browserbase proxy"; with an external proxy it is overridden per page through CDP `Emulation.setTimezoneOverride`. Locale and languages: not documented.)
- **Works over one HTTPS port, no TURN:** Not applicable: cloud only — https://docs.browserbase.com/platform/browser/getting-started/remote-browser-versus-local-browser
- **Open source you can audit:** No: closed cloud service — https://docs.browserbase.com/platform/browser/getting-started/remote-browser-versus-local-browser

### Steel

- **Runs on your own server:** Yes, open-source: steel-browser is Apache-2.0 and in public beta; Steel Local allows one session with limited stealth. The other rows describe the cloud — https://github.com/steel-dev/steel-browser, https://docs.steel.dev/overview/self-hosting/steel-local-vs-steel-cloud
- **No usage fees:** No: Launch $0.10/h; Scale $250/month plus $0.08/h; proxy $10/GB (Launch) or $6/GB (Scale); billed per minute, rounded up — https://docs.steel.dev/overview/pricinglimits
- **No plan limit on browsers at once:** No: 10 on Launch, 100 on Scale, 1,000+ on Enterprise — https://docs.steel.dev/overview/pricinglimits
- **No plan limit on session length:** No: 15 minutes on Launch, 1 hour on Scale, 24 hours on Enterprise; 5 minutes by default — https://docs.steel.dev/overview/sessions-api/session-lifecycle , https://docs.steel.dev/overview/sessions-api/configuration
- **Live view:** WebRTC H.264, 25 fps (headful); CDP screencast (headless) — https://docs.steel.dev/overview/sessions-api/embed-sessions/live-sessions
- **Take over, and the agent waits:** Partial: through the MCP tool `steel_session_handoff` — https://github.com/steel-dev/steel-mcp-server, https://docs.steel.dev/overview/sessions-api/human-in-the-loop
- **Build your own takeover screen:** Partial: an iframe of their viewer — https://docs.steel.dev/overview/sessions-api/embed-sessions/live-sessions
  ("Embed the session directly in your UI using an iframe.")
- **An API your agents can call:** Yes: REST API and Node and Python SDKs — https://docs.steel.dev/overview/sessions-api/overview , https://steel.apidocumentation.com/api-reference
- **An MCP server for your agents:** Yes: official MCP server you run from source; the hosted one is not live yet — https://github.com/steel-dev/steel-mcp-server , https://docs.steel.dev/cookbook/mcp
  (MIT, status 3.0.0; the README says "`mcp.steel.dev` is not live yet" and "not published to npm"; it works with Steel Cloud or a self-hosted steel-browser. Steel's docs have only a cookbook MCP you build yourself.)
- **Your typing hidden from the agent:** Not documented — https://docs.steel.dev/overview/sessions-api/human-in-the-loop
- **Encrypted session vaults only your password unlocks:** Not documented: profiles snapshot the User Data Directory (up to 300 MB) in Steel's cloud and are deleted after 30 days unused; their
  encryption is not documented (the separate Credentials feature uses a per-organization KMS key), and one API key
  reaches every profile of the organization — https://docs.steel.dev/overview/profiles-api/overview,
  https://docs.steel.dev/overview/credentials-api/overview, https://docs.steel.dev/overview/authentication
- **Private network off-limits:** Not documented — https://docs.steel.dev/overview/stealth/proxies
- **Built-in stealth:** Yes, on Enterprise only — https://steel.dev/pricing
- **Live sound:** Not documented — https://docs.steel.dev/overview/sessions-api/embed-sessions/live-sessions
- **Drag a file straight onto the page:** Not documented — https://docs.steel.dev/overview/sessions-api/human-in-the-loop
- **Your own file picker for uploads:** No: Files API upload, then `setInputFiles` with the session path — https://docs.steel.dev/overview/files-api/overview, https://docs.steel.dev/overview/sessions-api/human-in-the-loop, https://steel.dev/blog/webrtc
- **Downloads land on your computer:** No: Files API or a zip archive (`downloadArchive`) — https://docs.steel.dev/overview/files-api/overview, https://docs.steel.dev/cookbook/files
- **Copy and paste both ways:** Not documented — https://docs.steel.dev/overview/sessions-api/embed-sessions/live-sessions
- **Record the session:** Yes: headful sessions as HLS — https://docs.steel.dev/overview/sessions-api/embed-sessions/past-sessions
- **Turn off input, clipboard, files or sound per session:** Partial: interactive on or off; no other switch documented — https://docs.steel.dev/overview/sessions-api/configuration
  ("Set `debugConfig.interactive` to `false`, or add `interactive=false` to the viewer URL." Clipboard, upload, download, audio and recording switches: not on the configuration, live-sessions, past-sessions, human-in-the-loop or Files API pages.)
- **Your own proxy per session, location matched:** Partial: own proxy per session; time zone and language not documented — https://docs.steel.dev/overview/sessions-api/configuration , https://docs.steel.dev/overview/stealth/proxies
  (Bring your own with `proxyUrl` or `useProxy.server` at session creation; `proxyUrl` overrides `useProxy`. The configuration page lists no time zone or locale option and does not say they follow the proxy.)
- **Works over one HTTPS port, no TURN:** Unknown: the self-hosted ports are not in the pages read
- **Open source you can audit:** Yes: steel-browser is Apache-2.0 — https://github.com/steel-dev/steel-browser

### Hyperbrowser

- **Runs on your own server:** Not documented; only HyperAgent (AGPL-3.0) and the MCP server (MIT) are open source — https://raw.githubusercontent.com/hyperbrowserai/HyperAgent/main/LICENSE, https://api.github.com/orgs/hyperbrowserai/repos
- **No usage fees:** No: $0.10 per browser-hour; proxy $10/GB; $0.02 per agent step — https://hyperbrowser.ai/docs/pricing.md
- **No plan limit on browsers at once:** No: 1 free, 25 on Startup, 100 on Scale, 1,000+ on Enterprise — https://www.hyperbrowser.ai/pricing
- **No plan limit on session length:** No: `timeoutMinutes` from 1 to 720 (12 hours); per-plan caps not in their docs — https://hyperbrowser.ai/docs/sessions/create.md , https://hyperbrowser.ai/docs/sessions/lifecycle.md
- **Live view:** Embeddable `liveUrl` (12-hour token, view-only option); transport not documented — https://hyperbrowser.ai/docs/sessions/live-view.md
- **Take over, and the agent waits:** Partial: the live view is interactive; no pause of the agent documented — https://hyperbrowser.ai/docs/sessions/live-view.md
- **Build your own takeover screen:** Partial: an iframe of their viewer — https://hyperbrowser.ai/docs/sessions/live-view.md
  ("You can embed Live View directly into your application using an iframe.")
- **An API your agents can call:** Yes: REST API and Node and Python SDKs — https://hyperbrowser.ai/docs/api-reference/create-new-session.md , https://hyperbrowser.ai/docs/sdks/introduction.md
  (`POST /api/session`.)
- **An MCP server for your agents:** Yes: official MCP server, run locally; scrape, crawl and task tools — https://hyperbrowser.ai/docs/integrations/model-context-protocol.md , https://github.com/hyperbrowserai/mcp
  (`npx hyperbrowser-mcp`, no hosted endpoint; MIT. Tools: scrape, crawl, extract, Bing search, the browser-use, OpenAI CUA and Claude computer-use agents, profiles; no session tools.)
- **Your typing hidden from the agent:** Not documented — https://hyperbrowser.ai/docs/sessions/live-view.md
- **Encrypted session vaults only your password unlocks:** Not documented: Hyperbrowser saves a profile's user data directory (profiles can be forked); its security pages say "Production databases
  and managed storage systems are encrypted at rest", without naming profiles or who holds the key —
  https://hyperbrowser.ai/docs/sessions/profiles.md, https://www.hyperbrowser.ai/security, https://trust.hyperbrowser.ai
- **Private network off-limits:** Unknown: not documented in the pages checked
- **Built-in stealth:** Yes: stealth mode, Ultra Stealth on Enterprise — https://hyperbrowser.ai/docs/sessions/stealth.md
- **Live sound:** Not documented — https://hyperbrowser.ai/docs/sessions/live-view.md
- **Drag a file straight onto the page:** Unknown: not checked (the research did not cover Hyperbrowser's live view)
- **Your own file picker for uploads:** Unknown: not checked
- **Downloads land on your computer:** Unknown: not checked
- **Copy and paste both ways:** Unknown: not checked
- **Record the session:** Yes: rrweb by default; MP4 optional — https://hyperbrowser.ai/docs/sessions/recordings.md
- **Turn off input, clipboard, files or sound per session:** Partial: view-only, recording and saved downloads per session — https://hyperbrowser.ai/docs/sessions/parameters.md
  (`viewOnlyLiveView`: "Enable read-only Live View which disables user interactions."; `enableWebRecording`, `enableVideoWebRecording`, `saveDownloads`. Clipboard, upload and audio switches: not in their docs.)
- **Your own proxy per session, location matched:** Partial: own proxy per session, Enterprise plan only; language settable — https://hyperbrowser.ai/docs/sessions/proxy.md (also at https://docs.hyperbrowser.ai/sessions/advanced-privacy-and-anti-detection/proxies)
  (Bring your own with `proxyServer`, `proxyServerUsername` and `proxyServerPassword`; HTTP, HTTPS or SOCKS5; "only available on the Enterprise plan"; set at creation. Session parameters include `locales` (ISO 639-1) per https://docs.hyperbrowser.ai/reference/sdks/python/sessions , seen in a search result snippet rather than fetched. No time zone field found; no automatic matching documented.)
- **Works over one HTTPS port, no TURN:** Not applicable: cloud only — https://hyperbrowser.ai/docs/sessions/live-view.md
- **Open source you can audit:** No: the cloud browser is closed; only the agent and MCP are open — https://raw.githubusercontent.com/hyperbrowserai/HyperAgent/main/LICENSE , https://api.github.com/orgs/hyperbrowserai/repos

### Anchor

- **Runs on your own server:** Yes, on Enterprise: on-premises or bring-your-own cloud — https://docs.anchorbrowser.io/pricing.md
- **No usage fees:** No: docs list $0.05 per browser-hour plus $0.01 per browser created, proxy $8/GB; the marketing pricing page lists $0.09/h — https://docs.anchorbrowser.io/pricing.md, https://anchorbrowser.io/pricing
- **No plan limit on browsers at once:** No: up to 5 free, 25 on Starter, 200 on Growth, 500+ on Enterprise — https://docs.anchorbrowser.io/pricing.md
- **No plan limit on session length:** Yes: you set each session's maximum, with no upper limit — https://docs.anchorbrowser.io/advanced/session-timeout.md , https://docs.anchorbrowser.io/api-reference/browser-sessions/start-browser-session.md
  ("There is no upper limit on how long you can set the maximum duration." The guide gives a 180-minute default; the API reference says `max_duration` "Defaults to `20`. Set to `-1` to disable this limit." The idle timeout is 5 minutes by default, `-1` disables it.)
- **Live view:** VNC passing Chrome screencast JPEGs (inferred from Anchor's open-sourced GoVNC) — https://github.com/anchorbrowser/GoVNC, https://docs.anchorbrowser.io/advanced/browser-live-view
- **Take over, and the agent waits:** Partial: a "need human input" webhook — https://docs.anchorbrowser.io/llms.txt
- **Build your own takeover screen:** Partial: an iframe of their viewer — https://docs.anchorbrowser.io/advanced/browser-live-view.md
- **An API your agents can call:** Yes: REST API and TypeScript and Python SDKs — https://docs.anchorbrowser.io/api-reference/browser-sessions/start-browser-session.md , https://docs.anchorbrowser.io/sdk-reference/overview.md
- **An MCP server for your agents:** Yes: official MCP server, hosted; step-by-step browser control — https://docs.anchorbrowser.io/advanced/mcp , https://docs.anchorbrowser.io/advanced/mcp-open-source.md
  (Hosted at `https://api.anchorbrowser.io/v1/mcp`, plus `/v1/mcp/browser` for live step-by-step control; an open-source variant also exists.)
- **Your typing hidden from the agent:** Not documented — https://docs.anchorbrowser.io/advanced/browser-live-view
- **Encrypted session vaults only your password unlocks:** Not documented: their security page says all customer data is stored in AWS `us-east-2` (and also that they "do not store
  customer data from within the browser sessions", which stored profiles contradict); AES-256 at rest is stated for "configuration data or account
  information", not for profiles, and who holds the key is not documented —
  https://docs.anchorbrowser.io/essentials/managed-authentication.md, https://docs.anchorbrowser.io/security.md
- **Private network off-limits:** Not documented — https://docs.anchorbrowser.io/network-and-stealth/overview
- **Built-in stealth:** Yes: Extra Stealth on Growth ($2,000/month) — https://docs.anchorbrowser.io/network-and-stealth/overview, https://docs.anchorbrowser.io/pricing.md
- **Live sound:** Not documented — https://docs.anchorbrowser.io/advanced/browser-live-view.md
- **Drag a file straight onto the page:** Not documented — https://docs.anchorbrowser.io/advanced/browser-live-view.md
- **Your own file picker for uploads:** No: API or Playwright only; the uploads API returns paths for `DOM.setFileInputFiles` — https://docs.anchorbrowser.io/advanced/file-upload.md, https://docs.anchorbrowser.io/api-reference/browser-sessions/upload-files.md
- **Downloads land on your computer:** No: stored by Anchor and fetched through an API, or a P2P stream to your code — https://docs.anchorbrowser.io/advanced/file-download.md, https://docs.anchorbrowser.io/advanced/p2p-downloads.md
- **Copy and paste both ways:** Not documented for the viewer: an iframe `allow` attribute plus a get/set clipboard API for automation (that API's description comes from the llms.txt listing only) — https://docs.anchorbrowser.io/advanced/browser-live-view.md, https://docs.anchorbrowser.io/advanced/os-level-control.md
- **Record the session:** Yes: MP4, on by default — https://docs.anchorbrowser.io/essentials/recording
- **Turn off input, clipboard, files or sound per session:** Partial: read-only live view and recording per session — https://docs.anchorbrowser.io/api-reference/browser-sessions/start-browser-session.md , https://docs.anchorbrowser.io/essentials/recording.md
  (`session.live_view.read_only`, `session.recording.active`. Clipboard is only the iframe `allow` attribute; upload and audio switches: not in their docs.)
- **Your own proxy per session, location matched:** Partial: own proxy per session; time zone and language not documented — https://docs.anchorbrowser.io/advanced/bring-your-own-proxy.md , https://docs.anchorbrowser.io/advanced/proxy.md
  (Bring your own with a session `proxy` object: `type: "custom"`, `server` (http, https or socks5), user name and password. The built-in proxy targets a country, region or city. No time zone or locale mention.)
- **Works over one HTTPS port, no TURN:** Not applicable: cloud only — https://docs.anchorbrowser.io/pricing.md
- **Open source you can audit:** No: closed cloud service; on-premises is Enterprise only — https://docs.anchorbrowser.io/pricing.md

### Browserless

- **Runs on your own server:** Yes, with limits: an SSPL-1.0 image; stealth, captcha, recording, BQL and webhooks are not in the open-source build — https://github.com/browserless/browserless, https://docs.browserless.io/enterprise/open-source
- **No usage fees:** No: units of 30 s; plans $25–350/month billed annually; about $0.18/h at Scale overage (calc) — https://www.browserless.io/pricing, https://docs.browserless.io/overview/unit-consumption
- **No plan limit on browsers at once:** No: 2 free; 10, 40 or 100 on the paid plans; Enterprise custom — https://www.browserless.io/pricing
  ("2 max concurrent browsers", "10 max concurrent browsers", "40 …", "100 …"; the page also shows "+5", "+10", "+20" beside the paid numbers without saying what they mean.)
- **No plan limit on session length:** No: 2 minutes free; 15, 30 or 60 minutes on the paid plans — https://www.browserless.io/pricing
  ("2 min. max session time", "15 min.", "30 min.", "60 min. max session time".)
- **Live view:** JPEG frames on a canvas over WebSocket — https://docs.browserless.io/baas/interactive-browser-sessions/hybrid-automation
- **Take over, and the agent waits:** Yes: instruction card with Done / Couldn't do it, and a `Browserless.liveComplete` event — https://docs.browserless.io/baas/interactive-browser-sessions/hybrid-automation
- **Build your own takeover screen:** Partial: an iframe of their viewer — https://docs.browserless.io/baas/monitor-sessions/embedding-live-url.md
  ("you can embed the live browser inside your own UI with an iframe.")
- **An API your agents can call:** Yes: REST APIs, a session API, and TypeScript and Python SDKs — https://docs.browserless.io/rest-apis/quick-start.md , https://docs.browserless.io/open-api/session-create.md , https://docs.browserless.io/bap/getting-started.md
- **An MCP server for your agents:** Yes: official MCP server, hosted — https://docs.browserless.io/mcp/browserless-mcp-server
  (Hosted at `https://mcp.browserless.io/mcp`, with a stateful `browserless_agent` tool.)
- **Your typing hidden from the agent:** Partial: after a 1Password autologin fill, screenshots and screencasts are blocked and live views torn down — https://docs.browserless.io/baas/features/autologin
- **Encrypted session vaults only your password unlocks:** Not documented: profiles are scoped to your API token and kept in their cloud (customer data "primarily stored in the United
  States"); "Datastores containing sensitive
  customer data are encrypted at rest using encryption mechanisms provided by the underlying cloud infrastructure",
  profiles not named — https://docs.browserless.io/baas/features/authenticated-profiles.md, https://www.browserless.io/trust
- **Private network off-limits:** Not documented — https://docs.browserless.io/baas/bot-detection/proxies
- **Built-in stealth:** Yes: stealth routes and OS emulation — https://docs.browserless.io/baas/bot-detection/os-emulation
- **Live sound:** No: the live view has no audio; audio only in WebM recordings — https://docs.browserless.io/baas/interactive-browser-sessions/hybrid-automation, https://docs.browserless.io/baas/monitor-sessions/screen-recording, https://docs.browserless.io/baas/monitor-sessions/hybrid-automation.md
- **Drag a file straight onto the page:** Not documented — https://docs.browserless.io/baas/advanced-configurations/hybrid-automation-configurations.md
- **Your own file picker for uploads:** No: API only, `Browserless.uploadFile` with base64 data — https://docs.browserless.io/baas/features/file-transfers.md, https://docs.browserless.io/api-reference/cdp-extensions.md
- **Downloads land on your computer:** No: streamed to your script (CDP base64 or Playwright `saveAs`) — https://docs.browserless.io/baas/features/file-transfers.md
- **Copy and paste both ways:** Yes: "The viewer supports copy and paste in both directions"; text vs image not documented — https://docs.browserless.io/baas/advanced-configurations/hybrid-automation-configurations.md
- **Record the session:** Yes: rrweb replay; WebM screen recording — https://docs.browserless.io/baas/interactive-browser-sessions/session-replay, https://docs.browserless.io/baas/monitor-sessions/screen-recording
- **Turn off input, clipboard, files or sound per session:** Partial: watch-only Live URLs, with input dropped on their side; recording per session — https://docs.browserless.io/baas/monitor-sessions/embedding-live-url.md , https://docs.browserless.io/baas/monitor-sessions/screen-recording.md
  ("Browserless drops viewer input on the server before it reaches the remote browser." `interactable` is set per Live URL; `record=true` on the connection. Clipboard, upload, download and audio switches: not in their docs.)
- **Your own proxy per session, location matched:** Partial: own proxy per session; the language matches the built-in proxy only — https://docs.browserless.io/baas/bot-detection/proxies.md , https://docs.browserless.io/browserql/launch-parameters
  (Bring your own with the `externalProxyServer=http://user:pass@host:port` query parameter on the connection. `proxyLocaleMatch` sets the browser language to the proxy country on the stealth endpoints, with the built-in `proxyCountry`; time zone matching is not mentioned, and the docs say to set it with launch parameters.)
- **Works over one HTTPS port, no TURN:** Unknown: the self-hosted image's ports are not in the pages read
- **Open source you can audit:** Partial: SSPL-1.0 image; stealth and more are not in the open build — https://github.com/browserless/browserless , https://docs.browserless.io/enterprise/open-source

### Kernel

- **Runs on your own server:** Yes, open-source: kernel-images is Apache-2.0 (Docker or Unikraft). The other rows describe the cloud — https://github.com/kernel/kernel-images
- **No usage fees:** No: billed per second: about $0.06/h headless, $0.48/h headful (GPU about $2.88/h); no charge for idle time or proxies — https://kernel.sh/docs/info/pricing.md, https://www.kernel.sh/pricing
- **No plan limit on browsers at once:** No: 5 on Developer (free), 10 on Hobbyist, 150 on Start-Up, custom on Enterprise — https://kernel.sh/docs/browsers/concurrency-and-limits.md
  ("| Concurrent browsers | 5 | 10 | 150 | Custom |"; browsers in standby count.)
- **No plan limit on session length:** Unknown: no maximum in their docs; a session ends after its inactivity timeout, 60 seconds by default and at most 72 hours — https://kernel.sh/docs/api-reference/browsers/create-a-browser-session.md , https://kernel.sh/docs/browsers/termination.md
- **Live view:** WebRTC, adapted from neko — https://github.com/kernel/kernel-images, https://kernel.sh/docs/browsers/live-view.md
- **Take over, and the agent waits:** Partial: interactive live view with a runtime read-only toggle; no pause of the agent documented — https://kernel.sh/docs/browsers/live-view.md
- **Build your own takeover screen:** Partial: an iframe of their viewer, with status messages to the parent page — https://kernel.sh/docs/browsers/live-view.md
  ("The live view URL can be embedded in an iframe to integrate the browser view into your own application or dashboard.")
- **An API your agents can call:** Yes: REST API and TypeScript, Python and Go SDKs — https://kernel.sh/docs/api-reference/browsers/create-a-browser-session.md , https://kernel.sh/docs/quickstart.md
  (`POST https://api.onkernel.com/browsers`; SDKs `@onkernel/sdk`, `kernel` and `kernel-go-sdk`.)
- **An MCP server for your agents:** Yes: official MCP server, hosted — https://kernel.sh/docs/reference/mcp-server.md , https://kernel.sh/docs/reference/mcp-server/transports.md
  (Hosted at `https://mcp.onkernel.com/mcp`; tools include browser management, computer actions and Playwright.)
- **Your typing hidden from the agent:** Not documented — https://kernel.sh/docs/browsers/live-view.md
- **Encrypted session vaults only your password unlocks:** No: "Profile data is encrypted end to end using a per-organization key", and their security page says "Kernel
  uses its cloud provider's key management service to encrypt data at rest and to store and manage encryption keys"
  ("Cloud provider managed keys"); which key that is for profiles, and who holds it, is not stated; a profile
  downloads as a zstd-compressed tar archive; scoped API keys can reach a single profile — https://kernel.sh/docs/browsers/profiles.md,
  https://kernel.sh/docs/security.md, https://kernel.sh/docs/api-reference/profiles/download-profile-archive.md,
  https://kernel.sh/docs/info/api-key-permissions.md
- **Private network off-limits:** Yes: egress allowlists — https://kernel.sh/docs/browsers/egress-allowlists.md
- **Built-in stealth:** Yes: `stealth:true` (ISP proxy plus captcha solver) — https://kernel.sh/docs/browsers/bot-detection/stealth.md
- **Live sound:** Unknown: the open-source image's README calls its WebRTC audio "currently non-functional", while its config routes browser audio to the live view; the hosted service's audio is not documented — https://github.com/kernel/kernel-images, https://raw.githubusercontent.com/onkernel/kernel-images/main/images/chromium-headful/neko.yaml
- **Drag a file straight onto the page:** Not documented; the open-source client has no drop handler (src) — https://raw.githubusercontent.com/onkernel/kernel-images/main/images/chromium-headful/client/src/components/video.vue
- **Your own file picker for uploads:** No: API only (`setInputFiles`, File I/O) — https://kernel.sh/docs/browsers/file-io.md
- **Downloads land on your computer:** No: `~/Downloads` through File I/O APIs, or a directory zip, only while the session lives — https://kernel.sh/docs/browsers/file-io.md
- **Copy and paste both ways:** Yes: two-way text (text/plain only) — https://kernel.sh/docs/browsers/live-view.md, https://github.com/onkernel/kernel-images
- **Record the session:** Yes: H.264 MP4 replays — https://kernel.sh/docs/browsers/replays.md
- **Turn off input, clipboard, files or sound per session:** Partial: a read-only flag on the viewer URL; recording on demand — https://kernel.sh/docs/browsers/live-view.md , https://kernel.sh/docs/browsers/replays.md
  ("`readOnly` (bool): when set to `true`, the view will be non-interactive." It is a URL query parameter, toggled from the parent page by `KERNEL_SET_READ_ONLY`. Clipboard is the iframe `allow` attribute; upload, download and audio switches: not in their docs.)
- **Your own proxy per session, location matched:** Partial: own proxy per session; time zone and language not documented — https://kernel.sh/docs/proxies/overview.md , https://kernel.sh/docs/browsers/create-a-browser.md
  (Bring your own with the "custom" proxy type, attached by `proxy_id` when creating a browser and swappable on a running session through `browsers.update`. Neither page mentions time zone, locale or language.)
- **Works over one HTTPS port, no TURN:** Unknown: the ports of a self-hosted kernel-images are not in the pages read
- **Open source you can audit:** Yes: kernel-images is Apache-2.0 — https://github.com/kernel/kernel-images

### Browser Use Cloud

- **Runs on your own server:** No: the browser-use framework is MIT; the cloud browser is not self-hostable — https://github.com/browser-use/browser-use, https://docs.browser-use.com/open-source/customize/browser/remote.md
- **No usage fees:** No: $0.02 per browser-hour, billed per minute; the full timeout is charged upfront and refunded pro rata; proxy $5/GB — https://browser-use.com/pricing, https://docs.browser-use.com/cloud/api-v4/browsers/create-browser-session.md
- **No plan limit on browsers at once:** No: 10 at first, rising with total spend ($25,000 paid gives 1,000) — https://browser-use.com/pricing , https://docs.browser-use.com/cloud/guides/concurrency.md
- **No plan limit on session length:** No: up to 4 hours (60 minutes by default) — https://docs.browser-use.com/cloud/api-v4/browsers/create-browser-session.md
  ("All users can use up to 240 minutes (4 hours).")
- **Live view:** Iframe `liveUrl`; read-only is UI-only — https://docs.browser-use.com/cloud/browser/live-preview.md
- **Take over, and the agent waits:** Partial: interactive `liveUrl`; no pause of the agent documented — https://docs.browser-use.com/cloud/browser/live-preview.md
- **Build your own takeover screen:** Partial: an iframe of their viewer — https://docs.browser-use.com/cloud/browser/live-preview.md
- **An API your agents can call:** Yes: REST API and Python and TypeScript SDKs — https://docs.browser-use.com/cloud/api-v4-overview.md , https://docs.browser-use.com/cloud/api-v4/browsers/create-browser-session.md
- **An MCP server for your agents:** Yes: official MCP server, hosted; it hands whole tasks to their agent — https://docs.browser-use.com/cloud/guides/mcp-server.md
  (Hosted at `https://api.browser-use.com/v3/mcp`; tools such as `run_session`, `send_task` and `stop_session`; no step-by-step click or type tools.)
- **Your typing hidden from the agent:** Not documented — https://docs.browser-use.com/cloud/browser/live-preview.md
- **Encrypted session vaults only your password unlocks:** Not documented: profiles belong to a project in their cloud; the company states "AES-256 at rest" without naming
  profiles or who holds the key — https://docs.browser-use.com/cloud/guides/profile-sync.md, https://browser-use.com/security
- **Private network off-limits:** Unknown: not documented in the pages checked
- **Built-in stealth:** Yes: hardened Chromium fork with randomized fingerprints — https://docs.browser-use.com/cloud/browser/stealth.md
- **Live sound:** Not documented — https://docs.browser-use.com/cloud/browser/live-preview.md
- **Drag a file straight onto the page:** Not documented — https://docs.browser-use.com/cloud/browser/live-preview.md
- **Your own file picker for uploads:** Not documented — https://docs.browser-use.com/cloud/browser/live-preview.md
- **Downloads land on your computer:** Not documented — https://docs.browser-use.com/cloud/browser/live-preview.md
- **Copy and paste both ways:** Unknown: not checked
- **Record the session:** Yes: MP4, off by default — https://docs.browser-use.com/cloud/browser/live-preview.md
- **Turn off input, clipboard, files or sound per session:** Partial: recording per session; read-only is a viewer option they do not enforce — https://docs.browser-use.com/cloud/browser/live-preview.md , https://docs.browser-use.com/cloud/api-v4/browsers/create-browser-session.md
  ("View-only embeds are a UI restriction, not a server-enforced permission." `enableRecording`, off by default. Clipboard, upload, download and audio switches: not in their docs.)
- **Your own proxy per session, location matched:** Partial: own proxy per session; time zone and language not documented — https://docs.browser-use.com/cloud/api-v4/browsers/create-browser-session.md
  (Bring your own with `customProxy` {host, port, username, password, ignoreCertErrors} on the create-session request; the default is their proxy by `proxyCountryCode`. No time zone, locale or language field on the page.)
- **Works over one HTTPS port, no TURN:** Not applicable: cloud only — https://docs.browser-use.com/open-source/customize/browser/remote.md
- **Open source you can audit:** Partial: framework MIT; the cloud browser is closed — https://github.com/browser-use/browser-use , https://docs.browser-use.com/open-source/customize/browser/remote.md

### Bright Data Browser API

- **Runs on your own server:** No, cloud only — https://docs.brightdata.com/scraping-automation/scraping-browser/introduction
- **No usage fees:** No: browser time free; $8/GB pay-as-you-go ($499/month for 71 GB) — https://brightdata.com/pricing/scraping-browser, https://docs.brightdata.com/scraping-automation/scraping-browser/faqs.md
- **No plan limit on browsers at once:** Yes: unlimited on the paid plans — https://docs.brightdata.com/products/scraping-browser/introduction.md , https://brightdata.com/pricing/scraping-browser
  ("unlimited concurrent browser sessions"; the free tier's card states no number.)
- **No plan limit on session length:** No: up to 60 minutes; 5 minutes idle ends it — https://docs.brightdata.com/products/scraping-browser/configuration.md
  ("Browser API session can last up to 60 minutes.")
- **Live view:** Chrome DevTools frontend only (`Page.inspect`) — https://docs.brightdata.com/scraping-automation/scraping-browser/features/troubleshooting
- **Take over, and the agent waits:** No: DevTools only — https://docs.brightdata.com/scraping-automation/scraping-browser/features/troubleshooting
- **Build your own takeover screen:** No: a DevTools link only (`Page.inspect`) — https://docs.brightdata.com/products/scraping-browser/features/troubleshooting.md
- **An API your agents can call:** Partial: sessions start over a CDP WebSocket; REST only reads session details — https://docs.brightdata.com/scraping-automation/scraping-browser/introduction , https://docs.brightdata.com/api-reference/browser-api/get-session.md
  (Puppeteer, Playwright or Selenium connect over the WebSocket; `GET /browser_sessions/{id}` reads a session; no create or stop endpoint is referenced.)
- **An MCP server for your agents:** Yes: official MCP server, hosted or run locally; browser tools included — https://docs.brightdata.com/ai/mcp-server/faqs , https://docs.brightdata.com/ai/mcp-server/remote/advanced , https://github.com/brightdata/brightdata-mcp
  (Hosted at `https://mcp.brightdata.com/...` and local; `scraping_browser_*` navigate, click, type and screenshot tools.)
- **Your typing hidden from the agent:** Not documented — https://docs.brightdata.com/scraping-automation/scraping-browser/faqs.md
- **Encrypted session vaults only your password unlocks:** Not documented: no saved-login profile in their docs, and "Browser API is configured by default to
  prevent any attempts to log in to accounts by disabling password entry" — https://docs.brightdata.com/products/scraping-browser/faqs
- **Private network off-limits:** Unknown: not documented in the pages checked
- **Built-in stealth:** Yes: unblocking is the product (fingerprints, residential rotation, captcha solving) — https://docs.brightdata.com/scraping-automation/scraping-browser/introduction
- **Live sound:** Not offered — inferred from the DevTools-only view (https://docs.brightdata.com/scraping-automation/scraping-browser/features/troubleshooting)
- **Drag a file straight onto the page:** Not documented — https://docs.brightdata.com/products/scraping-browser/cdp-functions/custom.md
- **Your own file picker for uploads:** Not documented — https://docs.brightdata.com/products/scraping-browser/cdp-functions/custom.md
- **Downloads land on your computer:** Not documented — https://docs.brightdata.com/products/scraping-browser/cdp-functions/custom.md
- **Copy and paste both ways:** Unknown: not checked
- **Record the session:** Unknown: not documented in the pages checked
- **Turn off input, clipboard, files or sound per session:** Unknown: the only human view is DevTools; no switches in their docs — https://docs.brightdata.com/products/scraping-browser/features/troubleshooting.md
- **Your own proxy per session, location matched:** No: their own proxy network only; location picked by country — https://docs.brightdata.com/scraping-automation/scraping-browser/features/proxy-location
  (The page documents only Bright Data peers, the `-country` flag, radius targeting and `Proxy.setLocation`. No bring-your-own proxy found. `Proxy.getGeolocation` returns the peer's `tz`; whether the browser's time zone and locale follow it is not stated.)
- **Works over one HTTPS port, no TURN:** Not applicable: cloud only — https://docs.brightdata.com/scraping-automation/scraping-browser/introduction
- **Open source you can audit:** No: closed cloud service — https://docs.brightdata.com/scraping-automation/scraping-browser/introduction

### Cloudflare Browser Run

- **Runs on your own server:** No, cloud only — https://developers.cloudflare.com/browser-run/pricing/
- **No usage fees:** No: $0.09/h after 10 included hours, plus $2 per extra concurrent browser — https://developers.cloudflare.com/browser-run/pricing/
- **No plan limit on browsers at once:** No: 3 per account on Workers Free, 200 on Workers Paid; 10 included, then $2 per extra browser — https://developers.cloudflare.com/browser-run/limits/ , https://developers.cloudflare.com/browser-run/pricing/
- **No plan limit on session length:** Partial: no maximum while active; closed after 1 minute idle by default, at most 20 — https://developers.cloudflare.com/browser-run/limits/
  ("There is no fixed maximum lifetime for a browser session as long as it remains active. … You can increase this inactivity timeout to up to 20 minutes." Free accounts get 10 browser-minutes a day.)
- **Live view:** CDP WebSocket to one tab, signed JWT links — https://developers.cloudflare.com/browser-run/features/live-view/
- **Take over, and the agent waits:** Yes: `Cloudflare.handoff` with instructions and a timeout of up to 30 minutes; the person clicks Done or Failed — https://developers.cloudflare.com/browser-run/features/human-in-the-loop/
- **Build your own takeover screen:** Unknown: their hosted viewer; embedding not in their docs — https://developers.cloudflare.com/browser-run/features/live-view/ , https://developers.cloudflare.com/browser-run/features/human-in-the-loop/
  ("A generated URL opens Cloudflare's hosted interface in any browser." Neither page mentions iframes or a client SDK.)
- **An API your agents can call:** Yes: REST API to open and close sessions; Workers binding — https://developers.cloudflare.com/browser-run/cdp/session-management/index.md , https://developers.cloudflare.com/browser-run/reference/browser-binding-api/index.md
  (`POST` and `DELETE` on `.../browser-run/devtools/browser`.)
- **An MCP server for your agents:** Yes: hosted MCP for pages and screenshots; a Playwright MCP you deploy drives the browser — https://developers.cloudflare.com/agents/model-context-protocol/mcp-servers-for-cloudflare/index.md , https://developers.cloudflare.com/browser-run/playwright/playwright-mcp/index.md , https://developers.cloudflare.com/browser-run/cdp/mcp-clients/index.md
  (The hosted `https://browser.mcp.cloudflare.com/mcp` will "Fetch web pages, convert them to markdown and take screenshots"; `@cloudflare/playwright-mcp` is deployed to your own Worker; the third-party `chrome-devtools-mcp` can also be pointed at Browser Run.)
- **Your typing hidden from the agent:** Not documented — https://developers.cloudflare.com/browser-run/features/human-in-the-loop/
- **Encrypted session vaults only your password unlocks:** Not documented: no managed profile is documented; their Playwright example keeps storage state in your own Workers KV,
  where "Encryption keys are managed by Cloudflare" — https://developers.cloudflare.com/browser-run/playwright/index.md,
  https://developers.cloudflare.com/kv/reference/data-security/index.md
- **Private network off-limits:** Unknown: not documented in the pages checked
- **Built-in stealth:** No: "always identified as bot traffic", with Web Bot Auth signing — https://developers.cloudflare.com/browser-run/features/human-in-the-loop/
- **Live sound:** Not offered — inferred from the CDP single-tab view (https://developers.cloudflare.com/browser-run/features/live-view/)
- **Drag a file straight onto the page:** Not documented — https://developers.cloudflare.com/browser-run/features/live-view/
- **Your own file picker for uploads:** Not documented — https://developers.cloudflare.com/browser-run/features/live-view/
- **Downloads land on your computer:** Not documented — https://developers.cloudflare.com/browser-run/features/live-view/
- **Copy and paste both ways:** Unknown: not checked
- **Record the session:** Yes: rrweb kept 30 days, with HAR 1.2 export — https://developers.cloudflare.com/browser-run/features/session-recording/
- **Turn off input, clipboard, files or sound per session:** Partial: read-only links, blocked on their side; recording opt-in per session — https://developers.cloudflare.com/browser-run/features/live-view/ , https://developers.cloudflare.com/browser-run/features/session-recording/
  ("A read-only Live View guardrail applies only to the generated connection. Other connections and automation scripts can still control the session." Clipboard, upload, download and audio switches: not in their docs.)
- **Your own proxy per session, location matched:** Unknown: not in their docs — https://developers.cloudflare.com/browser-run/limits/
  (The limits page and overview show no proxy, time zone, locale or language setting.)
- **Works over one HTTPS port, no TURN:** Not applicable: cloud only — https://developers.cloudflare.com/browser-run/pricing/
- **Open source you can audit:** No: closed cloud service — https://developers.cloudflare.com/browser-run/pricing/

### AWS AgentCore Browser

- **Runs on your own server:** No, cloud only — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-tool.html
- **No usage fees:** No: $0.0895 per vCPU-hour plus $0.00945 per GB-hour; idle time free — https://aws.amazon.com/bedrock/agentcore/pricing/
- **No plan limit on browsers at once:** Partial: 1,000 active sessions per account by default, raised by a support ticket — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/bedrock-agentcore-limits.html
- **No plan limit on session length:** No: up to 8 hours — https://docs.aws.amazon.com/bedrock-agentcore/latest/APIReference/API_StartBrowserSession.html
  ("Maximum allowed: 28,800 seconds (8 hours)." The default is 3600 seconds there and 900 seconds in https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-resource-session-management.html.)
- **Live view:** Amazon DCV, embeddable with SigV4-presigned URLs — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-dcv-integration.html
- **Take over, and the agent waits:** Partial: the DCV view is interactive; no pause of the agent documented — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-dcv-integration.html
- **Build your own takeover screen:** Yes: a React component on the Amazon DCV Web Client renders the live view in your page — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-dcv-integration.md
  ("use the BrowserLiveView React component from the Amazon Bedrock AgentCore TypeScript SDK … while retaining full control over your UI.")
- **An API your agents can call:** Yes: AWS API and SDKs; automation over CDP — https://docs.aws.amazon.com/bedrock-agentcore/latest/APIReference/API_StartBrowserSession.md
  (`PUT /browsers/{id}/sessions/start` returns an automation stream and a live view stream; Boto3, JavaScript v3, Go, Java and other SDKs.)
- **An MCP server for your agents:** Yes: AWS Labs MCP server, run locally; browser tools included — https://github.com/awslabs/mcp/tree/main/src/amazon-bedrock-agentcore-mcp-server , https://awslabs.github.io/mcp/servers/amazon-bedrock-agentcore-mcp-server
  (Run with `uvx`; 25 browser tools, from `start_browser_session` to `browser_navigate` and `browser_click`.)
- **Your typing hidden from the agent:** Not documented — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-features.html
- **Encrypted session vaults only your password unlocks:** No: "AgentCore encrypts your data using AWS owned encryption keys from AWS Key Management Service";
  CreateBrowserProfile takes no KMS key; access per profile through IAM —
  https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/data-encryption.md,
  https://docs.aws.amazon.com/bedrock-agentcore-control/latest/APIReference/API_CreateBrowserProfile.html
- **Private network off-limits:** Not documented — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-features.html
- **Built-in stealth:** Not documented: Web Bot Auth signing is offered; stealth is not documented — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-features.html
- **Live sound:** Not documented — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-dcv-integration.html
- **Drag a file straight onto the page:** Not documented — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-features.html
- **Your own file picker for uploads:** Not documented — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-features.html
- **Downloads land on your computer:** Not documented — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-features.html
- **Copy and paste both ways:** Unknown: not checked
- **Record the session:** Yes: DOM, console and network captured into your S3 bucket — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-features.html
- **Turn off input, clipboard, files or sound per session:** Unknown: no viewer switches in their docs; recording is set per browser, not per session — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-resource-session-management.html , https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-enterprise-policies.html
  (Chrome policies can be applied per session; the page names no download or clipboard policy.)
- **Your own proxy per session, location matched:** Partial: own proxy per session; time zone and language not documented — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-proxies.md
  (Bring your own with `proxyConfiguration.proxies[].externalProxy` on StartBrowserSession: HTTP or HTTPS with CONNECT, basic auth through Secrets Manager, no SOCKS, set once at creation, up to 5 proxies, not validated at creation. The page says nothing on time zone or locale.)
- **Works over one HTTPS port, no TURN:** Not applicable: cloud only — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-tool.html
- **Open source you can audit:** No: closed cloud service — https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-tool.html

### E2B Desktop

- **Runs on your own server:** Yes: Apache-2.0; "E2B Embed" runs on a single self-hosted node — https://github.com/e2b-dev/desktop, https://e2b.dev/open-source
- **No usage fees:** No: $0.0504 per vCPU-hour plus $0.0162 per GiB-hour; about $0.17/h at 2 vCPU and 4 GiB (calc) — https://e2b.dev/pricing
- **No plan limit on browsers at once:** No: 20 sandboxes on Hobby, 100 to 1,100 on Pro, 1,100+ on Enterprise — https://docs.e2b.dev/billing.md
- **No plan limit on session length:** No: 1 hour on Hobby, 24 hours on Pro of continuous runtime, reset by pause and resume — https://docs.e2b.dev/faq/sandbox-lifetime.md
- **Live view:** x11vnc plus E2B's noVNC fork — https://docs.e2b.dev/template/examples/desktop
- **Take over, and the agent waits:** Partial: a shared VNC stream with `viewOnly`; no pause of the agent documented — https://raw.githubusercontent.com/e2b-dev/desktop/main/README.md
- **Build your own takeover screen:** Partial: a hosted noVNC page; no viewer SDK documented — https://docs.e2b.dev/template/examples/desktop.md
- **An API your agents can call:** Yes: REST API and Python and JavaScript Desktop SDKs — https://docs.e2b.dev/api-reference/sandboxes/create-sandbox-v2.md , https://docs.e2b.dev/sdk-reference/desktop-python-sdk/v2.4.2/desktop.md , https://github.com/e2b-dev/desktop
- **An MCP server for your agents:** No: none drives the desktop; their MCP gateway runs other servers in a sandbox — https://docs.e2b.dev/mcp-gateway.md , https://github.com/e2b-dev/mcp-server
  (The gateway "runs inside sandboxes" with Docker MCP Catalog tools and no desktop tools; `e2b-dev/mcp-server` runs code only and was archived on 2026-04-16.)
- **Your typing hidden from the agent:** Not documented — https://raw.githubusercontent.com/e2b-dev/desktop/main/README.md
- **Encrypted session vaults only your password unlocks:** No: no browser-profile feature is documented; a paused sandbox keeps its filesystem and memory, and on E2B's
  managed cloud "Sandbox storage sits under Google Cloud's default encryption at rest" ("No E2B key material") — https://docs.e2b.dev/sandbox/persistence.md,
  https://e2b.dev/security
- **Private network off-limits:** Unknown: not documented in the pages checked
- **Built-in stealth:** Unknown: not documented in the pages checked
- **Live sound:** Not documented — https://docs.e2b.dev/template/examples/desktop
- **Drag a file straight onto the page:** Not documented — https://raw.githubusercontent.com/e2b-dev/desktop/main/README.md
- **Your own file picker for uploads:** Not documented — https://raw.githubusercontent.com/e2b-dev/desktop/main/README.md
- **Downloads land on your computer:** Not documented — https://raw.githubusercontent.com/e2b-dev/desktop/main/README.md
- **Copy and paste both ways:** Unknown: not checked
- **Record the session:** Unknown: not documented in the pages checked
- **Turn off input, clipboard, files or sound per session:** Partial: a view-only flag on the stream URL; no other switch documented — https://raw.githubusercontent.com/e2b-dev/desktop/main/README.md , https://raw.githubusercontent.com/e2b-dev/E2B/main/packages/desktop-js/src/sandbox.ts
  (`stream.getUrl({ viewOnly: true })` adds noVNC's `view_only` query parameter; that the server does not enforce it is a reading of the source.)
- **Your own proxy per session, location matched:** Unknown: not in their docs — https://github.com/e2b-dev/desktop
  (The README documents no proxy, time zone or locale option; the stream options are requireAuth, viewOnly and windowId only.)
- **Works over one HTTPS port, no TURN:** Unknown: not in their docs — https://github.com/e2b-dev/desktop
  (The README does not describe the stream transport or ports; "only one stream at a time".)
- **Open source you can audit:** Yes: Apache-2.0 — https://github.com/e2b-dev/desktop , https://e2b.dev/open-source

### Hyperbeam

- **Runs on your own server:** Unknown: no self-hosted offering found — https://hyperbeam.com
- **No usage fees:** No: 10,000 free minutes per month, then $0.007 per participant-minute — https://hyperbeam.com
- **No plan limit on browsers at once:** Unknown: not in their docs — https://docs.hyperbeam.com/home/faq , https://docs.hyperbeam.com/rest-api/dispatch/start-chromium-session.md , https://hyperbeam.com
- **No plan limit on session length:** Yes: no maximum; offline, inactive and absolute timeouts you set per session — https://docs.hyperbeam.com/guides/timeouts.md
  (Offline defaults to 3600 seconds; inactive and absolute default to none.)
- **Live view:** WebRTC, 24–60 fps, up to 1080p — https://docs.hyperbeam.com/rest-api/dispatch/start-chromium-session.md
- **Take over, and the agent waits:** Not applicable: co-browsing only; control passes by roles and control priority — https://docs.hyperbeam.com/guides/roles.md, https://docs.hyperbeam.com/client-sdk/javascript/reference.md
- **Build your own takeover screen:** Yes: the `@hyperbeam/web` SDK renders the session into your own element — https://docs.hyperbeam.com/client-sdk/javascript/overview.md , https://docs.hyperbeam.com/client-sdk/javascript/reference.md
- **An API your agents can call:** Partial: a REST API starts sessions; input only through the in-page JavaScript SDK — https://docs.hyperbeam.com/rest-api/dispatch/start-chromium-session.md , https://docs.hyperbeam.com/client-sdk/javascript/reference.md
  (Starting a session returns `embed_url` and `admin_token`, with no CDP or automation endpoint; input goes through `hb.sendEvent` and `hb.tabs` in the page.)
- **An MCP server for your agents:** Unknown: not in their docs — https://docs.hyperbeam.com/llms.txt
- **Your typing hidden from the agent:** Not applicable: co-browsing, no agent — https://hyperbeam.com
- **Encrypted session vaults only your password unlocks:** Not documented: "we encrypt and store the Chrome profile of the session in Amazon S3", expiring after three months; the
  algorithm and who holds the key are not documented — https://docs.hyperbeam.com/guides/persist-session-state.md
- **Private network off-limits:** Partial: an HTTP-only PAC proxy; WebRTC traffic bypasses it — https://docs.hyperbeam.com/home/faq.md
- **Built-in stealth:** Unknown: not documented in the pages checked
- **Live sound:** Yes: audio out as a MediaStreamTrack — https://docs.hyperbeam.com/client-sdk/javascript/reference.md, https://docs.hyperbeam.com/home/features.md
- **Drag a file straight onto the page:** Not documented: `file_upload` and `file_download` roles exist; the mechanism is not documented — https://docs.hyperbeam.com/guides/roles.md
- **Your own file picker for uploads:** Not documented: a `file_upload` role (off by default) "Allows the user to upload files from their local file system"; whether the page's own chooser opens the local picker is not documented; its `@hyperbeam/web` npm package (0.0.39) loads its client code from the session server at runtime, so the package shows no file handling — https://docs.hyperbeam.com/guides/roles.md, https://registry.npmjs.org/@hyperbeam/web
- **Downloads land on your computer:** Not documented: a `file_download` role (off by default) "Allows the user to download files to their local file system"; mechanism, progress and size not documented — https://docs.hyperbeam.com/guides/roles.md
- **Copy and paste both ways:** Partial: copy out by the `clipboard_copy` role (off by default); paste in not documented — https://docs.hyperbeam.com/guides/roles.md, https://docs.hyperbeam.com/client-sdk/javascript/reference.md
- **Record the session:** Unknown: not documented in the pages checked
- **Turn off input, clipboard, files or sound per session:** Yes: roles per session and per user: `control`, `clipboard_copy`, `file_upload`, `file_download` and more — https://docs.hyperbeam.com/guides/roles.md , https://docs.hyperbeam.com/rest-api/dispatch/start-chromium-session.md
  (Audio is a client-side volume; no recording option documented.)
- **Your own proxy per session, location matched:** Partial: an HTTP-only PAC proxy; WebRTC traffic bypasses it — https://docs.hyperbeam.com/home/faq.md
  (FAQ: an `http_proxy` object with a PAC file, user name and password; "Only HTTP traffic is sent through the proxy server. WebRTC traffic is excluded." Whether it is per session is not stated; time zone and locale are not covered.)
- **Works over one HTTPS port, no TURN:** Not applicable: cloud only (the FAQ says WebRTC is used for streaming) — https://docs.hyperbeam.com/home/faq.md
- **Open source you can audit:** No: closed cloud service — https://hyperbeam.com

### neko

- **Runs on your own server:** Yes: Apache-2.0 — https://github.com/m1k1o/neko
- **No usage fees:** Yes: free software — https://github.com/m1k1o/neko
- **No plan limit on browsers at once:** Yes: free software; one browser per server, more with neko-rooms — https://github.com/m1k1o/neko
- **No plan limit on session length:** Yes: free software; no session limit in their docs (the login cookie lasts 24 hours by default, configurable) — https://neko.m1k1o.net/docs/v3/configuration/authentication
- **Live view:** WebRTC; JPEG screencast fallback, off by default (the WebSocket carries control) — https://neko.m1k1o.net/docs/v3/configuration/capture, https://github.com/m1k1o/neko/releases
- **Take over, and the agent waits:** Not applicable: no agent; a host role and per-member permissions pass control between people — https://neko.m1k1o.net/docs/v3/configuration/authentication
- **Build your own takeover screen:** Yes: `@m1k1o/neko`, a framework-free client library — https://raw.githubusercontent.com/m1k1o/neko/master/client/core/README.md
- **An API your agents can call:** Partial: REST API for control, screenshots, clipboard and uploads; mouse and keys go over its WebSocket — https://neko.m1k1o.net/docs/v3/api
  (Sessions, room control, screen and screenshot, clipboard, keyboard map and upload; no mouse or key input endpoint.)
- **An MCP server for your agents:** Not applicable: no agent. No MCP server in its docs or https://github.com/m1k1o/neko README; a community "neko-mcp" only: https://glama.ai/mcp/servers/annnfs7a5b
- **Your typing hidden from the agent:** Not applicable: no agent — https://github.com/m1k1o/neko
- **Encrypted session vaults only your password unlocks:** Not documented: logins persist in a profile folder you mount on your host once the cookie-clearing policy is changed;
  encryption is not documented — https://neko.m1k1o.net/docs/v3/customization/browsers
- **Private network off-limits:** Not offered — https://neko.m1k1o.net/docs/v3/configuration
- **Built-in stealth:** Not a product feature — https://github.com/m1k1o/neko
- **Live sound:** Yes: Opus; microphone — https://neko.m1k1o.net/docs/v3/configuration/capture, https://github.com/m1k1o/neko/releases/tag/v3.2.0
- **Drag a file straight onto the page:** Yes: a drop lands at the cursor (coordinates plus a replayed XDND drop), on by default; folders skipped — https://neko.m1k1o.net/docs/v3/configuration/desktop, https://raw.githubusercontent.com/m1k1o/neko/master/server/pkg/drop/drop.c, https://raw.githubusercontent.com/m1k1o/neko/master/client/core/src/overlay.ts, https://raw.githubusercontent.com/m1k1o/neko/master/server/internal/api/room/upload.go
- **Your own file picker for uploads:** Partial: `file_chooser_dialog` is experimental and off by default, and the bundled client has no handler for it; the remote dialog otherwise — https://neko.m1k1o.net/docs/v3/configuration/desktop, https://github.com/m1k1o/neko, https://raw.githubusercontent.com/m1k1o/neko/master/server/internal/config/desktop.go, https://raw.githubusercontent.com/m1k1o/neko/master/server/internal/desktop/filechooserdialog.go
- **Downloads land on your computer:** Partial: from the File Transfer plugin's list (off by default; by default only admins may download) — https://neko.m1k1o.net/docs/v3/configuration/plugins, https://raw.githubusercontent.com/m1k1o/neko/master/client/src/plugins/filetransfer/Files.tsx, https://raw.githubusercontent.com/m1k1o/neko/master/server/internal/plugins/filetransfer/config.go
- **Copy and paste both ways:** Yes: text, automatic in Chromium; a textarea fallback in Firefox and Safari — https://raw.githubusercontent.com/m1k1o/neko/master/client/src/features/video/useClipboardSync.ts, https://raw.githubusercontent.com/m1k1o/neko/master/client/core/src/client.ts, https://raw.githubusercontent.com/m1k1o/neko/master/server/internal/desktop/clipboard.go
- **Record the session:** Partial: by saving the RTMP broadcast — https://github.com/m1k1o/neko, https://neko.m1k1o.net/docs/v3/configuration/capture
- **Turn off input, clipboard, files or sound per session:** Yes: per user and per room: control (`can_host`), clipboard (`can_access_clipboard`), file transfer (`filetransfer.enabled`), watching (`can_watch`), locked controls — https://neko.m1k1o.net/docs/v3/configuration/authentication , https://neko.m1k1o.net/docs/v3/configuration/plugins , https://neko.m1k1o.net/docs/v3/configuration
- **Your own proxy per session, location matched:** Unknown: not in their docs — https://github.com/m1k1o/neko
  (The README names no proxy, time zone or locale setting. Chromium flags can be passed; unverified whether that is documented for a proxy.)
- **Works over one HTTPS port, no TURN:** No: WebRTC needs a UDP port range or one mux port, with a TURN fallback — https://neko.m1k1o.net/docs/v3/configuration/webrtc
  (It opens a UDP range (example 59000-59100) or one UDP/TCP mux port (`udpmux`/`tcpmux`, example 59000) that must not be remapped; TURN is needed when a direct path fails, example coturn ports 3478/tcp and 49160-49200/udp.)
- **Open source you can audit:** Yes: Apache-2.0 — https://github.com/m1k1o/neko

### Selkies / LinuxServer

- **Runs on your own server:** Yes: Selkies is MPL-2.0; the LinuxServer images are GPL-3.0 — https://github.com/selkies-project/selkies, https://github.com/linuxserver/docker-webtop
- **No usage fees:** Yes: free software — https://github.com/selkies-project/selkies
- **No plan limit on browsers at once:** Yes: free software; one desktop per container — https://docs.linuxserver.io/selkies/
- **No plan limit on session length:** Yes: free software; no limit in their docs — https://docs.selkies.io/latest/settings , https://docs.linuxserver.io/selkies/user-guide/configuration/
- **Live view:** WebSocket with WebCodecs decode by default; WebRTC optional — https://github.com/selkies-project/selkies/releases
- **Take over, and the agent waits:** Not applicable: no agent; controller, viewer and collaborator roles in secure mode (`mk_control`) — https://docs.selkies.io/latest/secure-mode
- **Build your own takeover screen:** Yes: selkies-web-core "is embeddable in any HTML5 page and is what a custom interface builds on" — https://docs.selkies.io/latest/components/web-client
- **An API your agents can call:** Partial: HTTP API for tokens, recording, screenshots and files; input only over the stream — https://docs.selkies.io/latest/secure-mode
  (`/api/tokens`, `/api/recording`, `/api/sessions`, `/api/screenshot`, `/api/upload`.)
- **An MCP server for your agents:** Not applicable: no agent. No MCP server in https://docs.selkies.io/latest/ or the https://github.com/selkies-project/selkies README
- **Your typing hidden from the agent:** Not applicable: no agent — https://github.com/selkies-project/selkies
- **Encrypted session vaults only your password unlocks:** Not documented: the home volume you mount (`/config`, `/home/ubuntu`) keeps the browser profile; its encryption is
  not documented (SealSkin documents encryption for shared files, not home directories) — https://docs.linuxserver.io/selkies/user-guide/installation/,
  https://docs.selkies.io/latest/components/base-image, https://docs.linuxserver.io/selkies/components/sealskin/
- **Private network off-limits:** Not offered — https://docs.linuxserver.io/selkies/user-guide/configuration/
- **Built-in stealth:** Not a product feature — https://github.com/selkies-project/selkies
- **Live sound:** Yes: Opus; microphone opt-in — https://docs.selkies.io/latest/settings, https://docs.linuxserver.io/selkies/user-guide/web-client/
- **Drag a file straight onto the page:** Partial: dropped files upload into a folder, with no position — https://docs.linuxserver.io/selkies/user-guide/web-client/, https://raw.githubusercontent.com/selkies-project/selkies/main/addons/selkies-web-core/lib/file-upload.js
- **Your own file picker for uploads:** Partial: upload to a remote folder first (`~/Desktop`, `SELKIES_FILE_MANAGER_PATH`), then the remote dialog (inferred) — https://docs.linuxserver.io/selkies/user-guide/web-client/, https://docs.selkies.io/latest/settings, https://github.com/linuxserver/docker-baseimage-selkies, https://github.com/linuxserver/docker-chromium
- **Downloads land on your computer:** Partial: from the Files panel, folders as zip; only printed PDFs go to the local print dialog — https://docs.linuxserver.io/selkies/user-guide/web-client/, https://raw.githubusercontent.com/selkies-project/selkies/main/addons/selkies-dashboard-wish/src/components/dashboard/file-manager.tsx, https://raw.githubusercontent.com/selkies-project/selkies/main/addons/selkies-web-core/lib/print-jobs.js
- **Copy and paste both ways:** Yes: text, rich text and images — https://docs.linuxserver.io/selkies/user-guide/web-client/, https://raw.githubusercontent.com/selkies-project/selkies/main/addons/selkies-web-core/lib/clipboard-sync.js
- **Record the session:** Yes: recording API (`/api/recording`) — https://docs.selkies.io/latest/secure-mode, https://docs.linuxserver.io/selkies/user-guide/configuration/
- **Turn off input, clipboard, files or sound per session:** Yes: per container: `SELKIES_ENABLE_CLIPBOARD` (both, in, out, off), `SELKIES_FILE_TRANSFERS`, `SELKIES_AUDIO_ENABLED`, `SELKIES_MICROPHONE_ENABLED`; viewer roles refused input and files — https://docs.selkies.io/latest/settings , https://docs.selkies.io/latest/secure-mode
- **Your own proxy per session, location matched:** Unknown: not in their docs — https://github.com/linuxserver/docker-webtop
  (The README covers ports 3000/3001 and no per-session proxy, time zone or locale feature.)
- **Works over one HTTPS port, no TURN:** Yes: WebSocket by default on 3001; WebRTC (opt-in) needs UDP — https://docs.linuxserver.io/selkies/user-guide/webrtc/ , https://docs.selkies.io/latest/
  (By default it "streams over plain WebSockets" through web port 3000/3001 (HTTPS required for WebCodecs). WebRTC is opt-in: a UDP mux port (e.g. 59000) or the range 49152-65535, an optional TCP mux, and STUN/TURN for relay.)
- **Open source you can audit:** Yes: MPL-2.0 (Selkies), GPL-3.0 (LinuxServer images) — https://github.com/selkies-project/selkies , https://github.com/linuxserver/docker-webtop

### KasmVNC

- **Runs on your own server:** Yes: GPL-2.0 — https://github.com/kasmtech/KasmVNC
- **No usage fees:** Yes: free software — https://github.com/kasmtech/KasmVNC
- **No plan limit on browsers at once:** Yes: free software; one desktop per server — https://kasmweb.com/kasmvnc/docs/latest/configuration.html
- **No plan limit on session length:** Yes: idle and session timeouts you set, `never` by default — https://kasmweb.com/kasmvnc/docs/latest/configuration.html
- **Live view:** WebSocket; video mode since 1.5.0 — https://github.com/kasmtech/KasmVNC, https://github.com/kasmtech/KasmVNC/releases
- **Take over, and the agent waits:** Not applicable: no agent; `shared` or `exclusive` sessions between people — https://kasmweb.com/kasmvnc/docs/latest/configuration.html
- **Build your own takeover screen:** Unknown: its web client is a noVNC fork, with no viewer library in their docs — https://github.com/kasmtech/noVNC
- **An API your agents can call:** Partial: HTTP API for screenshots, users and stats; no input or session start — https://kasm.com/kasmvnc/docs/latest/developer_api.html
  (`/api/get_screenshot`, `/api/create_user`, `/api/downloads` and more.)
- **An MCP server for your agents:** Not applicable: no agent. No MCP server in its docs or the https://github.com/kasmtech/KasmVNC README
- **Your typing hidden from the agent:** Not applicable: no agent — https://github.com/kasmtech/KasmVNC
- **Encrypted session vaults only your password unlocks:** Not applicable: a display server with no saved-login concept — https://github.com/kasmtech/KasmVNC
- **Private network off-limits:** Not offered — https://kasmweb.com/kasmvnc/docs/latest/configuration.html
- **Built-in stealth:** Not a product feature — https://github.com/kasmtech/KasmVNC
- **Live sound:** No: audio and microphone are "only available when using Kasm Workspaces" — https://github.com/kasmtech/KasmVNC/issues/339
- **Drag a file straight onto the page:** No: "no action associated with dragging and dropping files on kasmvnc webpage" — https://github.com/kasmtech/noVNC/issues/109
- **Your own file picker for uploads:** No: the remote dialog only; "KasmVNC itself has no file upload/download feature at all" — https://github.com/kasmtech/KasmVNC/issues/250
- **Downloads land on your computer:** No: no client download; the server only exposes a `~/Downloads` listing API — https://kasm.com/kasmvnc/docs/latest/developer_api.html, https://github.com/kasmtech/KasmVNC/blob/master/common/network/websocket.c
- **Copy and paste both ways:** Yes: text and images in Chromium-based browsers; a manual panel elsewhere — https://github.com/kasmtech/KasmVNC/blob/master/README.md, https://kasm.com/kasmvnc/docs/latest/configuration.html
- **Record the session:** Not offered — https://github.com/kasmtech/KasmVNC
- **Turn off input, clipboard, files or sound per session:** Yes: per server: clipboard each way, keyboard and pointer (`data_loss_prevention`, `pointer.enabled`); view-only users — https://kasmweb.com/kasmvnc/docs/latest/configuration.html , https://github.com/kasmtech/KasmVNC
  (It has no file transfer or sound to turn off.)
- **Your own proxy per session, location matched:** Unknown: not in their docs — https://github.com/kasmtech/KasmVNC
  (A VNC server and web client, with no browser proxy or locale feature.)
- **Works over one HTTPS port, no TURN:** Yes: HTTPS/WebSocket on 8443; optional WebRTC UDP — https://kasmweb.com/kasmvnc/docs/1.3.3/configuration.html , https://github.com/kasmtech/KasmVNC
  (`websocket_port` defaults to 8443 plus the display number. The configuration also has a `udp` section (port auto, stun_server auto) for "WebRTC UDP Transit"; whether it is off by default was not found, hence "optional".)
- **Open source you can audit:** Yes: GPL-2.0 — https://github.com/kasmtech/KasmVNC

### Kasm Workspaces

- **Runs on your own server:** Yes: proprietary, with a free Community Edition (5 concurrent sessions, non-commercial) — https://trust.kasmweb.com/resources/features-matrix
- **No usage fees:** Yes, in the free Community Edition (5 sessions, non-commercial); Starter "$10/User or $20/Session"; Enterprise by quote — https://trust.kasmweb.com/resources/features-matrix
- **No plan limit on browsers at once:** No: the Community Edition allows 5 concurrent sessions, non-commercial; paid licenses count concurrent sessions or named users — https://trust.kasmweb.com/resources/features-matrix , https://docs.kasm.com/docs/latest/reference/license
- **No plan limit on session length:** Yes: timeouts the admin sets per group; the hard `session_time_limit` is off by default — https://docs.kasm.com/docs/latest/how-to/security/users-groups-mgmt/groups/group-settings
  (`keepalive_expiration` 1 hour and `idle_disconnect` 20 minutes by default in the All Users group, both changeable.)
- **Live view:** KasmVNC — https://kasm.com/docs/latest/_sources/user_guide/control_panel.md.txt
- **Take over, and the agent waits:** Not applicable: no agent; share URLs are view-only by default, or full control for everyone — https://docs.kasm.com/docs/guide/session_sharing/index.html
- **Build your own takeover screen:** Partial: a developer API and iframe embedding; no viewer library — https://docs.kasm.com/docs/latest/reference/developer-api , https://docs.kasm.com/docs/latest/how-to/workspaces-sessions/container-workspace/embed-kasm-in-iframe
- **An API your agents can call:** Partial: a developer API starts sessions, runs commands and takes screenshots; no mouse or key input — https://kasm.com/docs/latest/developers/developer_api.html
  (`request_kasm`, `exec_command_kasm`, `get_kasm_screenshot`, `destroy_kasm`.)
- **An MCP server for your agents:** Not applicable: no agent. No MCP server in their docs; a community one only: https://glama.ai/mcp/servers/roguedev-ai/kasm-mcp-server-v2
- **Your typing hidden from the agent:** Not applicable: no agent — https://kasm.com/solutions/platform
- **Encrypted session vaults only your password unlocks:** No: persistent profiles live on your volume or S3, with no encryption documented; Profile Sync, a Preview
  "not currently recommended for production use", gives each user's profile "a different encryption key managed by
  Kasm" — https://docs.kasm.com/docs/how-to/data-storage/persistent-profiles/, https://kasmweb.com/docs/1.12.0/guide/persistent_data.html
- **Private network off-limits:** Partial: VPN egress gateways (OpenVPN, WireGuard, managed PureVPN) — https://kasm.com/docs/latest/guide/egress.html
- **Built-in stealth:** Not a product feature — https://kasm.com/solutions/platform
- **Live sound:** Yes: sound, plus microphone and webcam pass-through — https://docs.kasm.com/docs/tutorials/user-guide/peripheral-support
- **Drag a file straight onto the page:** Partial: not in the docs; Kasm staff say it "already supports drag and drop for file uploads" (destination unverified, presumably `~/Uploads`); no positioned drop documented — https://github.com/kasmtech/KasmVNC/issues/250
- **Your own file picker for uploads:** Partial: upload into `~/Uploads` from the control panel first, then the remote dialog (inferred) — https://docs.kasm.com/docs/tutorials/user-guide/control-panel, https://github.com/kasmtech/workspaces-core-images/blob/develop/src/common/startup_scripts/vnc_startup.sh
- **Downloads land on your computer:** Partial: from a list of the remote `~/Downloads` folder — https://docs.kasm.com/docs/tutorials/user-guide/control-panel, https://docs.kasm.com/docs/reference/release-notes/1.16.0, https://docs.kasm.com/docs/reference/release-notes/1.16.1, https://docs.kasm.com/docs/reference/release-notes/1.19.0
- **Copy and paste both ways:** Yes: automatic text in Chromium-based browsers; the control panel elsewhere — https://docs.kasm.com/docs/tutorials/user-guide/control-panel, https://docs.kasm.com/docs/how-to/security/users-groups-mgmt/groups
- **Record the session:** Partial: Enterprise only — https://trust.kasmweb.com/resources/features-matrix, https://kasmweb.com/docs/latest/guide/session_recording.html
- **Turn off input, clipboard, files or sound per session:** Partial: clipboard, uploads, downloads, audio, microphone per user group, not per session — https://docs.kasm.com/docs/latest/how-to/security/users-groups-mgmt/groups/group-settings
  (`allow_kasm_clipboard_down`, `allow_kasm_clipboard_up`, `allow_kasm_uploads`, `allow_kasm_downloads`, `allow_kasm_audio`, `allow_kasm_microphone`; recording by the Enterprise `record_sessions` group setting; view-only only for joiners of a shared session.)
- **Your own proxy per session, location matched:** Partial: egress gateways per workspace, user or group; not per session — https://docs.kasm.com/docs/guide/egress/index.html
  (Egress providers are OpenVPN, WireGuard or Tailscale gateways mapped to a Workspace, User or Group and selectable on launch. No HTTP proxy per session and no time zone or locale matching found.)
- **Works over one HTTPS port, no TURN:** Partial: 443 for users; KasmVNC on 8443 between servers — https://docs.kasm.com/docs/install/multi_server_install/index.html
  (Multi-server doc: 443 from the end user to the web app; 8443 from the web app to the VM. The pages read do not mention WebRTC or UDP for Kasm sessions, so "no UDP" is unverified.)
- **Open source you can audit:** Partial: proprietary, with a free Community Edition — https://trust.kasmweb.com/resources/features-matrix
  (Several Kasm components, such as KasmVNC and the workspace images, are open source; the platform is not.)

### Apache Guacamole

- **Runs on your own server:** Yes: Apache-2.0 — https://guacamole.apache.org/
- **No usage fees:** Yes: free software — https://guacamole.apache.org/
- **No plan limit on browsers at once:** Yes: unlimited unless the admin sets a limit — https://guacamole.apache.org/doc/gug/mysql-auth.html
  ("By default, overall concurrent access to Guacamole is not limited.")
- **No plan limit on session length:** Yes: only an idle login timeout, 60 minutes by default and configurable (`api-session-timeout`) — https://guacamole.apache.org/doc/gug/configuring-guacamole.html
- **Live view:** Guacamole protocol over WebSocket — https://guacamole.apache.org/
- **Take over, and the agent waits:** Not applicable: no agent; shared connections through sharing profiles — https://guacamole.apache.org/doc/gug/administration.html
- **Build your own takeover screen:** Yes: guacamole-common-js, a JavaScript client library — https://guacamole.apache.org/doc/gug/guacamole-common-js.html
- **An API your agents can call:** Partial: a REST API for users, connections and active sessions, in its source but not its manual — https://github.com/apache/guacamole-client/tree/main/guacamole/src/main/java/org/apache/guacamole/rest , https://guacamole.apache.org/doc/gug/ , https://github.com/apache/guacamole-manual
  (Packages `auth`, `connection`, `activeconnection`, `tunnel`, `user` and more. The manual has no REST API section: a search of its source found none. It manages connections; it does not send input.)
- **An MCP server for your agents:** Not applicable: no agent. No MCP server in the manual or repository
- **Your typing hidden from the agent:** Not applicable: no agent — https://guacamole.apache.org/
- **Encrypted session vaults only your password unlocks:** Not applicable: a remote desktop gateway with no saved-login concept — https://guacamole.apache.org/doc/gug/introduction.html
- **Private network off-limits:** Not offered — https://guacamole.apache.org/doc/gug/configuring-guacamole.html
- **Built-in stealth:** Not a product feature — https://guacamole.apache.org/
- **Live sound:** Yes: RDP audio; VNC audio needs PulseAudio — https://guacamole.apache.org/doc/gug/configuring-guacamole.html
- **Drag a file straight onto the page:** Partial: dropped files upload into the SFTP directory or the RDP drive — https://guacamole.apache.org/doc/gug/using-guacamole.html
- **Your own file picker for uploads:** Partial: upload into the SFTP folder or RDP drive first, then the remote dialog (inferred) — https://guacamole.apache.org/doc/gug/using-guacamole.html
- **Downloads land on your computer:** Partial: from a file browser; files saved in the RDP drive's "Download" folder push to the browser — https://guacamole.apache.org/doc/gug/using-guacamole.html
- **Copy and paste both ways:** Yes: "bidirectional access to the clipboard by default"; the docs speak only of text — https://guacamole.apache.org/doc/gug/using-guacamole.html, https://guacamole.apache.org/doc/gug/configuring-guacamole.html, https://raw.githubusercontent.com/apache/guacamole-client/1.6.0/guacamole/src/main/frontend/src/app/clipboard/services/clipboardService.js
- **Record the session:** Yes: with an in-browser player and key-event capture — https://guacamole.apache.org/releases/1.6.0/, https://guacamole.apache.org/doc/gug/configuring-guacamole.html
- **Turn off input, clipboard, files or sound per session:** Yes: per connection: `read-only`, `disable-copy`, `disable-paste`, `disable-upload`, `disable-download`, audio and recording parameters — https://guacamole.apache.org/doc/gug/configuring-guacamole.html
- **Your own proxy per session, location matched:** Unknown: not in their docs — https://guacamole.apache.org/doc/gug/guacamole-architecture.html
  (A remote-desktop gateway, not a browser; no proxy or locale feature on that page.)
- **Works over one HTTPS port, no TURN:** Yes: the browser uses HTTP(S), WebSocket or an HTTP tunnel — https://guacamole.apache.org/doc/gug/guacamole-architecture.html
  (The client "connects back to the server over HTTP", WebSocket preferred with an HTTP tunnel fallback. The page names no port; guacd is a separate internal TCP daemon behind the web app.)
- **Open source you can audit:** Yes: Apache-2.0 — https://guacamole.apache.org/

### xpra-html5

- **Runs on your own server:** Yes: xpra-html5 is MPL-2.0, xpra is GPL-2.0+ — https://github.com/Xpra-org/xpra-html5, https://github.com/Xpra-org/xpra
- **No usage fees:** Yes: free software — https://github.com/Xpra-org/xpra
- **No plan limit on browsers at once:** Yes: free software; one session per server — https://github.com/Xpra-org/xpra
- **No plan limit on session length:** Yes: `--idle-timeout` and `--server-idle-timeout` you set; 0 disables them — https://raw.githubusercontent.com/Xpra-org/xpra/master/fs/share/man/man1/xpra.1
- **Live view:** WebSocket; the server switches to video per window or region automatically — https://github.com/Xpra-org/xpra/blob/master/docs/Usage/Encodings.md
- **Take over, and the agent waits:** Not applicable: no agent; sharing control is not documented — https://github.com/Xpra-org/xpra/blob/master/docs/Features/README.md
- **Build your own takeover screen:** Unknown: the HTML5 client is an app; no library API in their docs — https://github.com/Xpra-org/xpra-html5
- **An API your agents can call:** No: no HTTP API; a command line, `xpra control` — https://github.com/Xpra-org/xpra/tree/master/docs
  (Only `xpra control :DISPLAY ...` command-line usage, for example in docs/Features/File-Transfers.md and docs/Usage/Logging.md.)
- **An MCP server for your agents:** Not applicable: no agent. No MCP server in its docs or README
- **Your typing hidden from the agent:** Not applicable: no agent — https://github.com/Xpra-org/xpra
- **Encrypted session vaults only your password unlocks:** Not applicable: a remote display client with no saved-login concept — https://github.com/Xpra-org/xpra-html5
- **Private network off-limits:** Not offered — https://github.com/Xpra-org/xpra
- **Built-in stealth:** Not a product feature — https://github.com/Xpra-org/xpra
- **Live sound:** Yes: speaker audio; no microphone in the HTML5 client — https://github.com/Xpra-org/xpra-html5/blob/master/docs/Configuration.md, https://github.com/Xpra-org/xpra/blob/master/docs/Features/Audio.md
- **Drag a file straight onto the page:** Partial: dropped files upload (optionally opened), never at a position — https://github.com/Xpra-org/xpra/blob/master/docs/Features/File-Transfers.md, https://raw.githubusercontent.com/Xpra-org/xpra-html5/master/html5/js/Client.js
- **Your own file picker for uploads:** Partial: "Upload file" saves into the server's download folder first, then the remote dialog (inferred) — https://github.com/Xpra-org/xpra/blob/master/docs/Features/File-Transfers.md, https://raw.githubusercontent.com/Xpra-org/xpra-html5/master/html5/index.html, https://raw.githubusercontent.com/Xpra-org/xpra/master/xpra/platform/posix/paths.py
- **Downloads land on your computer:** Partial: only files pushed with `xpra send-file`, which the client assembles whole in memory before saving — https://github.com/Xpra-org/xpra/blob/master/docs/Features/File-Transfers.md, https://github.com/Xpra-org/xpra-html5/blob/master/docs/CHANGELOG.md
- **Copy and paste both ways:** Partial: text, HTML and PNG, reached only "when the browser decides it is appropriate" — https://github.com/Xpra-org/xpra/blob/master/docs/Features/Clipboard.md, https://github.com/Xpra-org/xpra-html5/blob/master/docs/Configuration.md
- **Record the session:** Not documented — https://github.com/Xpra-org/xpra/blob/master/docs/Features/README.md
- **Turn off input, clipboard, files or sound per session:** Yes: start options `--readonly`, `--clipboard`/`--clipboard-direction`, `--file-transfer`, `--audio` — https://raw.githubusercontent.com/Xpra-org/xpra/master/fs/share/man/man1/xpra.1
- **Your own proxy per session, location matched:** Unknown: not in their docs — https://github.com/Xpra-org/xpra-html5
- **Works over one HTTPS port, no TURN:** Yes: one TCP port serves the page and the WebSocket — https://github.com/Xpra-org/xpra-html5
  (README: start an xpra session on a TCP port and "access this session with your browser by pointing it to that port"; `xpra --html=on` needs no websockify since v2.4, per https://www.xpra.org/trac/ticket/1045 (seen in a search result). No UDP or WebRTC in the README.)
- **Open source you can audit:** Yes: MPL-2.0 client, GPL-2.0+ xpra — https://github.com/Xpra-org/xpra-html5 , https://github.com/Xpra-org/xpra

### noVNC

- **Runs on your own server:** Yes: MPL-2.0 — https://github.com/novnc/noVNC
- **No usage fees:** Yes: free software — https://github.com/novnc/noVNC
- **No plan limit on browsers at once:** Yes: a client only; no limit of its own — https://github.com/novnc/noVNC
- **No plan limit on session length:** Yes: a client only; no limit of its own — https://github.com/novnc/noVNC
- **Live view:** VNC over WebSocket — https://github.com/novnc/noVNC
- **Take over, and the agent waits:** Not applicable: no agent; a view-only flag — https://github.com/novnc/noVNC
- **Build your own takeover screen:** Yes: "noVNC is both a HTML VNC client JavaScript library and an application built on top of that library" — https://github.com/novnc/noVNC
- **An API your agents can call:** No: a JavaScript client library; no HTTP API — https://github.com/novnc/noVNC/blob/master/docs/API.md
  (The `RFB` client object only.)
- **An MCP server for your agents:** Not applicable: no agent. No MCP server in its README; community VNC MCP servers such as https://glama.ai/mcp/servers/regulad/vnc-mcp are not noVNC projects
- **Your typing hidden from the agent:** Not applicable: no agent — https://github.com/novnc/noVNC
- **Encrypted session vaults only your password unlocks:** Not applicable: a VNC client with no saved-login concept — https://github.com/novnc/noVNC
- **Private network off-limits:** Not offered — https://github.com/novnc/noVNC
- **Built-in stealth:** Not a product feature — https://github.com/novnc/noVNC
- **Live sound:** No: the audio issue has been open since 2013 and the PR is unmerged — https://github.com/novnc/noVNC/issues/302, https://github.com/novnc/noVNC/pull/1952
- **Drag a file straight onto the page:** No: "noVNC does not have any file transfer capability" — https://github.com/novnc/noVNC/issues/169
- **Your own file picker for uploads:** No: no file transfer — https://github.com/novnc/noVNC/issues/169, https://github.com/novnc/noVNC/issues/1869
- **Downloads land on your computer:** No: no file transfer — https://github.com/novnc/noVNC/issues/169, https://github.com/novnc/noVNC/issues/1869
- **Copy and paste both ways:** Partial: text; automatic in Chromium since 1.7, a panel elsewhere — https://github.com/novnc/noVNC/pull/1993, https://github.com/novnc/noVNC/blob/master/core/clipboard.js
- **Record the session:** Not offered — https://github.com/novnc/noVNC
- **Turn off input, clipboard, files or sound per session:** Partial: `view_only`, set by the client page itself; no clipboard switch — https://github.com/novnc/noVNC/blob/master/docs/EMBEDDING.md , https://github.com/novnc/noVNC/blob/master/docs/API.md
- **Your own proxy per session, location matched:** Unknown: not in their docs — https://github.com/novnc/noVNC
- **Works over one HTTPS port, no TURN:** Yes: WebSocket through one websockify port — https://github.com/novnc/noVNC
  (README: it needs WebSockets or a websockify proxy; `novnc_proxy` serves the page and proxies on one address (`--listen localhost:6081`); no UDP or WebRTC.)
- **Open source you can audit:** Yes: MPL-2.0 — https://github.com/novnc/noVNC

### Browsers that run where the agent or the person is

- **Playwright MCP:** reads the accessibility tree (coordinates only with `--caps vision`); attaches to a cloud browser with `--cdp-endpoint` or to your own Chrome with `--extension`; no live view — https://github.com/microsoft/playwright-mcp, https://www.npmjs.com/package/@playwright/mcp
- **Chrome DevTools MCP:** Google's Apache-2.0 MCP server for Chrome DevTools — https://github.com/ChromeDevTools/chrome-devtools-mcp
- **Claude in Chrome:** runs in your own Chrome; no remote or cloud browser and no takeover model — https://support.claude.com/en/articles/12012173-getting-started-with-claude-in-chrome
- **ChatGPT's cloud browser:** runs "on a separate computer in the cloud"; you can take over its computer; sign-in goes through a secure form so the model never sees the credentials — https://learn.chatgpt.com/docs/browser, https://help.openai.com/en/articles/20001280-using-cloud-browser

### Researched but not in the table

Airtop, shutting down 2026-11-13 (https://www.airtop.ai/); Scrapybara, VMs shut down 2025-10-15
(https://x.com/scrapybara/status/1971655785869726110); Apify (https://apify.com/pricing); ZenRows
(https://docs.zenrows.com/scraping-browser/introduction); ScrapingBee (https://www.scrapingbee.com/documentation/);
Notte (https://github.com/nottelabs/notte); Skyvern (https://github.com/Skyvern-AI/skyvern); Rebrowser
(https://rebrowser.net/); Multilogin
(https://multiloginx.helpjuice.com/getting-started-with-multilogin-x-automation/automation-faq); Lightpanda
(https://github.com/lightpanda-io/browser); jlesage/docker-firefox, a Firefox behind noVNC
whose uploads and downloads go through an optional web file manager (off by default; Firefox saves to
`/config/downloads`), with no drop handling, text clipboard sync in Chromium-based browsers and audio only with
`WEB_AUDIO` (off by default) (https://github.com/jlesage/docker-firefox/blob/master/README.md, https://github.com/jlesage/docker-firefox/blob/master/rootfs/usr/lib/firefox/browser/defaults/preferences/firefox-branding.js, https://github.com/jlesage/docker-baseimage-gui/blob/master/README.md, https://github.com/jlesage/docker-baseimage-gui/blob/master/rootfs/opt/noVNC/app/fileManager.js, https://github.com/jlesage/docker-baseimage-gui/blob/master/src/novnc/auto-clipboard-sync.patch).

### Price range

The cloud agent browsers in the table list $0.02 to about $0.48 per standard browser-hour (Bright Data bills only per
GB). The low end is Browser Use Cloud ($0.02/h, https://browser-use.com/pricing) and the high end is Kernel's headful
browser (about $0.48/h, https://kernel.sh/docs/info/pricing.md); Kernel's GPU browser, about $2.88/h on the same
page, is not a standard browser. Bright Data charges nothing for browser time, only $8/GB
(https://brightdata.com/pricing/scraping-browser). Apify, researched but not in the table, comes to about $0.52–0.80/h
for a 4 GB run (calc, https://apify.com/pricing).

### Enterprise isolation, not in the table

Cloudflare Browser Isolation (https://developers.cloudflare.com/cloudflare-one/remote-browser-isolation/), Menlo,
Zscaler Cloud Browser Isolation (https://www.zscaler.com/resources/solution-briefs/cloud-browser-isolation-protecting-data.pdf),
Netskope RBI (https://docs.netskope.com/en/introduction-to-remote-browser-isolation-rbi), and the enterprise browsers
Island (https://www.island.io/product) and Prisma Access Browser secure an organisation's staff. None of these
publishes a list price, except that Prisma Access Browser is free to Prisma SASE AI customers
(https://www.darkreading.com/cloud-security/palo-alto-networks-closes-talon-cybersecurity-acquisition).

## Open points

- Self-hosted ports of Browserless, Steel and Kernel: not checked, shown as unknown.
- Hyperbrowser's proxy page was fetched first-hand and says Enterprise only; its SDK `locales` field comes from a
  search result snippet of the Python SDK reference.
- Kasm Workspaces and UDP: the absence of UDP is not verified.
- Browserbase locale and languages: not documented on the pages read.
- Clipboard in the live views of Hyperbrowser, Browser Use Cloud, Cloudflare Browser Run, AWS AgentCore Browser,
  Bright Data and E2B Desktop: not checked.
- Limits and switches: Anchor's default session maximum is 180 minutes in its guide and 20 in its API reference;
  Browserless's pricing page shows "+5", "+10" and "+20" beside its concurrency numbers without explaining them;
  E2B's view-only being client-side is read from its SDK source; Kernel documents no maximum session length.
