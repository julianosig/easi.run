// The preview's example data and its scenarios: each one a state of the admin (a world) the mock backend answers from,
// the address the page opens at, and the clicks and typing that lead to the screen it shows (`steps`, which the page's
// preview runtime plays). Example data only: nothing here is real or reaches the client's build.

import { ALL_FEATURES } from '../src/features.ts';
import type { MessageParams } from '../src/i18n/keys.ts';
import type { DemoInfo, SessionInfo, TokenInfo } from '../src/ui/api.ts';
import type { ProfileList } from '../src/ui/profiles.ts';
import type { Proxy } from '../src/ui/proxies.ts';
import type { SecondStep } from '../src/ui/second.ts';

/** One click or entry the preview runtime plays on the page, once the element shows. */
export type Step =
  | { fill: string; value: string }
  /** An element by a selector that names no text (the page's language changes its text). */
  | { click: string }
  /** A button or link whose visible text or accessible name is the message `press` in the page's language. */
  | { press: keyof MessageParams; params?: Record<string, string> }
  | { wait: number };

/** How the passkey prompt (the browser's, which the preview stands in for) answers. */
export type PasskeyAnswer = 'ask' | 'ok' | 'cancel' | 'untrusted';

export interface SessionPreview extends SessionInfo {
  settings: { codec: 'auto' | 'av1' | 'h264' | 'tiles_only'; bitrate_kbps: number; max_fps: number };
  network: { latency_ms: number; loss_percent: number; bandwidth_kbps: number };
  /** The example screen it streams (preview/frames/<frame>.png). */
  frame: 'travel' | 'signin';
}

/** What the stream does once a viewer connects. */
export interface StreamPlan {
  /** Who holds control when the viewer connects. */
  holder: 'agent' | 'you' | 'other';
  /** Never answers: the screen stays "Connecting…". */
  silent?: boolean;
  /** After this long the stream ends: `drop` as a failed network does, `close` as a session that stopped. */
  endAfterMs?: number;
  end?: 'drop' | 'close';
  /** Text the page copied, offered once the viewer holds control. */
  copied?: string;
  /** The page opens a file chooser once the viewer holds control. */
  fileRequest?: boolean;
}

export interface World {
  enrolled: boolean;
  loggedIn: boolean;
  /** How `POST /api/enroll` answers the first password: it sets it, the page is at another address, or it is not saved. */
  enroll: 'ok' | 'elsewhere' | 'failed';
  /** How `POST /api/login` answers a password (any password, unless it says `wrong` or `limit`). */
  login: 'ok' | 'wrong' | 'limited' | 'elsewhere';
  /** How `POST /api/login/code` answers a code (any code, unless it is 000000). */
  code: 'ok' | 'wrong' | 'again';
  passkey: PasskeyAnswer;
  /** The second step at sign-in, as Security shows it; the login asks for what is on, and offers a passkey once one is added. */
  security: SecondStep;
  sessions: SessionPreview[];
  /** The proxies, each with the address it was saved with (`proxy`, never listed); see `proxyCheck` in the mock. */
  egresses: ProxyPreview[];
  profiles: ProfileList;
  tokens: TokenInfo[];
  stream: StreamPlan;
  demo: DemoInfo | null;
  /** How the demo's door answers a visitor. */
  door: 'ok' | 'full' | 'failed';
  /** Seconds a new demo visit has; it ends on the server too. */
  visitSeconds: number;
}

export interface ProxyPreview extends Pick<Proxy, 'id' | 'name' | 'timezone' | 'languages'> {
  /** The whole address, credentials included: how the mock checks it (a host with `slow` times out, `down` is
   * unreachable, `refuse` refuses, `web` is no proxy, `tls` fails TLS, `odd` answers unexpectedly, `busy` finds the server running other
   * checks (503), `broken` finds it unable to run one (502); a password `wrong` is refused as such; `fra`, `gru` and `tyo` exit in Germany, Brazil and Japan, anything else in the US). */
  proxy: string;
}

export interface Scenario {
  id: string;
  group: string;
  title: string;
  /** What to look at, or what to try, on the index. */
  note?: string;
  /** The address's fragment the page opens with (a watch link). */
  hash?: string;
  world: (w: World) => void;
  steps?: Step[];
}

const now = Math.floor(Date.UTC(2026, 9, 25, 9, 30) / 1000);
const hour = 3600;
const day = 24 * hour;
const settings = { codec: 'auto', bitrate_kbps: 4000, max_fps: 30 } as const;
const network = { latency_ms: 0, loss_percent: 0, bandwidth_kbps: 0 };
/** A session as the preview starts one: every feature on, the default stream, and no proxy, profile or failure. */
export function makeSession(fields: Pick<SessionPreview, 'id' | 'url'> & Partial<SessionPreview>): SessionPreview {
  return { devtools: false, features: { ...ALL_FEATURES }, settings: { ...settings }, network: { ...network }, egress: null, egress_id: null, profile: null, failed: null, frame: 'travel', ...fields };
}
export const TRAVEL = 'a1b2c3d4e5f6';
export const BANK = '0f9e8d7c6b5a';
export const DEMO_SESSION = 'de30de30de30';

/** The admin as most scenarios have it: set up, logged in, with sessions, profiles and tokens to show. */
export function baseWorld(): World {
  return {
    enrolled: true,
    loggedIn: true,
    enroll: 'ok',
    login: 'ok',
    code: 'ok',
    passkey: 'ask',
    security: {
      app: true,
      recovery_codes: 8,
      passkeys: [{ id: 'pk-macbook', created: now - 12 * day }],
      passkeys_possible: true,
    },
    sessions: [
      makeSession({ id: TRAVEL, url: 'https://wayfare.example/flights?from=LIS&to=AMS', profile: { id: 'p-personal', name: 'Personal' } }),
      makeSession({
        id: BANK,
        url: 'https://secure.lumenbank.example/sign-in/verify',
        devtools: true,
        settings: { codec: 'av1', bitrate_kbps: 6000, max_fps: 60 },
        egress: 'frankfurt',
        egress_id: 'e-frankfurt',
        frame: 'signin',
      }),
      makeSession({
        id: '5c6d7e8f9a0b',
        url: 'https://shop.example/checkout/shipping',
        features: { ...ALL_FEATURES, uploads: false, recording: false },
        network: { latency_ms: 120, loss_percent: 1, bandwidth_kbps: 8000 },
        egress: 'sao-paulo',
        egress_id: 'e-saopaulo',
        profile: { id: 'p-shopping', name: 'Shopping' },
      }),
      makeSession({ id: '9a8b7c6d5e4f', url: 'https://news.example/', failed: { code: 'server-failed', error: 'controller: the browser stopped responding' } }),
    ],
    egresses: [
      { id: 'e-frankfurt', name: 'frankfurt', proxy: 'http://ana:s3cret@fra.proxy.example:8080', timezone: 'Europe/Berlin', languages: ['de-DE', 'de'] },
      { id: 'e-saopaulo', name: 'sao-paulo', proxy: 'socks5://ana:s3cret@gru.proxy.example:1080', timezone: 'America/Sao_Paulo', languages: ['pt-BR', 'pt'] },
      { id: 'e-tokyo', name: 'tokyo', proxy: 'http://ana:s3cret@slow.tyo.proxy.example:3128', timezone: 'Asia/Tokyo', languages: ['ja-JP', 'ja'] },
    ],
    profiles: {
      unlocked: true,
      max: 8,
      profiles: [
        { id: 'p-personal', name: 'Personal', allowed: ['local', 't-laptop'], created: now - 40 * day, used: now - 2 * hour, size: 19_400_000, session: TRAVEL, unsaved: null, egress: null },
        { id: 'p-work', name: 'Work', allowed: [], created: now - 9 * day, used: now - 3 * day, size: 7_300_000, session: null, unsaved: null, egress: null },
        { id: 'p-shopping', name: 'Shopping', allowed: ['t-nightly'], created: now - 2 * day, used: null, size: null, session: '5c6d7e8f9a0b', unsaved: null, egress: null },
        { id: 'p-research', name: 'Research', allowed: ['t-nightly'], created: now - 20 * day, used: now - 5 * day, size: 12_800_000, session: null, unsaved: { code: 'save-failed', error: 'writing the saved session’s archive: no space left on device; it keeps its last save' }, egress: null },
      ],
    },
    tokens: [
      { id: 't-laptop', name: 'Claude Code on my laptop', created: now - 30 * day, last_used: now - 25 * 60, revoked: false },
      { id: 't-nightly', name: 'Nightly price checker', created: now - 6 * day, last_used: now - 7 * hour, revoked: false },
      { id: 't-old', name: 'Old CI runner', created: now - 90 * day, last_used: null, revoked: true },
    ],
    stream: { holder: 'agent' },
    demo: null,
    door: 'ok',
    visitSeconds: 287,
  };
}

/** Logged out, with the second step as given (none by default: the password alone). */
const loggedOut = (w: World, app = false, passkey = false) => {
  w.loggedIn = false;
  w.security = { app, recovery_codes: app || passkey ? 8 : 0, passkeys: passkey ? [{ id: 'pk-macbook', created: now - 12 * day }] : [], passkeys_possible: true };
};
const notSetUp = (w: World) => {
  w.enrolled = false;
  w.loggedIn = false;
  w.security = { app: false, recovery_codes: 0, passkeys: [], passkeys_possible: true };
};
const password: Step[] = [{ fill: '#password', value: 'correct horse battery' }, { click: 'button[type=submit]' }];
const created: Step[] = [
  { fill: '#password', value: 'correct horse battery' },
  { fill: '#confirm', value: 'correct horse battery' },
  { click: 'button[type=submit]' },
];
const demo = (w: World) => {
  w.loggedIn = false;
  w.demo = { sites: ['takeover', 'drag', 'upload', 'download', 'sound', 'typing', 'clipboard'], seconds: 300, embedders: [] };
};
const watch = (id: string) => `#session=${id}`;

export const SCENARIOS: Scenario[] = [
  // First password.
  { id: 'first-password', group: 'First password', title: 'Create your password', note: 'Create any password of 12 characters or more.', world: notSetUp },
  { id: 'first-password-typing', group: 'First password', title: 'Typing: too short, not matching', world: notSetUp, steps: [{ fill: '#password', value: 'short pass' }, { fill: '#confirm', value: 'shirt' }] },
  { id: 'first-password-elsewhere', group: 'First password', title: 'Opened at another address', world: (w) => (notSetUp(w), (w.enroll = 'elsewhere')), steps: created },
  { id: 'first-password-failed', group: 'First password', title: 'The password could not be saved', world: (w) => (notSetUp(w), (w.enroll = 'failed')), steps: created },
  // The offer of a second step, right after.
  { id: 'protect', group: 'Protect your admin', title: 'The offer of a second step', world: notSetUp, steps: created },
  { id: 'protect-app', group: 'Protect your admin', title: 'Authenticator app: QR code', world: notSetUp, steps: [...created, { press: 'account.protect.app' }] },
  { id: 'protect-codes', group: 'Protect your admin', title: 'Recovery codes', note: 'Any 6 digits turn the app on.', world: notSetUp, steps: [...created, { press: 'account.protect.app' }, { fill: '#app-code', value: '492817' }] },
  { id: 'protect-passkey', group: 'Protect your admin', title: 'Adding a passkey: the browser’s prompt', note: 'The prompt is the preview’s stand-in for the browser’s own.', world: notSetUp, steps: [...created, { press: 'account.protect.passkey' }] },
  { id: 'protect-untrusted', group: 'Protect your admin', title: 'Passkey refused: certificate not trusted', world: (w) => (notSetUp(w), (w.passkey = 'untrusted')), steps: [...created, { press: 'account.protect.passkey' }] },
  { id: 'protect-cancelled', group: 'Protect your admin', title: 'Passkey prompt closed', world: (w) => (notSetUp(w), (w.passkey = 'cancel')), steps: [...created, { press: 'account.protect.passkey' }] },
  // Login.
  { id: 'login', group: 'Log in', title: 'Log in', note: 'Any password logs in; one with “wrong” is refused, one with “limit” rate-limited.', world: (w) => loggedOut(w) },
  { id: 'login-wrong', group: 'Log in', title: 'Wrong password', world: (w) => (loggedOut(w), (w.login = 'wrong')), steps: password },
  { id: 'login-limited', group: 'Log in', title: 'Too many attempts', world: (w) => (loggedOut(w), (w.login = 'limited')), steps: password },
  { id: 'login-elsewhere', group: 'Log in', title: 'Opened at another address', world: (w) => (loggedOut(w), (w.login = 'elsewhere')), steps: password },
  { id: 'login-passkey', group: 'Log in', title: 'With a passkey offered', world: (w) => loggedOut(w, false, true) },
  { id: 'login-passkey-prompt', group: 'Log in', title: 'Signing in with a passkey: the prompt', world: (w) => loggedOut(w, false, true), steps: [{ press: 'account.login.passkey' }] },
  // The second step.
  { id: 'code', group: 'Second step', title: 'Authenticator code', note: 'Any 6 digits sign in; 000000 is refused.', world: (w) => loggedOut(w, true, true), steps: password },
  { id: 'code-wrong', group: 'Second step', title: 'Wrong code', world: (w) => (loggedOut(w, true), (w.code = 'wrong')), steps: [...password, { fill: '#code', value: '123456' }, { click: 'button[type=submit]' }] },
  { id: 'code-expired', group: 'Second step', title: 'Took too long', world: (w) => (loggedOut(w, true), (w.code = 'again')), steps: [...password, { fill: '#code', value: '123456' }, { click: 'button[type=submit]' }] },
  { id: 'recovery', group: 'Second step', title: 'Recovery code', world: (w) => loggedOut(w, true, true), steps: [...password, { press: 'account.second.recovery.switch' }] },
  { id: 'passkey-first', group: 'Second step', title: 'Passkey only: confirm it’s you', world: (w) => loggedOut(w, false, true), steps: password },
  { id: 'passkey-prompt', group: 'Second step', title: 'Passkey: the prompt, asked by itself', world: (w) => loggedOut(w, false, true), steps: [...password] },
  { id: 'passkey-cancelled', group: 'Second step', title: 'Passkey: prompt closed', world: (w) => (loggedOut(w, false, true), (w.passkey = 'cancel')), steps: [...password] },
  { id: 'passkey-untrusted', group: 'Second step', title: 'Passkey: certificate not trusted', world: (w) => (loggedOut(w, true, true), (w.passkey = 'untrusted')), steps: [...password, { press: 'account.second.passkey.switch' }] },
  // The admin.
  { id: 'sessions', group: 'Admin', title: 'Sessions: running and saved', note: 'Watch opens a session; Resume a saved one; New session opens the form. The sidebar opens the other pages.', hash: '#sessions', world: () => {} },
  { id: 'sessions-new', group: 'Admin', title: 'Starting a session', hash: '#sessions', world: () => {}, steps: [{ press: 'sessions.new' }] },
  { id: 'sessions-empty', group: 'Admin', title: 'Nothing yet', hash: '#sessions', world: (w) => ((w.sessions = []), (w.profiles.profiles = []), (w.tokens = []), (w.egresses = [])) },
  { id: 'sessions-saved-only', group: 'Admin', title: 'One saved session, none running: no filter', hash: '#sessions', world: (w) => ((w.sessions = []), (w.profiles.profiles = w.profiles.profiles.filter((p) => p.id === 'p-work'))) },
  { id: 'agents-empty', group: 'Admin', title: 'No agents yet', hash: '#agents', world: (w) => (w.tokens = []) },
  { id: 'sessions-locked', group: 'Admin', title: 'Saved sessions locked (after a passkey sign-in)', hash: '#sessions', world: (w) => (w.profiles.unlocked = false) },
  { id: 'profile-agents', group: 'Admin', title: 'A saved session’s agents', hash: '#sessions', world: () => {}, steps: [{ press: 'sessions.edit', params: { name: 'Work' } }] },
  { id: 'agents', group: 'Admin', title: 'Agent tokens', hash: '#agents', world: () => {} },
  { id: 'token-created', group: 'Admin', title: 'A token just created', hash: '#agents', world: () => {}, steps: [{ press: 'agents.new' }, { fill: '#token-name', value: 'Research agent' }, { press: 'agents.create.submit' }] },
  { id: 'proxies', group: 'Proxies', title: 'Proxies', note: 'Each is checked as the page opens, one after another; tokyo times out.', hash: '#proxies', world: () => {} },
  { id: 'proxies-empty', group: 'Proxies', title: 'No proxies yet', hash: '#proxies', world: (w) => (w.egresses = []) },
  { id: 'proxy-add', group: 'Proxies', title: 'Adding a proxy', note: 'Try http://ana:pw@gru.example:8080 (Brazil). A host with “slow”, “down”, “refuse”, “web”, “tls” or “odd”, or the password “wrong”, fails its test; one with “busy” finds the server busy, “broken” unable to check.', hash: '#proxies', world: () => {}, steps: [{ press: 'proxies.add' }] },
  { id: 'proxy-connected', group: 'Proxies', title: 'A proxy connected', hash: '#proxies', world: () => {}, steps: [{ press: 'proxies.add' }, { fill: '#proxy-name', value: 'rio' }, { fill: '#proxy-address', value: 'http://ana:pw@gru2.proxy.example:8080' }, { press: 'proxies.connect' }] },
  { id: 'proxy-refused', group: 'Proxies', title: 'A proxy that fails to connect', hash: '#proxies', world: () => {}, steps: [{ press: 'proxies.add' }, { fill: '#proxy-name', value: 'office' }, { fill: '#proxy-address', value: 'http://ana:wrong@fra2.proxy.example:8080' }, { press: 'proxies.connect' }] },
  { id: 'proxy-edit', group: 'Proxies', title: 'Editing a proxy', hash: '#proxies', world: () => {}, steps: [{ press: 'proxies.proxy.edit', params: { name: 'frankfurt' } }] },
  { id: 'proxy-session', group: 'Proxies', title: 'Starting a session through a proxy', note: 'Turn on Use proxy and pick tokyo: its check times out.', hash: '#sessions', world: () => {}, steps: [{ press: 'sessions.new' }, { wait: 300 }] },
  { id: 'proxy-many', group: 'Proxies', title: 'Many proxies: finding one by its name', note: 'New session, then Use proxy: type part of a name, or move with the arrows.', hash: '#sessions', world: (w) => (w.egresses = [...w.egresses, ...['amsterdam', 'berlin', 'chicago', 'dallas', 'dubai', 'lisbon', 'london', 'madrid', 'miami', 'paris', 'seoul', 'sydney'].map((name) => ({ id: `e-${name}`, name, proxy: `http://ana:s3cret@${name}.proxy.example:8080`, timezone: 'UTC', languages: ['en-US', 'en'] }))]) },
  { id: 'security', group: 'Security', title: 'Second step on', hash: '#security', world: () => {} },
  { id: 'security-off', group: 'Security', title: 'Password alone', hash: '#security', world: (w) => (w.security = { app: false, recovery_codes: 0, passkeys: [], passkeys_possible: true }) },
  { id: 'security-ip', group: 'Security', title: 'At an IP address (no passkeys)', hash: '#security', world: (w) => (w.security = { app: true, recovery_codes: 1, passkeys: [], passkeys_possible: false }) },
  { id: 'security-replace', group: 'Security', title: 'Replacing the app', hash: '#security', world: () => {}, steps: [{ press: 'security.app.replace' }] },
  { id: 'security-confirm', group: 'Security', title: 'Asking for the password', note: 'Any password confirms; “wrong” is refused.', hash: '#security', world: () => {}, steps: [{ press: 'security.app.turnOff' }] },
  // The session screen.
  { id: 'session', group: 'Browser session', title: 'Watching (the agent may act)', note: 'Take control, Give back, Record and Volume work against the mock.', hash: watch(TRAVEL), world: () => {} },
  { id: 'session-control', group: 'Browser session', title: 'You hold control', hash: watch(BANK), world: (w) => (w.stream = { holder: 'you' }) },
  { id: 'session-other', group: 'Browser session', title: 'Another viewer holds control', hash: watch(TRAVEL), world: (w) => (w.stream = { holder: 'other' }) },
  { id: 'session-offers', group: 'Browser session', title: 'The page copied text and asks for files', hash: watch(TRAVEL), world: (w) => (w.stream = { holder: 'you', copied: 'LIS → AMS, Thu 14 May, €112', fileRequest: true }) },
  { id: 'session-recording', group: 'Browser session', title: 'Recording', hash: watch(BANK), world: (w) => (w.stream = { holder: 'you' }), steps: [{ press: 'session.record.open' }, { wait: 300 }, { press: 'session.record.start' }] },
  { id: 'session-connecting', group: 'Browser session', title: 'Connecting', hash: watch(TRAVEL), world: (w) => (w.stream = { holder: 'agent', silent: true }) },
  { id: 'session-lost', group: 'Browser session', title: 'Connection lost', note: 'The stream drops 3 seconds in.', hash: watch(TRAVEL), world: (w) => (w.stream = { holder: 'you', endAfterMs: 3000, end: 'drop' }) },
  { id: 'session-ended', group: 'Browser session', title: 'Session ended', note: 'The session stops 3 seconds in.', hash: watch(BANK), world: (w) => (w.stream = { holder: 'agent', endAfterMs: 3000, end: 'close' }) },
  // The public demo.
  { id: 'demo', group: 'Public demo', title: 'A visit', world: demo },
  { id: 'demo-waiting', group: 'Public demo', title: 'Every browser in use', world: (w) => (demo(w), (w.door = 'full')) },
  { id: 'demo-failed', group: 'Public demo', title: 'The visit could not start', world: (w) => (demo(w), (w.door = 'failed')) },
  { id: 'demo-over', group: 'Public demo', title: 'Time’s up', note: 'The visit ends 8 seconds in.', world: (w) => (demo(w), (w.visitSeconds = 8)) },
];

export const DEFAULT_SCENARIO = 'login';
