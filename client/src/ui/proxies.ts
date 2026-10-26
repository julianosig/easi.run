// The proxies (egresses, as the API calls them) a session may browse from: what a check's answer says to a person, and
// how a proxy's place, time zone and languages read.

/** A proxy as `GET /api/egresses` lists it (admin/src/egresses.rs): never its user name or password. */
export interface Proxy {
  id: string;
  /** 1 to 32 of a-z, 0-9 and -, unique: what agents name it. */
  name: string;
  scheme: string;
  host: string;
  port: number;
  has_user: boolean;
  has_password: boolean;
  timezone: string;
  languages: string[];
  /** Where its exit address is, as its last passed check found. */
  country: string | null;
  country_name: string | null;
  /** How many sessions use it now. */
  in_use: number;
}
/** `GET /api/egresses`: the proxies, and how many there may be. */
export interface ProxyList {
  egresses: Proxy[];
  max: number;
}

export type CheckReason = 'unreachable' | 'timeout' | 'auth' | 'refused' | 'tls' | 'protocol' | 'unexpected';
/** Where a proxy's traffic leaves from, as a check found it: a region or city only when known. */
export interface Exit {
  ip: string;
  country: string;
  country_name: string | null;
  region: string | null;
  city: string | null;
}
/** What checking a proxy answers: where its exit is, and the time zone and languages a browser there would have. */
export type Check =
  | {
      ok: true;
      latency_ms: number;
      exit?: Exit;
      suggested?: { timezone: string; languages: string[] };
    }
  | { ok: false; reason: CheckReason; detail?: string };
/** The time zones and languages the admin offers to pick from. */
export interface ProxyOptions {
  timezones: string[];
  languages: { tag: string; name: string }[];
}

const REASONS: Record<CheckReason, string> = {
  auth: 'Wrong user name or password',
  unreachable: 'Can’t reach the proxy',
  timeout: 'The proxy took too long',
  refused: 'The proxy refused the connection',
  tls: 'Secure connection failed',
  protocol: 'Not a proxy at that address',
  unexpected: 'The proxy answered, but not as expected',
};

/** What a check that could not run because the server runs others now (503) says: it may be asked again. */
export const BUSY = 'Busy, try again in a moment';
/** A proxy's name as the admin takes it: 1 to 32 of a-z, 0-9 and -. */
export const NAME_PATTERN = '[a-z0-9\\-]{1,32}';

/** Whether `reason` is one a proxy's check fails with. */
export const isCheckReason = (reason: unknown): reason is CheckReason => typeof reason === 'string' && Object.hasOwn(REASONS, reason);

/** Why a check failed, in plain words; an unknown reason reads as the proxy being unreachable. */
export function reasonText(reason: unknown): string {
  return typeof reason === 'string' && Object.hasOwn(REASONS, reason) ? REASONS[reason as CheckReason] : REASONS.unreachable;
}

/** The flag of a two-letter country code, as the emoji its regional letters make; nothing for anything else. */
export function flagOf(code: string | null | undefined): string {
  if (!code || !/^[A-Za-z]{2}$/.test(code)) return '';
  return String.fromCodePoint(...[...code.toUpperCase()].map((c) => 0x1f1e6 + c.charCodeAt(0) - 65));
}

/** Where a check found the exit, as a person reads it: its flag, city, region and country (each name once), and its
 * address. */
export function exitText(exit: Exit): string {
  const names = [exit.city, exit.region, exit.country_name ?? exit.country].filter((n): n is string => !!n);
  const place = names.filter((n, i) => names.indexOf(n) === i).join(', ');
  return [`${flagOf(exit.country)} ${place}`.trim(), exit.ip].join(' · ');
}

/** A time zone as a person reads it: its city ("America/Sao_Paulo" reads "Sao Paulo"). */
export const zoneName = (zone: string) => (zone.split('/').pop() ?? zone).replace(/_/g, ' ');

/** A language tag by its name in `names` (the admin's list), else as the browser names it, else the tag itself. */
export function languageName(tag: string, names: { tag: string; name: string }[] = []): string {
  const known = names.find((l) => l.tag === tag)?.name;
  if (known) return known;
  try {
    return new Intl.DisplayNames(['en'], { type: 'language' }).of(tag) ?? tag;
  } catch {
    return tag;
  }
}

/** A proxy's address as a list shows it: never its password. */
export const addressText = (proxy: Pick<Proxy, 'host' | 'port'>) => `${proxy.host}:${proxy.port}`;
