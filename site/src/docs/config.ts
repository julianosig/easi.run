// Every setting the docs list, as the code reads it. `default`: null when required, '' when unset by default. What each
// does, and the values it takes, are messages (config.<group>.<name>.what and .values).

export interface Var {
  name: string;
  default: string | null;
  /** Whether it has accepted values to say: `config.<group>.<name>.values`, beside `.what` (messages/en/config.json). */
  values?: true;
}

/** The groups, as the messages name them. */
export type Group = 'compose' | 'admin' | 'controller' | 'bench';

/** Variables the Compose files read from your shell or a `.env` file next to them. */
export const COMPOSE: Var[] = [
  { name: 'DOCKER_GID', default: '65534', values: true },
  { name: 'ADMIN_PORT', default: '9443' },
  { name: 'ADMIN_UDP_URL', default: '' },
  { name: 'ADMIN_UDP_BIND', default: '127.0.0.1' },
  { name: 'ADMIN_TIMEZONE', default: 'UTC' },
  { name: 'ADMIN_LANGUAGES', default: 'en-US,en' },
  { name: 'ADMIN_LOCAL_MCP', default: '1' },
  { name: 'ADMIN_MAX_SESSIONS', default: '1' },
  { name: 'BROWSER_CPUS', default: '2' },
  { name: 'BROWSER_MEMORY_MIB', default: '2048' },
];

/** The admin container's environment. */
export const ADMIN: Var[] = [
  { name: 'ADMIN_ORIGIN', default: null, values: true },
  { name: 'ADMIN_LISTEN', default: '0.0.0.0:9443' },
  { name: 'ADMIN_UDP_URL', default: '', values: true },
  { name: 'ADMIN_HOSTNAMES', default: 'localhost,127.0.0.1', values: true },
  { name: 'ADMIN_OUTPUT', default: '1280x800', values: true },
  { name: 'ADMIN_MAX_SESSIONS', default: '1', values: true },
  { name: 'ADMIN_TIMEZONE', default: 'UTC', values: true },
  { name: 'ADMIN_LANGUAGES', default: 'en-US,en', values: true },
  { name: 'ADMIN_LOCAL_MCP', default: '1', values: true },
  { name: 'ADMIN_STATE', default: '/var/lib/admin' },
  { name: 'ADMIN_STATIC', default: '/srv/admin' },
  { name: 'ADMIN_STATIC_PATH', default: '/', values: true },
  { name: 'ADMIN_SDK', default: '/srv/sdk' },
  { name: 'ADMIN_STREAMS', default: '/run/streams' },
  { name: 'ADMIN_CONTROLLER', default: '/run/controller/controller.sock' },
  { name: 'ADMIN_DEMO_SECONDS', default: '', values: true },
  { name: 'ADMIN_DEMO_SITES', default: '', values: true },
  { name: 'ADMIN_FRAME_ANCESTORS', default: '', values: true },
  { name: 'RUST_LOG', default: 'info' },
];

/** The controller container's environment. */
export const CONTROLLER: Var[] = [
  { name: 'IMAGE_DISPLAY, IMAGE_BROWSER, IMAGE_AGENT, IMAGE_PROXY', default: null, values: true },
  { name: 'STREAMS_VOLUME', default: null },
  { name: 'CONTROLLER_INSTANCE', default: 'easirun', values: true },
  { name: 'CONTROLLER_BROWSER_CPUS', default: '2', values: true },
  { name: 'CONTROLLER_BROWSER_MEMORY_MIB', default: '2048', values: true },
  { name: 'CONTROLLER_MAX_SESSIONS', default: '16', values: true },
  { name: 'CONTROLLER_SOCKET', default: '/run/controller/controller.sock' },
  { name: 'CONTROLLER_ADMIN_UID', default: '10004' },
  { name: 'CONTROLLER_BROWSER_SECCOMP', default: '/etc/controller/browser-seccomp.json' },
  { name: 'RUST_LOG', default: 'info' },
];

/** The controller's walled-garden and benchmark settings (advanced: they change what a session may reach). */
export const BENCH: Var[] = [
  { name: 'CONTROLLER_BENCH_NETWORK', default: '', values: true },
  { name: 'CONTROLLER_BENCH_FIXTURE', default: '', values: true },
  { name: 'CONTROLLER_BENCH_SPKI_FILE', default: '' },
  { name: 'CONTROLLER_BENCH_FIXTURE_ONLY', default: '', values: true },
  { name: 'CONTROLLER_MAX_SESSION_SECONDS', default: '', values: true },
  { name: 'CONTROLLER_BENCH_LANES', default: '' },
];
