// The proxies (egresses, as the API calls them) a session may browse from: what a check's answer says to a person, and
// how a proxy's place, time zone and languages read. Texts are message keys, which the page translates as it draws.
import type { MessageParams } from '../i18n/keys.ts';

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
  | { ok: false; code: CheckCode; detail?: string };
/** The time zones and languages the admin offers to pick from. */
export interface ProxyOptions {
  timezones: string[];
  languages: { tag: string; name: string }[];
}

/** Each code a proxy's check fails with (admin/src/coded.rs), and the message saying why. */
const CHECKS = {
  'proxy-check-auth': 'errors.proxy-check-auth',
  'proxy-check-unreachable': 'errors.proxy-check-unreachable',
  'proxy-check-timeout': 'errors.proxy-check-timeout',
  'proxy-check-refused': 'errors.proxy-check-refused',
  'proxy-check-tls': 'errors.proxy-check-tls',
  'proxy-check-protocol': 'errors.proxy-check-protocol',
  'proxy-check-unexpected': 'errors.proxy-check-unexpected',
} as const satisfies Record<string, keyof MessageParams>;
/** Why a proxy's check failed: its code. */
export type CheckCode = keyof typeof CHECKS;
/** The message saying why a check failed. */
export type CheckKey = (typeof CHECKS)[CheckCode];

/** What a check that could not run because the server runs others now (503) says (its message key): it may be asked
 * again. */
export const BUSY = 'errors.proxy-checks-busy' satisfies keyof MessageParams;
/** A proxy's name as the admin takes it: 1 to 32 of a-z, 0-9 and -. */
export const NAME_PATTERN = '[a-z0-9\\-]{1,32}';

/** Whether `code` is one a proxy's check fails with. */
export const isCheckCode = (code: unknown): code is CheckCode => typeof code === 'string' && Object.hasOwn(CHECKS, code);

/** Why a check failed, in plain words (its message key); an unknown code reads as the proxy being unreachable. */
export function checkKey(code: unknown): CheckKey {
  return isCheckCode(code) ? CHECKS[code] : CHECKS['proxy-check-unreachable'];
}

/** The flag of a two-letter country code, as the emoji its regional letters make; nothing for anything else. */
export function flagOf(code: string | null | undefined): string {
  if (!code || !/^[A-Za-z]{2}$/.test(code)) return '';
  return String.fromCodePoint(...[...code.toUpperCase()].map((c) => 0x1f1e6 + c.charCodeAt(0) - 65));
}

/** Where a check found the exit, as a person reads it in `locale`: its flag, city, region and country (each name once),
 * and its address. */
export function exitText(exit: Exit, locale: string): string {
  const names = [exit.city, exit.region, countryName(exit.country, exit.country_name, locale) ?? exit.country].filter((n): n is string => !!n);
  const place = names.filter((n, i) => names.indexOf(n) === i).join(', ');
  return [`${flagOf(exit.country)} ${place}`.trim(), exit.ip].join(' · ');
}

/** A time zone as a person reads it: its city ("America/Sao_Paulo" reads "Sao Paulo"). */
export const zoneName = (zone: string) => (zone.split('/').pop() ?? zone).replace(/_/g, ' ');

/** `code` as the browser names it in `locale` (the page's language), or null for a code it has no name for. */
function browserName(type: 'language' | 'region', code: string, locale: string): string | null {
  try {
    return new Intl.DisplayNames([locale], { type, fallback: 'none' }).of(code) ?? null;
  } catch {
    // Not a code the browser knows.
    return null;
  }
}

/** A language tag as the browser names it in `locale`, else by its name in `names` (the admin's list, in English), else
 * the tag itself. */
export const languageName = (tag: string, locale: string, names: { tag: string; name: string }[] = []): string =>
  browserName('language', tag, locale) ?? names.find((l) => l.tag === tag)?.name ?? tag;

/** A country by its two-letter code as the browser names it in `locale`, else the admin's name for it (`name`, in
 * English); null with neither. */
export const countryName = (code: string | null, name: string | null, locale: string): string | null =>
  (code && /^[A-Za-z]{2}$/.test(code) ? browserName('region', code.toUpperCase(), locale) : null) ?? name;

/** A proxy's address as a list shows it: never its password. */
export const addressText = (proxy: Pick<Proxy, 'host' | 'port'>) => `${proxy.host}:${proxy.port}`;
