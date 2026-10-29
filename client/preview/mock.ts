// The design preview's mock backend, as a Vite dev-server plugin: it answers the admin page's API (/api/*), its viewer
// stream (/ws/sessions/<id>) and the preview's own pages (/__preview/*) from example data, so every screen can be seen
// with hot reload and no stack. It exists only in the preview's dev server (vite.config.ts here); the client's build
// (client/vite.config.ts) never loads it.
//
// Each browser gets its own copy of a scenario's world (an `easi-preview` cookie names it), which the requests change
// as the real admin would: logging in, starting or closing a session, taking control.

import { randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';
import type { IncomingMessage, ServerResponse } from 'node:http';
import type { Duplex } from 'node:stream';
import { fileURLToPath } from 'node:url';
import type { Plugin } from 'vite';
import { baseWorld, DEFAULT_SCENARIO, DEMO_SESSION, makeSession, type ProxyPreview, type Scenario, SCENARIOS, type SessionPreview, type World } from './scenarios.ts';
import { CODEC_AV1, CODEC_H264, Codec, LeaseHolder, RecordPhase } from '../src/protocol.ts';
import { NAME_PATTERN } from '../src/ui/proxies.ts';
import { incoming, message, type Picture, readPng, Socket, tiles } from './stream.ts';

const COOKIE = 'easi-preview';
/** Worlds kept at most; the oldest goes first. */
const MAX_WORLDS = 200;
const CSRF = 'preview-csrf-token';
const RECOVERY = ['k7qd-2mzt-8fhx-w4rp', 'n3vb-6ycl-p9ga-t2je', 'h8su-4wkf-x6dm-q3zn', 'b5rt-9pvq-j2ce-m7ly', 'f4gx-7nza-r8wd-c6us', 'w2hm-5tke-v3qb-y9fp', 'p6cj-3ldr-z4xn-g8tw', 'e9ya-8qfu-s5mk-d2vh'];
const TOTP_SECRET = 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP';

interface Instance {
  id: string;
  scenario: Scenario;
  world: World;
  /** Open viewer streams, by session id. */
  viewers: Map<string, Set<Viewer>>;
  /** Who holds control of each session, by session id (see `leaseFor`). */
  leases: Map<string, Lease>;
  /** The demo a demo visitor's visit shows now, and when that visit ends (ms since the epoch; 0 before it starts). */
  site: string | null;
  visitEnds: number;
}
interface Viewer {
  socket: Socket;
  token: string;
}
/** A session's lease: its epoch, and the viewer token holding it, `agent`, or `other` (a viewer elsewhere). */
interface Lease {
  epoch: number;
  holder: string;
}

const here = fileURLToPath(new URL('.', import.meta.url));
const pictures = new Map<string, Picture>();
function picture(name: string): Picture {
  let p = pictures.get(name);
  if (!p) pictures.set(name, (p = readPng(readFileSync(`${here}frames/${name}.png`))));
  return p;
}

const worlds = new Map<string, Instance>();
function create(scenario: Scenario): Instance {
  const world = baseWorld();
  scenario.world(world);
  const instance: Instance = { id: randomBytes(6).toString('hex'), scenario, world, viewers: new Map(), leases: new Map(), site: null, visitEnds: 0 };
  worlds.set(instance.id, instance);
  if (worlds.size > MAX_WORLDS) worlds.delete(worlds.keys().next().value!);
  return instance;
}
const byId = (id: string) => SCENARIOS.find((s) => s.id === id);

function cookieOf(req: IncomingMessage): string | null {
  const match = /(?:^|;\s*)easi-preview=([0-9a-f]{12})/.exec(req.headers.cookie ?? '');
  return match?.[1] ?? null;
}
/** The world this browser uses, made from the default scenario when it has none yet. */
function instanceFor(req: IncomingMessage, res: ServerResponse | null): Instance {
  const known = worlds.get(cookieOf(req) ?? '');
  if (known) return known;
  const made = create(byId(DEFAULT_SCENARIO)!);
  res?.setHeader('set-cookie', `${COOKIE}=${made.id}; Path=/; SameSite=Lax; HttpOnly`);
  return made;
}

function json(res: ServerResponse, status: number, body: unknown, headers: Record<string, string> = {}): void {
  res.statusCode = status;
  // A text body is a plain-text answer, as the admin's body parser refuses a request it cannot read.
  res.setHeader('content-type', typeof body === 'string' ? 'text/plain; charset=utf-8' : 'application/json');
  res.setHeader('cache-control', 'no-store');
  for (const [name, value] of Object.entries(headers)) res.setHeader(name, value);
  res.end(typeof body === 'string' ? body : JSON.stringify(body));
}
async function bodyOf(req: IncomingMessage): Promise<Record<string, unknown>> {
  let text = '';
  for await (const chunk of req) text += chunk;
  try {
    const parsed: unknown = JSON.parse(text || '{}');
    return parsed !== null && typeof parsed === 'object' ? (parsed as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

/** A session as `GET /api/sessions` lists it: the preview's own fields left out. */
const listed = ({ frame: _frame, ...s }: SessionPreview) => s;
const unix = () => Math.floor(Date.now() / 1000);
const elsewhere = { code: 'wrong-origin', error: 'this admin is set up for another address', origin: 'https://easi.example.com:9443' };
/** A proxy's name as the admin takes it, by the page's own rule. */
const PROXY_NAME = new RegExp(`^(?:${NAME_PATTERN})$`);
/** How every password check answers a wrong one. */
const WRONG_PASSWORD: Answer = [403, { code: 'wrong-password', error: 'that is not your current password' }];
const wrongPassword = (password: unknown) => typeof password === 'string' && /wrong/i.test(password);

/** A status, a body, and any headers besides the JSON ones. */
type Answer = [number, unknown, Record<string, string>?];
/** What the admin answers when its controller runs as many checks as it takes at once (`busy` in a proxy's host). */
const BUSY: Answer = [503, { code: 'proxy-checks-busy', error: 'other checks are under way; try again shortly' }, { 'retry-after': '2' }];
/** What the admin answers when its controller cannot run a check (`broken` in a proxy's host). */
const UNCHECKED: Answer = [502, { code: 'proxy-check-unavailable', error: 'the check could not run: the controller could not create the check' }];
const MAX_EGRESSES = 32;
const MAX_LANGUAGES = 8;

/** Where a proxy's exit is, as a check would find it from its host (`fra`, `gru`, `tyo`; anywhere else the US). */
function proxyPlace(host: string) {
  if (host.includes('fra')) return { ip: '185.220.101.17', country: 'DE', country_name: 'Germany', region: 'Hesse', city: 'Frankfurt am Main', timezone: 'Europe/Berlin', languages: ['de-DE', 'de', 'en-US', 'en'] };
  if (host.includes('gru')) return { ip: '189.45.12.200', country: 'BR', country_name: 'Brazil', region: 'São Paulo', city: 'São Paulo', timezone: 'America/Sao_Paulo', languages: ['pt-BR', 'pt', 'en-US', 'en'] };
  if (host.includes('tyo')) return { ip: '103.5.140.33', country: 'JP', country_name: 'Japan', region: 'Tokyo', city: 'Tokyo', timezone: 'Asia/Tokyo', languages: ['ja-JP', 'ja', 'en-US', 'en'] };
  return { ip: '198.51.100.24', country: 'US', country_name: 'United States', region: null, city: null, timezone: 'America/New_York', languages: ['en-US', 'en'] };
}
/** `GET /api/egresses/options`, as locale/src/lib.rs lists them. */
const PROXY_OPTIONS = {
  timezones: ['UTC', 'America/New_York', 'America/Chicago', 'America/Denver', 'America/Los_Angeles', 'America/Phoenix', 'America/Anchorage', 'Pacific/Honolulu', 'America/Toronto', 'America/Vancouver', 'America/Mexico_City', 'America/Bogota', 'America/Lima', 'America/Santiago', 'America/Argentina/Buenos_Aires', 'America/Sao_Paulo', 'Europe/London', 'Europe/Dublin', 'Europe/Lisbon', 'Europe/Madrid', 'Europe/Paris', 'Europe/Amsterdam', 'Europe/Brussels', 'Europe/Berlin', 'Europe/Zurich', 'Europe/Rome', 'Europe/Stockholm', 'Europe/Warsaw', 'Europe/Athens', 'Europe/Istanbul', 'Europe/Kyiv', 'Europe/Moscow', 'Africa/Cairo', 'Africa/Lagos', 'Africa/Johannesburg', 'Asia/Dubai', 'Asia/Kolkata', 'Asia/Bangkok', 'Asia/Jakarta', 'Asia/Singapore', 'Asia/Hong_Kong', 'Asia/Shanghai', 'Asia/Taipei', 'Asia/Seoul', 'Asia/Tokyo', 'Australia/Sydney', 'Pacific/Auckland'],
  languages: [{ tag: 'en-US', name: 'English (United States)' }, { tag: 'en-GB', name: 'English (United Kingdom)' }, { tag: 'en', name: 'English' }, { tag: 'es-ES', name: 'Spanish (Spain)' }, { tag: 'es-MX', name: 'Spanish (Mexico)' }, { tag: 'es-419', name: 'Spanish (Latin America)' }, { tag: 'es', name: 'Spanish' }, { tag: 'pt-BR', name: 'Portuguese (Brazil)' }, { tag: 'pt-PT', name: 'Portuguese (Portugal)' }, { tag: 'pt', name: 'Portuguese' }, { tag: 'fr-FR', name: 'French (France)' }, { tag: 'fr-CA', name: 'French (Canada)' }, { tag: 'fr', name: 'French' }, { tag: 'de-DE', name: 'German (Germany)' }, { tag: 'de', name: 'German' }, { tag: 'it-IT', name: 'Italian' }, { tag: 'nl-NL', name: 'Dutch' }, { tag: 'pl-PL', name: 'Polish' }, { tag: 'ru-RU', name: 'Russian' }, { tag: 'uk-UA', name: 'Ukrainian' }, { tag: 'tr-TR', name: 'Turkish' }, { tag: 'ar', name: 'Arabic' }, { tag: 'he-IL', name: 'Hebrew' }, { tag: 'hi-IN', name: 'Hindi' }, { tag: 'id-ID', name: 'Indonesian' }, { tag: 'th-TH', name: 'Thai' }, { tag: 'vi-VN', name: 'Vietnamese' }, { tag: 'zh-CN', name: 'Chinese (Simplified)' }, { tag: 'zh-TW', name: 'Chinese (Traditional)' }, { tag: 'ja-JP', name: 'Japanese' }, { tag: 'ko-KR', name: 'Korean' }, { tag: 'sv-SE', name: 'Swedish' }],
};

/** A proxy as the admin lists it: its address without the user name or password, and how many sessions use it. */
function proxyListed(w: World, p: ProxyPreview) {
  const u = new URL(p.proxy);
  const port = Number(u.port) || (u.protocol === 'https:' ? 443 : u.protocol === 'socks5:' ? 1080 : 80);
  const in_use = w.sessions.filter((s) => s.egress_id === p.id).length;
  const place = proxyPlace(u.hostname);
  return { id: p.id, name: p.name, scheme: u.protocol.slice(0, -1), host: u.hostname.replace(/^\[(.*)\]$/, '$1'), port, has_user: u.username !== '', has_password: u.password !== '', timezone: p.timezone, languages: p.languages, country: place.country, country_name: place.country_name, in_use };
}
/** Why the admin refuses a proxy address, as `the proxy: …` says (never the address itself); null for one it takes. */
function proxyRefusal(address: unknown): string | null {
  const text = String(address).trim();
  const [scheme, rest] = text.includes('://') ? text.split('://', 2) : [null, ''];
  if (scheme === null) return 'not a URL';
  if (!['http', 'https', 'socks5'].includes(scheme!)) return 'the scheme is not http, https, or socks5';
  // The port is always written out: the admin assumes none.
  const port = /:(\d*)$/.exec(rest!.slice(rest!.lastIndexOf('@') + 1).replace(/^\[[^\]]*\]/, ''));
  if (!port) return 'the address needs a port';
  if (!(Number(port[1]) >= 1 && Number(port[1]) <= 65535)) return 'the port is not 1 to 65535';
  let u: URL;
  try {
    u = new URL(text);
  } catch {
    return 'the host is neither a DNS name nor an IP address';
  }
  if (!u.hostname) return 'the host is neither a DNS name nor an IP address';
  if (u.password && !u.username) return 'a password needs a user name';
  if (u.protocol === 'socks5:' && (u.username === '') !== (u.password === '')) return 'a SOCKS5 proxy needs a non-empty user name and password together';
  return null;
}
/** Checks an address as the admin would, after a moment: the host or password says how it fails (see `ProxyPreview`). */
async function proxyCheck(address: unknown): Promise<Answer> {
  const refused = proxyRefusal(address);
  if (refused) return [400, { code: 'proxy-url', error: `the proxy: ${refused}` }];
  const u = new URL(String(address).trim());
  const h = u.hostname;
  if (h.includes('busy')) return BUSY;
  if (h.includes('broken')) return UNCHECKED;
  await new Promise((r) => setTimeout(r, 700));
  const fail = (reason: string, detail: string): Answer => [200, { ok: false, code: `proxy-check-${reason}`, error: `the proxy did not pass its check: ${detail}`, detail }];
  if (h.includes('slow')) return fail('timeout', 'no answer in 10 s');
  if (h.includes('down')) return fail('unreachable', 'no route to host');
  if (h.includes('refuse')) return fail('refused', 'connection refused');
  if (h.includes('web')) return fail('protocol', 'HTTP/1.1 200 OK to CONNECT');
  if (h.includes('tls')) return fail('tls', 'certificate signed by an unknown authority');
  if (/wrong/i.test(decodeURIComponent(u.password))) return fail('auth', '407 Proxy Authentication Required');
  if (h.includes('odd')) return fail('unexpected', 'the exit address could not be read');
  const { ip, country, country_name, region, city, timezone, languages } = proxyPlace(h);
  return [200, { ok: true, latency_ms: 40 + Math.floor(Math.random() * 140), exit: { ip, country, country_name, region, city }, suggested: { timezone, languages } }];
}
/** A check's pass (its time), its failure's reason and detail, or the admin's answer when it could not run. */
async function proxyPassed(address: unknown): Promise<{ latency_ms: number } | { code: string; detail: string } | { answer: Answer }> {
  const checked = await proxyCheck(address);
  const answer = checked[1] as { ok: boolean; latency_ms: number; code: string; detail: string };
  if (checked[0] !== 200) return { answer: checked };
  return answer.ok ? { latency_ms: answer.latency_ms } : { code: answer.code, detail: answer.detail };
}
/** A save refused by its check, as the admin answers it. */
const failedCheck = (code: string, detail: string): Answer => [422, { code, error: `the proxy did not pass its check: ${detail}`, detail }];
/** A proxy's fields from a create or change (`id`, the one changed), or the admin's refusal. */
function proxyFields(w: World, body: Record<string, unknown>, id: string | null): Partial<ProxyPreview> | Answer {
  const out: Partial<ProxyPreview> = {};
  const partial = id !== null;
  // A field given as null, of the wrong type, or unknown, is refused as the admin's body parser does: 422, in plain text.
  const unread = (why: string): Answer => [422, `Failed to deserialize the JSON body into the target type: ${why}`];
  const unknown = Object.keys(body).find((k) => !['name', 'proxy', 'timezone', 'languages'].includes(k));
  if (unknown !== undefined) return unread(`unknown field \`${unknown}\``);
  for (const [k, v] of Object.entries(body)) {
    if (k === 'languages' ? !Array.isArray(v) || v.some((t) => typeof t !== 'string') : typeof v !== 'string') return unread(`${k}: invalid type`);
  }
  if (!partial && (body.name === undefined || body.proxy === undefined)) return unread('missing field');
  if (body.name !== undefined) {
    const name = String(body.name).trim();
    if (!PROXY_NAME.test(name)) return [400, { code: 'proxy-name', error: 'name it in 1 to 32 of a-z, 0-9 and -', max: 32 }];
    if (w.egresses.some((p) => p.name === name && p.id !== id)) return [409, { code: 'proxy-name-taken', error: 'another egress has that name' }];
    out.name = name;
  }
  if (body.proxy !== undefined || !partial) {
    const refused = proxyRefusal(body.proxy);
    if (refused) return [400, { code: 'proxy-url', error: `the proxy: ${refused}` }];
    out.proxy = String(body.proxy).trim();
  }
  // Left out with a new address, they come from where its exit is (as the admin fills them from its check).
  const place = out.proxy ? proxyPlace(new URL(out.proxy).hostname) : null;
  if (typeof body.timezone === 'string') out.timezone = body.timezone;
  else if (place) out.timezone = place.timezone;
  if (Array.isArray(body.languages)) out.languages = body.languages.map(String);
  else if (place) out.languages = place.languages;
  // An IANA name: one the admin offers, or one Node's own copy of the database knows.
  const zones = [...PROXY_OPTIONS.timezones, ...Intl.supportedValuesOf('timeZone')];
  if (out.timezone !== undefined && !zones.includes(out.timezone)) {
    return [400, { code: 'proxy-timezone', error: 'the time zone is not an IANA name such as America/Sao_Paulo' }];
  }
  const tag = /^[a-z]{2,3}(-([A-Z]{2}|[0-9]{3}|[A-Za-z]{4}))*$/;
  if (out.languages !== undefined && (out.languages.length < 1 || out.languages.length > MAX_LANGUAGES || !out.languages.every((t) => tag.test(t)))) {
    return [400, { code: 'proxy-languages', error: `languages must be 1 to ${MAX_LANGUAGES} tags such as pt-BR`, max: MAX_LANGUAGES }];
  }
  return out;
}
type Route = (i: Instance, body: Record<string, unknown>, match: RegExpExecArray) => Answer | Promise<Answer>;

function needsLogin(route: Route): Route {
  return (i, body, match) => (i.world.loggedIn ? route(i, body, match) : [401, { code: 'log-in', error: 'log in first' }]);
}
/** The administrator's alone: a demo visitor, logged in all the same, is refused. */
function needsAdministrator(route: Route): Route {
  return needsLogin((i, body, match) => (i.world.demo ? [403, { code: 'forbidden', error: 'forbidden' }] : route(i, body, match)));
}

function passkeyOptions(rpId: string, create: boolean): unknown {
  const challenge = randomBytes(32).toString('base64url');
  return create
    ? {
        publicKey: {
          challenge,
          rp: { name: 'easi.run', id: rpId },
          user: { id: randomBytes(16).toString('base64url'), name: 'admin', displayName: 'easi.run admin' },
          pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
          timeout: 60000,
          excludeCredentials: [],
        },
      }
    : { publicKey: { challenge, rpId, allowCredentials: [{ type: 'public-key', id: randomBytes(16).toString('base64url') }], timeout: 60000, userVerification: 'preferred' } };
}

/** The first factor added issues recovery codes; later ones none. */
function addFactor(w: World): string[] | null {
  const first = !w.security.app && w.security.passkeys.length === 0;
  if (first) w.security.recovery_codes = RECOVERY.length;
  return first ? RECOVERY : null;
}

function routes(host: () => string): [string, RegExp, Route][] {
  /** The visitor's visit, or null once its time ran out. */
  const visit = (i: Instance) => {
    const left = Math.ceil((i.visitEnds - Date.now()) / 1000);
    return left > 0 ? { session: DEMO_SESSION, site: currentSite(i), seconds_left: left } : null;
  };
  /** What the login offers and asks for follows what Security has on. */
  const passkeyOn = (w: World) => w.security.passkeys.length > 0 && w.security.passkeys_possible;
  return [
    ['GET', /^\/api\/status$/, (i) => [200, { enrolled: i.world.enrolled, passkey: passkeyOn(i.world), demo: i.world.demo }]],
    ['GET', /^\/api\/me$/, (i) => (i.world.loggedIn ? [200, { csrf: CSRF, demo: i.world.demo ? { visit: visit(i) } : null }] : [401, { code: 'log-in', error: 'log in first' }])],
    [
      'POST',
      /^\/api\/enroll$/,
      (i) => {
        const w = i.world;
        if (w.enroll === 'elsewhere') return [403, elsewhere];
        if (w.enroll === 'failed') return [500, { code: 'server-unsaved', error: 'the password could not be saved on the server; its log says why' }];
        if (w.enrolled) return [403, { code: 'forbidden', error: 'forbidden' }];
        w.enrolled = true;
        w.loggedIn = true;
        return [200, { csrf: CSRF }];
      },
    ],
    [
      'POST',
      /^\/api\/login$/,
      (i, body) => {
        const w = i.world;
        if (w.demo) {
          if (w.door === 'full') return [409, { code: 'demo-full', error: 'every demo browser is in use', retry_after: 30 }];
          if (w.door === 'failed') return [502, { code: 'demo-not-opened', error: 'the demo did not open; try again' }];
          w.loggedIn = true;
          i.site = typeof body.demo === 'string' ? body.demo : w.demo.sites[0]!;
          i.visitEnds = Date.now() + w.visitSeconds * 1000;
          return [200, { csrf: CSRF, demo: visit(i) }];
        }
        const pw = body.password;
        if (w.login === 'elsewhere') return [403, elsewhere];
        if (w.login === 'limited' || (typeof pw === 'string' && /limit/i.test(pw))) return [429, { code: 'too-many-attempts', error: 'too many attempts; wait' }];
        if (w.login === 'wrong' || wrongPassword(pw)) return [401, { code: 'log-in', error: 'log in first' }];
        const app = w.security.app;
        const passkey = passkeyOn(w);
        if (app || passkey) return [200, { second_step: { step: randomBytes(16).toString('base64url'), app, passkey } }];
        w.loggedIn = true;
        return [200, { csrf: CSRF }];
      },
    ],
    [
      'POST',
      /^\/api\/login\/code$/,
      (i, body) => {
        const w = i.world;
        if (w.code === 'again') return [401, { code: 'login-again', error: 'that took too long; enter your password again' }];
        if (w.code === 'wrong' || body.code === '000000' || body.code === 'wrong') return [401, { code: 'log-in', error: 'log in first' }];
        w.loggedIn = true;
        return [200, { csrf: CSRF }];
      },
    ],
    ['POST', /^\/api\/login\/passkey$/, () => [200, { ceremony: randomBytes(16).toString('base64url'), options: passkeyOptions(host(), false) }]],
    ['POST', /^\/api\/login\/passkey\/finish$/, (i) => ((i.world.loggedIn = true), [200, { csrf: CSRF }])],
    ['POST', /^\/api\/logout$/, (i) => ((i.world.loggedIn = false), [200, {}])],
    ['GET', /^\/api\/egresses$/, needsAdministrator((i) => [200, { egresses: i.world.egresses.map((p) => proxyListed(i.world, p)), max: MAX_EGRESSES }])],
    [
      'POST',
      /^\/api\/egresses$/,
      needsAdministrator(async (i, body) => {
        const fields = proxyFields(i.world, body, null);
        if (Array.isArray(fields)) return fields;
        if (i.world.egresses.length >= MAX_EGRESSES) return [409, { code: 'too-many-proxies', error: `at most ${MAX_EGRESSES} egresses; delete one first`, max: MAX_EGRESSES }];
        // Checked first: saved only when it passes, with where its exit is.
        const checked = await proxyPassed(fields.proxy);
        if ('answer' in checked) return checked.answer;
        if ('code' in checked) return failedCheck(checked.code, checked.detail);
        // Admitted again as it is saved: another request may have taken the name or the last place meanwhile.
        if (i.world.egresses.some((x) => x.name === fields.name)) return [409, { code: 'proxy-name-taken', error: 'another egress has that name' }];
        if (i.world.egresses.length >= MAX_EGRESSES) return [409, { code: 'too-many-proxies', error: `at most ${MAX_EGRESSES} egresses; delete one first`, max: MAX_EGRESSES }];
        const p = { id: `e-${randomBytes(3).toString('hex')}`, ...fields } as ProxyPreview;
        i.world.egresses.push(p);
        return [200, { ...proxyListed(i.world, p), latency_ms: checked.latency_ms }];
      }),
    ],
    ['GET', /^\/api\/egresses\/options$/, needsAdministrator(() => [200, PROXY_OPTIONS])],
    ['POST', /^\/api\/egresses\/check$/, needsAdministrator((_, body) => proxyCheck(body.proxy))],
    [
      'POST',
      /^\/api\/egresses\/([\w-]+)\/check$/,
      needsAdministrator((i, _, m) => {
        const p = i.world.egresses.find((x) => x.id === m[1]);
        return p ? proxyCheck(p.proxy) : [404, { code: 'no-proxy', error: 'no such egress' }];
      }),
    ],
    [
      'PATCH',
      /^\/api\/egresses\/([\w-]+)$/,
      needsAdministrator(async (i, body, m) => {
        const p = i.world.egresses.find((x) => x.id === m[1]);
        if (!p) return [404, { code: 'no-proxy', error: 'no such egress' }];
        const fields = proxyFields(i.world, body, p.id);
        if (Array.isArray(fields)) return fields;
        // A new address is checked before it replaces the old one.
        let latency: number | undefined;
        if (fields.proxy) {
          const checked = await proxyPassed(fields.proxy);
          if ('answer' in checked) return checked.answer;
          if ('code' in checked) return failedCheck(checked.code, checked.detail);
          latency = checked.latency_ms;
        }
        // As it is now: deleted or renamed by another request meanwhile.
        const now = i.world.egresses.find((x) => x.id === m[1]);
        if (!now) return [404, { code: 'no-proxy', error: 'no such egress' }];
        if (fields.name !== undefined && i.world.egresses.some((x) => x.name === fields.name && x.id !== now.id)) return [409, { code: 'proxy-name-taken', error: 'another egress has that name' }];
        Object.assign(now, fields);
        return [200, latency === undefined ? proxyListed(i.world, now) : { ...proxyListed(i.world, now), latency_ms: latency }];
      }),
    ],
    [
      'DELETE',
      /^\/api\/egresses\/([\w-]+)$/,
      needsAdministrator((i, _, m) => {
        if (!i.world.egresses.some((x) => x.id === m[1])) return [404, { code: 'no-proxy', error: 'no such egress' }];
        const using = i.world.sessions.filter((s) => s.egress_id === m[1]).map((s) => s.id);
        if (using.length > 0) return [409, { code: 'proxy-in-use', error: `sessions use this egress (${using.join(', ')}); stop them first`, sessions: using }];
        i.world.egresses = i.world.egresses.filter((x) => x.id !== m[1]);
        return [200, { ok: true }];
      }),
    ],
    ['GET', /^\/api\/sessions$/, needsLogin((i) => [200, { sessions: i.world.sessions.map(listed) }])],
    [
      'POST',
      /^\/api\/sessions$/,
      needsLogin(async (i, body) => {
        const w = i.world;
        const profile = w.profiles.profiles.find((p) => p.id === body.profile) ?? null;
        if (profile && !w.profiles.unlocked) return [423, { code: 'saved-locked', error: 'saved sessions are locked until the administrator logs in with the password, or enters it once to unlock them' }];
        const egress = typeof body.egress === 'string' ? body.egress : null;
        if (egress !== null) {
          // Checked again before use, by its id: a failing proxy refuses the start with its reason, before anything starts.
          const p = w.egresses.find((x) => x.id === egress);
          if (!p) return [400, { code: 'no-proxy', error: 'no such egress' }];
          const checked = await proxyPassed(p.proxy);
          if ('answer' in checked) return checked.answer[0] === 503 ? [503, checked.answer[1]] : checked.answer;
          if ('code' in checked) return [424, { code: checked.code, error: `the egress ${p.name} failed its check: ${checked.detail}`, detail: checked.detail, name: p.name }];
        }
        // A saved session remembers where it left from, as the admin's profiles do.
        if (profile) profile.egress = egress;
        const s = makeSession({
          id: randomBytes(6).toString('hex'),
          url: typeof body.url === 'string' ? body.url : 'https://example.com/',
          devtools: body.devtools === true,
          // Its proxy's name as it starts, which the listing keeps, and its id.
          egress: w.egresses.find((x) => x.id === egress)?.name ?? null,
          egress_id: egress,
          profile: profile ? { id: profile.id, name: profile.name } : null,
        });
        if (profile) profile.session = s.id;
        w.sessions.unshift(s);
        return [200, listed(s)];
      }),
    ],
    [
      'DELETE',
      /^\/api\/sessions\/([0-9a-f]{12})$/,
      needsLogin((i, _, m) => {
        const w = i.world;
        w.sessions = w.sessions.filter((s) => s.id !== m[1]);
        for (const p of w.profiles.profiles) if (p.session === m[1]) ((p.session = null), (p.used = unix()));
        for (const v of i.viewers.get(m[1]!) ?? []) v.socket.close(1000);
        return [200, {}];
      }),
    ],
    ['POST', /^\/api\/sessions\/([0-9a-f]{12})\/control$/, needsLogin((i, body, m) => (lease(i, m[1]!, body.viewer, true), [200, {}]))],
    ['POST', /^\/api\/sessions\/([0-9a-f]{12})\/release$/, needsLogin((i, body, m) => (lease(i, m[1]!, body.viewer, false), [200, {}]))],
    [
      'PUT',
      /^\/api\/sessions\/([0-9a-f]{12})\/(settings|network)$/,
      needsLogin((i, body, m) => {
        const s = i.world.sessions.find((x) => x.id === m[1]);
        if (!s) return [404, { code: 'no-session', error: 'no such session' }];
        if (m[2] === 'settings') s.settings = body as SessionPreview['settings'];
        else s.network = body as SessionPreview['network'];
        return [200, {}];
      }),
    ],
    ['GET', /^\/api\/sessions\/([0-9a-f]{12})\/stats$/, needsLogin((i, _, m) => [200, { session: i.world.sessions.find((s) => s.id === m[1]) ?? null, containers: containers(), viewers: [] }])],
    [
      'POST',
      /^\/api\/sessions\/([0-9a-f]{12})\/agent$/,
      needsLogin((_i, body) => [
        200,
        body.tool === 'list_windows'
          ? { windows: [{ id: 1, title: 'Wayfare — Flights & stays - Google Chrome', app_id: 'google-chrome', focused: true, x: 0, y: 0, width: 1280, height: 800 }] }
          : { code: 'server-failed', error: `the preview runs no tool (${String(body.tool)})` },
      ]),
    ],
    ['POST', /^\/api\/demo\/switch$/, needsLogin((i, body) => ((i.site = String(body.demo)), visit(i) ? [200, { demo: visit(i) }] : [410, { code: 'demo-turn-over', error: 'your turn is over' }]))],
    ['GET', /^\/api\/tokens$/, needsLogin((i) => [200, { tokens: i.world.tokens }])],
    [
      'POST',
      /^\/api\/tokens$/,
      needsLogin((i, body) => {
        const t = { id: `t-${randomBytes(3).toString('hex')}`, name: String(body.name), created: unix(), last_used: null, revoked: false };
        i.world.tokens.unshift(t);
        return [200, { ...t, token: `easi_${randomBytes(24).toString('base64url')}` }];
      }),
    ],
    ['DELETE', /^\/api\/tokens\/([\w-]+)$/, needsLogin((i, _, m) => ((i.world.tokens = i.world.tokens.filter((t) => t.id !== m[1])), [200, {}]))],
    ['GET', /^\/api\/profiles$/, needsLogin((i) => [200, i.world.profiles])],
    [
      'POST',
      /^\/api\/profiles$/,
      needsLogin((i, body) => {
        const p = { id: `p-${randomBytes(3).toString('hex')}`, name: String(body.name), allowed: [], created: unix(), used: null, size: null, session: null, unsaved: null, egress: null };
        i.world.profiles.profiles.push(p);
        return [200, p];
      }),
    ],
    ['POST', /^\/api\/profiles\/unlock$/, needsLogin((i, body) => (wrongPassword(body.password) ? WRONG_PASSWORD : ((i.world.profiles.unlocked = true), [200, {}])))],
    [
      'PUT',
      /^\/api\/profiles\/([\w-]+)$/,
      needsLogin((i, body, m) => {
        const p = i.world.profiles.profiles.find((x) => x.id === m[1]);
        if (!p) return [404, { code: 'no-saved-session', error: 'no such saved session' }];
        if (typeof body.name === 'string') p.name = body.name;
        if (Array.isArray(body.allowed)) p.allowed = body.allowed as string[];
        return [200, p];
      }),
    ],
    ['DELETE', /^\/api\/profiles\/([\w-]+)$/, needsLogin((i, _, m) => ((i.world.profiles.profiles = i.world.profiles.profiles.filter((p) => p.id !== m[1])), [200, {}]))],
    ['GET', /^\/api\/security$/, needsLogin((i) => [200, i.world.security])],
    ['POST', /^\/api\/security\/app$/, needsLogin(() => [200, { secret: TOTP_SECRET, uri: `otpauth://totp/easi.run:admin?secret=${TOTP_SECRET}&issuer=easi.run` }])],
    [
      'POST',
      /^\/api\/security\/app\/confirm$/,
      needsLogin((i, body) => {
        if (body.code === '000000') return [400, { code: 'wrong-code', error: 'that is not the code the app shows; try the next one' }];
        if (wrongPassword(body.password)) return WRONG_PASSWORD;
        const codes = addFactor(i.world);
        i.world.security.app = true;
        return [200, { recovery_codes: codes }];
      }),
    ],
    [
      'POST',
      /^\/api\/security\/app\/remove$/,
      needsLogin((i, body) => {
        if (wrongPassword(body.password)) return WRONG_PASSWORD;
        i.world.security.app = false;
        if (i.world.security.passkeys.length === 0) i.world.security.recovery_codes = 0;
        return [200, {}];
      }),
    ],
    ['POST', /^\/api\/security\/recovery$/, needsLogin((i, body) => (wrongPassword(body.password) ? WRONG_PASSWORD : ((i.world.security.recovery_codes = RECOVERY.length), [200, { recovery_codes: RECOVERY }])))],
    ['POST', /^\/api\/security\/passkeys$/, needsLogin((_i, body) => (wrongPassword(body.password) ? WRONG_PASSWORD : [200, { options: passkeyOptions(host(), true) }]))],
    [
      'POST',
      /^\/api\/security\/passkeys\/finish$/,
      needsLogin((i) => {
        const codes = addFactor(i.world);
        i.world.security.passkeys.push({ id: `pk-${randomBytes(3).toString('hex')}`, created: unix() });
        return [200, { recovery_codes: codes }];
      }),
    ],
    [
      'POST',
      /^\/api\/security\/passkeys\/([\w-]+)\/remove$/,
      needsLogin((i, body, m) => {
        if (wrongPassword(body.password)) return WRONG_PASSWORD;
        const s = i.world.security;
        s.passkeys = s.passkeys.filter((p) => p.id !== m[1]);
        if (!s.app && s.passkeys.length === 0) s.recovery_codes = 0;
        return [200, {}];
      }),
    ],
  ];
}

const currentSite = (i: Instance) => i.site ?? i.world.demo?.sites[0] ?? '';

const jitter = (v: number, spread: number) => Math.round((v + (Math.random() - 0.5) * spread) * 10) / 10;
function containers(): unknown {
  const mib = 1024 * 1024;
  return {
    containers: {
      agent: { status: 'running', cpu_percent: jitter(0.6, 0.4), memory_bytes: Math.round(jitter(38, 2) * mib) },
      browser: { status: 'running', cpu_percent: jitter(14, 6), memory_bytes: Math.round(jitter(412, 20) * mib) },
      display: { status: 'running', cpu_percent: jitter(9, 4), memory_bytes: Math.round(jitter(96, 6) * mib) },
      proxy: { status: 'running', cpu_percent: jitter(0.3, 0.2), memory_bytes: Math.round(jitter(12, 1) * mib) },
    },
  };
}

/** A session's lease, shared by its viewers: at first the agent's, or another viewer's when the scenario says so. */
function leaseFor(i: Instance, session: string): Lease {
  let l = i.leases.get(session);
  if (!l) i.leases.set(session, (l = { epoch: 1, holder: i.world.stream.holder === 'other' ? 'other' : 'agent' }));
  return l;
}
function holderFor(l: Lease, token: string): number {
  return l.holder === token ? LeaseHolder.You : l.holder === 'agent' ? LeaseHolder.Agent : LeaseHolder.Other;
}
function lease(i: Instance, session: string, viewer: unknown, take: boolean): void {
  const l = leaseFor(i, session);
  if (take && typeof viewer === 'string') l.holder = viewer;
  else if (!take && l.holder === viewer) l.holder = 'agent';
  else return;
  l.epoch += 1;
  for (const v of i.viewers.get(session) ?? []) {
    v.socket.send(message.lease(l.epoch, holderFor(l, v.token)));
    if (holderFor(l, v.token) === LeaseHolder.You) offers(i, v, l.epoch);
  }
}
/** What the page offers a viewer that just took control: copied text, a file chooser. */
function offers(i: Instance, v: Viewer, epoch: number): void {
  const plan = i.world.stream;
  setTimeout(() => {
    if (plan.copied) v.socket.send(message.copied(epoch, plan.copied));
    if (plan.fileRequest) v.socket.send(message.fileRequest(epoch, 1));
  }, 400);
}

const serverUs = () => process.hrtime.bigint() / 1000n;

/** A viewer's stream: the configuration and the whole screen once it says hello, then statistics, the clock, control. */
function watch(i: Instance, session: SessionPreview, token: string, socket: Socket): void {
  const v: Viewer = { socket, token };
  let set = i.viewers.get(session.id);
  if (!set) i.viewers.set(session.id, (set = new Set()));
  set.add(v);
  const plan = i.world.stream;
  const first = leaseFor(i, session.id);
  if (plan.holder === 'you' && first.holder === 'agent' && first.epoch === 1) first.holder = token;
  const epoch = 1;
  const pic = picture(session.frame);
  let stats: ReturnType<typeof setInterval> | undefined;
  let recording: { id: number; file: number; started: number; epoch: number } | null = null;
  socket.onClose = () => {
    clearInterval(stats);
    i.viewers.get(session.id)?.delete(v);
    // A viewer that goes gives control back, as the admin's lease does.
    lease(i, session.id, token, false);
  };
  socket.onMessage = (data) => {
    if (plan.silent) return;
    const m = incoming(data);
    if (m.type === 'hello') {
      const codec = m.codecs & CODEC_AV1 ? Codec.Av1 : m.codecs & CODEC_H264 ? Codec.H264 : Codec.None;
      socket.send(message.config(epoch, pic.width, pic.height, codec));
      for (const t of tiles(pic, epoch)) socket.send(t);
      // After the tiles' revisions (one per band of the picture).
      socket.send(message.cursor(epoch, 100n, 640, 420));
      const now = leaseFor(i, session.id);
      socket.send(message.lease(now.epoch, holderFor(now, token)));
      if (holderFor(now, token) === LeaseHolder.You) offers(i, v, now.epoch);
      stats = setInterval(() => socket.send(message.serverStats(serverUs(), 30, 0, 3100 + Math.round(Math.random() * 600), 6400)), 1000);
      if (plan.endAfterMs !== undefined) setTimeout(() => (plan.end === 'close' ? socket.close(1000) : socket.drop()), plan.endAfterMs);
    } else if (m.type === 'ping') {
      // A little latency, as over a real connection.
      setTimeout(() => socket.send(message.pong(m.client, serverUs())), 18);
    } else if (m.type === 'record-start') {
      // The recording's download starts with it and grows until it ends, as the display offers it.
      const le = leaseFor(i, session.id).epoch;
      recording = { id: m.recording, file: 1000 + m.recording, started: Date.now(), epoch: le };
      socket.send(message.recordState(le, m.recording, RecordPhase.Recording, recording.file, 0, 0));
      socket.send(message.fileOffer(le, recording.file, `easi.run recording ${new Date().toISOString().slice(0, 16).replace('T', ' ')}.txt`));
      socket.send(message.fileLink(le, recording.file, randomBytes(32)));
    } else if (m.type === 'record-stop' && recording?.id === m.recording) {
      const r = recording;
      recording = null;
      const seconds = Math.round((Date.now() - r.started) / 1000);
      setTimeout(() => {
        socket.send(message.recordState(r.epoch, r.id, RecordPhase.Ended, r.file, seconds, RECORDING.length));
        socket.send(message.fileEnd(r.epoch, r.file, RECORDING.length));
      }, 800);
    }
  };
}

/** What a recording's download is in the preview: a few words, not a video. */
const RECORDING = Buffer.from('A recording made in the easi.run design preview: there is no video in it.\n');

/** The viewer stream's upgrade, for a session of this browser's world; other upgrades (Vite's own) are left alone. */
function upgrade(req: IncomingMessage, raw: Duplex): void {
  const url = new URL(req.url ?? '/', 'http://preview');
  const match = /^\/ws\/sessions\/([0-9a-f]{12})$/.exec(url.pathname);
  if (!match) return;
  const i = worlds.get(cookieOf(req) ?? '');
  const id = match[1]!;
  const session = i?.world.sessions.find((s) => s.id === id) ?? (i?.world.demo && id === DEMO_SESSION ? demoSession() : undefined);
  if (!i || !i.world.loggedIn || !session) {
    raw.end('HTTP/1.1 404 Not Found\r\n\r\n');
    return;
  }
  if (Socket.accept(req, raw)) watch(i, session, url.searchParams.get('viewer') ?? '', new Socket(raw));
}

/** The demo visitor's session: the bank's sign-in, for the takeover demo. */
function demoSession(): SessionPreview {
  return { ...baseWorld().sessions[1]!, id: DEMO_SESSION, frame: 'signin' };
}

const escape = (text: string) => text.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);

/** The index of scenarios, grouped as the screens go: each link starts its scenario afresh. */
function index(): string {
  const groups = [...new Set(SCENARIOS.map((s) => s.group))];
  const items = groups
    .map(
      (g) =>
        `<section><h2>${escape(g)}</h2><ul>${SCENARIOS.filter((s) => s.group === g)
          .map((s) => `<li><a href="/__preview/go/${s.id}">${escape(s.title)}</a> <code>${s.id}</code>${s.note ? `<p>${escape(s.note)}</p>` : ''}</li>`)
          .join('')}</ul></section>`,
    )
    .join('');
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>easi.run design preview</title><style>
:root{color-scheme:light;font-family:system-ui,sans-serif;color:#14203a;background:#f5f7fb}
body{margin:0 auto;max-width:72rem;padding:32px 16px 64px}h1{font-size:28px;margin:0 0 4px;letter-spacing:-.02em}
.lead{color:#5b6680;margin:0 0 24px;max-width:52rem;line-height:1.5}main{display:grid;gap:16px;grid-template-columns:repeat(auto-fill,minmax(20rem,1fr))}
section{background:#fff;border:1px solid #e1e6ef;border-radius:16px;padding:16px 20px}h2{font-size:15px;margin:0 0 8px}
ul{list-style:none;margin:0;padding:0}li{padding:7px 0;border-top:1px solid #eef1f6}li:first-child{border-top:0}
a{color:#2457f5;font-weight:600;text-decoration:none}a:hover{text-decoration:underline}code{font-size:12px;color:#8590a6;margin-left:6px}
li p{margin:2px 0 0;font-size:13px;color:#5b6680}</style></head><body>
<h1>easi.run design preview</h1>
<p class="lead">The admin page on example data, with hot reload: nothing here talks to a real admin. Each link starts its
scenario afresh in this browser; from there the page works as usual (log in, open a session, take control). Any address
also takes <code>?scenario=&lt;id&gt;</code>. The preview’s chip at the bottom left comes back here.</p>
<main>${items}</main></body></html>`;
}

function redirect(res: ServerResponse, location: string, instance?: Instance): void {
  if (instance) res.setHeader('set-cookie', `${COOKIE}=${instance.id}; Path=/; SameSite=Lax; HttpOnly`);
  res.statusCode = 302;
  res.setHeader('location', location);
  res.setHeader('cache-control', 'no-store');
  res.end();
}

/** Starts `scenario` afresh for this browser, at the address it opens with. */
function start(res: ServerResponse, scenario: Scenario): void {
  const instance = create(scenario);
  redirect(res, `/${scenario.hash ?? ''}`, instance);
}

export function preview(): Plugin {
  let host = 'localhost';
  const table = routes(() => host);
  return {
    name: 'easi-run-preview',
    apply: 'serve',
    // By its file path: this directory may lie outside the client served (another checkout's, see client/preview/run.sh).
    transformIndexHtml: () => [{ tag: 'script', attrs: { type: 'module', src: `/@fs${here}runtime.ts` }, injectTo: 'body' }],
    configureServer(server) {
      server.httpServer?.on('upgrade', upgrade);
      server.middlewares.use((req, res, next) => {
        void (async () => {
          const url = new URL(req.url ?? '/', 'http://preview');
          host = (req.headers.host ?? 'localhost').replace(/:\d+$/, '');
          if (url.pathname === '/__preview' || url.pathname === '/__preview/') {
            res.setHeader('content-type', 'text/html; charset=utf-8');
            return res.end(index());
          }
          const go = /^\/__preview\/go\/([\w-]+)$/.exec(url.pathname);
          if (go) {
            const scenario = byId(go[1]!);
            return scenario ? start(res, scenario) : redirect(res, '/__preview/');
          }
          if (url.pathname === '/' && url.searchParams.has('scenario')) {
            const scenario = byId(url.searchParams.get('scenario')!);
            return scenario ? start(res, scenario) : redirect(res, '/__preview/');
          }
          if (url.pathname === '/__preview/state') {
            const i = instanceFor(req, res);
            return json(res, 200, { run: i.id, id: i.scenario.id, title: i.scenario.title, steps: i.scenario.steps ?? [], passkey: i.world.passkey });
          }
          const file = /^\/api\/sessions\/[0-9a-f]{12}\/files\/[\w-]+$/.exec(url.pathname);
          if (file) {
            res.setHeader('content-type', 'text/plain');
            res.setHeader('content-disposition', 'attachment; filename="easi.run preview recording.txt"');
            return res.end(RECORDING);
          }
          if (url.pathname.startsWith('/api/')) {
            const i = instanceFor(req, res);
            for (const [method, pattern, route] of table) {
              const match = pattern.exec(url.pathname);
              if (match && method === req.method) {
                const [status, body, headers] = await route(i, await bodyOf(req), match);
                return json(res, status, body, headers);
              }
            }
            return json(res, 404, { error: 'the preview has no answer for this request' });
          }
          // The page itself: a browser without a world gets the default scenario's.
          if (req.headers.accept?.includes('text/html')) instanceFor(req, res);
          next();
        })().catch(next);
      });
    },
  };
}
