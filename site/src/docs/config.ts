// Every setting the docs list, as the code reads it. `default`: null when required, '' when unset by default.

export interface Var {
  name: string;
  default: string | null;
  what: string;
  values?: string;
}

/** Variables the Compose files read from your shell or a `.env` file next to them. */
export const COMPOSE: Var[] = [
  { name: 'DOCKER_GID', default: '65534', what: 'The group of the Docker socket, given to the controller. Without it the controller cannot reach Docker.', values: 'Set it with DOCKER_GID=$(stat -c %g /var/run/docker.sock).' },
  { name: 'ADMIN_PORT', default: '9443', what: 'The host loopback port Compose publishes the admin on (inside its container it listens on 9443); also sets ADMIN_ORIGIN to https://localhost:<port>.' },
  { name: 'ADMIN_TIMEZONE', default: 'UTC', what: 'Passed to the admin as ADMIN_TIMEZONE.' },
  { name: 'ADMIN_LANGUAGES', default: 'en-US,en', what: 'Passed to the admin as ADMIN_LANGUAGES.' },
  { name: 'ADMIN_LOCAL_MCP', default: '1', what: 'Passed to the admin as ADMIN_LOCAL_MCP.' },
  { name: 'ADMIN_MAX_SESSIONS', default: '1', what: 'Passed to the admin as ADMIN_MAX_SESSIONS, and to the controller as CONTROLLER_MAX_SESSIONS. The installer sets it from the server’s memory unless .env does: each session takes up to BROWSER_MEMORY_MIB plus 1152 MiB, and 1 GiB stays for the system.' },
  { name: 'BROWSER_CPUS', default: '2', what: 'Each browser session’s CPU limit, passed as CONTROLLER_BROWSER_CPUS.' },
  { name: 'BROWSER_MEMORY_MIB', default: '2048', what: 'Each browser session’s memory limit, passed as CONTROLLER_BROWSER_MEMORY_MIB.' },
];

/** The admin container's environment. */
export const ADMIN: Var[] = [
  { name: 'ADMIN_ORIGIN', default: null, what: 'The exact origin people open the admin at. Every state change and stream must come from it.', values: 'Starts with https://, no trailing slash.' },
  { name: 'ADMIN_LISTEN', default: '0.0.0.0:9443', what: 'The HTTPS address inside the container.' },
  { name: 'ADMIN_HOSTNAMES', default: 'localhost,127.0.0.1', what: 'Names on the self-signed certificate the admin makes; the next start remakes it when they change.', values: 'Comma-separated. To use your own certificate, put tls-cert.pem and tls-key.pem in ADMIN_STATE.' },
  { name: 'ADMIN_OUTPUT', default: '1280x800', what: 'The screen size of every session.', values: 'WxH, even, 320–1920 by 240–1080.' },
  { name: 'ADMIN_MAX_SESSIONS', default: '1', what: 'Browser sessions at once, yours and every agent token’s together.', values: '1–16.' },
  { name: 'ADMIN_TIMEZONE', default: 'UTC', what: 'The browsers’ time zone when a session uses no egress.', values: 'An IANA name, such as America/Sao_Paulo.' },
  { name: 'ADMIN_LANGUAGES', default: 'en-US,en', what: 'The browsers’ languages, preferred first, when a session uses no egress.', values: '1–8 tags such as pt-BR: a 2–3 letter language and up to two more subtags of 2–8 letters or digits.' },
  { name: 'ADMIN_LOCAL_MCP', default: '1', what: 'Serves MCP to local clients through admin mcp (see Agents). Unset in the public demo, it is off there.', values: '1 or 0; the demo refuses 1.' },
  { name: 'ADMIN_STATE', default: '/var/lib/admin', what: 'The password hash, the agent tokens’ hashes, and the TLS certificate and key.' },
  { name: 'ADMIN_STATIC', default: '/srv/admin', what: 'The web client the admin serves: mine, or your own (see Custom clients).' },
  { name: 'ADMIN_STATIC_PATH', default: '/', what: 'Where the web client is served. Under a path, the admin’s own /api, /ws, /sdk and /mcp stay at the root, and /name sends the browser on to /name/.', values: '/ or /name: 1–32 of a–z, 0–9, -; not /api, /ws, /sdk or /mcp.' },
  { name: 'ADMIN_SDK', default: '/srv/sdk', what: 'The viewer SDK, served at /sdk/ whatever ADMIN_STATIC holds.' },
  { name: 'ADMIN_STREAMS', default: '/run/streams', what: 'One directory per session, holding its display’s stream socket.' },
  { name: 'ADMIN_CONTROLLER', default: '/run/controller/controller.sock', what: 'The controller’s socket.' },
  { name: 'ADMIN_DEMO_SECONDS', default: '', what: 'Turns on the public demo: each visitor’s turn, in seconds (see Public demo).', values: '10–600, with ADMIN_DEMO_SITES.' },
  { name: 'ADMIN_DEMO_SITES', default: '', what: 'The demos a visitor may pick.', values: 'name=https://start/ pairs, comma-separated; names of 1–32 a–z letters, 1–16 sites.' },
  { name: 'ADMIN_FRAME_ANCESTORS', default: '', what: 'The pages that may show the public demo in a frame (the frame-ancestors of every response’s Content-Security-Policy); unset, nothing may frame the admin.', values: 'Up to four https://host[:port] origins, comma-separated. Only with ADMIN_DEMO_SECONDS.' },
  { name: 'RUST_LOG', default: 'info', what: 'The log filter.' },
];

/** The controller container's environment. */
export const CONTROLLER: Var[] = [
  { name: 'IMAGE_DISPLAY, IMAGE_BROWSER, IMAGE_AGENT, IMAGE_PROXY', default: null, what: 'The four session images.', values: 'Pinned by @sha256:, or a local name:tag resolved once at start (restart the controller after rebuilding one).' },
  { name: 'STREAMS_VOLUME', default: null, what: 'The Docker volume mounted into each session’s display; the admin sees it at ADMIN_STREAMS.' },
  { name: 'CONTROLLER_INSTANCE', default: 'easirun', what: 'This installation’s name: a prefix and label on every container, network and volume it creates. Cleanup touches only its own.', values: 'Up to 16 characters, a lowercase letter first, then a–z, 0–9, -.' },
  { name: 'CONTROLLER_BROWSER_CPUS', default: '2', what: 'Each browser’s CPU limit.', values: '0.5–16.' },
  { name: 'CONTROLLER_BROWSER_MEMORY_MIB', default: '2048', what: 'Each browser’s memory limit, swap included.', values: '1024–65536.' },
  { name: 'CONTROLLER_MAX_SESSIONS', default: '16', what: 'The most sessions the controller holds at once, whatever the admin asks.', values: '1–16. Keep it at or above ADMIN_MAX_SESSIONS.' },
  { name: 'CONTROLLER_SOCKET', default: '/run/controller/controller.sock', what: 'The socket it serves the admin on.' },
  { name: 'CONTROLLER_ADMIN_UID', default: '10004', what: 'The only uid allowed to connect.' },
  { name: 'CONTROLLER_BROWSER_SECCOMP', default: '/etc/controller/browser-seccomp.json', what: 'The browser’s seccomp profile.' },
  { name: 'RUST_LOG', default: 'info', what: 'The log filter.' },
];

/** The controller's walled-garden and benchmark settings (advanced: they change what a session may reach). */
export const BENCH: Var[] = [
  { name: 'CONTROLLER_BENCH_NETWORK', default: '', what: 'A Docker network the session proxies join, holding one extra destination.', values: 'All three BENCH variables together, or none.' },
  { name: 'CONTROLLER_BENCH_FIXTURE', default: '', what: 'That destination, reachable by name without DNS.', values: 'name=ipv4' },
  { name: 'CONTROLLER_BENCH_SPKI_FILE', default: '', what: 'A file with the base64 SHA-256 of the destination’s public key: Chrome accepts a certificate with that key even when no authority vouches for it.' },
  { name: 'CONTROLLER_BENCH_FIXTURE_ONLY', default: '', what: 'The walled garden: every address a browser asks for goes to that destination, and nothing else is reachable.', values: '1. Needs CONTROLLER_MAX_SESSION_SECONDS and an internal network.' },
  { name: 'CONTROLLER_MAX_SESSION_SECONDS', default: '', what: 'The walled garden’s age limit: stops any session older than this, whatever the admin does, and removes this installation’s leftovers at start.', values: '10–3600. Required with CONTROLLER_BENCH_FIXTURE_ONLY, refused without it.' },
  { name: 'CONTROLLER_BENCH_LANES', default: '', what: 'CPU sets for benchmark lanes, which a session’s lane picks.' },
];
