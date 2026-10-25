// The second step at sign-in, as admin/src/second_step.rs serves it: what the admin answers, the codes people type,
// and the passkey ceremonies, whose binary fields travel as base64url in JSON and as bytes to the browser's WebAuthn.

import { api } from './api.ts';

/** What the second step holds, as `GET /api/security` answers. */
export interface SecondStep {
  app: boolean;
  /** Recovery codes not used yet. */
  recovery_codes: number;
  passkeys: PasskeyInfo[];
  /** The admin's address can have passkeys: its host is a domain name, not an IP address. */
  passkeys_possible: boolean;
}
export interface PasskeyInfo {
  id: string;
  /** Unix seconds. */
  created: number;
}
/** What `POST /api/login` answers once the password is right and the second step is on. */
export interface PendingStep {
  step: string;
  app: boolean;
  passkey: boolean;
}

/** A code as typed for the app: its six digits, spaces and dashes between them left out. */
export function appCode(text: string): string | null {
  const digits = text.replace(/[\s-]/g, '');
  return /^\d{6}$/.test(digits) ? digits : null;
}

export function toBase64url(bytes: ArrayBuffer | Uint8Array): string {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let binary = '';
  for (const byte of view) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function fromBase64url(text: string): Uint8Array<ArrayBuffer> {
  const base64 = text.replace(/-/g, '+').replace(/_/g, '/');
  const binary = atob(base64 + '='.repeat((4 - (base64.length % 4)) % 4));
  return Uint8Array.from(binary, (c) => c.charCodeAt(0));
}

type Json = Record<string, unknown>;
interface Described {
  type: 'public-key';
  id: string;
  transports?: AuthenticatorTransport[];
}

const credentials = (list: unknown): PublicKeyCredentialDescriptor[] =>
  ((list as Described[] | undefined) ?? []).map((c) => ({ ...c, id: fromBase64url(c.id) }));

/** The options for `navigator.credentials.create`, from what the admin answered (webauthn-rs's JSON). */
export function creationOptions(options: { publicKey: Json }): PublicKeyCredentialCreationOptions {
  const key = options.publicKey;
  const user = key.user as Json;
  return {
    ...(key as unknown as PublicKeyCredentialCreationOptions),
    challenge: fromBase64url(key.challenge as string),
    user: { ...(user as unknown as PublicKeyCredentialUserEntity), id: fromBase64url(user.id as string) },
    excludeCredentials: credentials(key.excludeCredentials),
  };
}

/** The options for `navigator.credentials.get`, from what the admin answered. */
export function requestOptions(options: { publicKey: Json }): PublicKeyCredentialRequestOptions {
  const key = options.publicKey;
  return {
    ...(key as unknown as PublicKeyCredentialRequestOptions),
    challenge: fromBase64url(key.challenge as string),
    allowCredentials: credentials(key.allowCredentials),
  };
}

/** A new passkey as the admin reads it. The extensions' outputs are left out: the admin uses none. */
export function registrationJSON(credential: PublicKeyCredential): Json {
  const response = credential.response as AuthenticatorAttestationResponse;
  return {
    id: credential.id,
    rawId: toBase64url(credential.rawId),
    type: credential.type,
    response: {
      attestationObject: toBase64url(response.attestationObject),
      clientDataJSON: toBase64url(response.clientDataJSON),
      transports: response.getTransports?.() ?? [],
    },
  };
}

/** A passkey's signature as the admin reads it. */
export function assertionJSON(credential: PublicKeyCredential): Json {
  const response = credential.response as AuthenticatorAssertionResponse;
  return {
    id: credential.id,
    rawId: toBase64url(credential.rawId),
    type: credential.type,
    response: {
      authenticatorData: toBase64url(response.authenticatorData),
      clientDataJSON: toBase64url(response.clientDataJSON),
      signature: toBase64url(response.signature),
      userHandle: response.userHandle ? toBase64url(response.userHandle) : null,
    },
  };
}

/**
 * Why the browser refused a passkey: `untrusted` when it offers none at this address (a browser offers passkeys only
 * on a site whose certificate it trusts, and the admin's own certificate, reached through a tunnel, is not one),
 * `cancelled` when the person closed its dialog or let it time out, `added` when this device holds one already.
 */
export type PasskeyRefusal = 'untrusted' | 'cancelled' | 'added' | 'other';

export function passkeyRefusal(error: unknown): PasskeyRefusal {
  if (!(error instanceof DOMException)) return 'other';
  // Chrome refuses on a page with a certificate error at once, with a NotAllowedError that says why; other browsers
  // refuse an address they offer no passkeys at with a SecurityError.
  if (error.name === 'SecurityError' || /certificate|TLS/i.test(error.message)) return 'untrusted';
  if (error.name === 'NotAllowedError' || error.name === 'AbortError') return 'cancelled';
  if (error.name === 'InvalidStateError') return 'added';
  return 'other';
}

/** Whether this browser has WebAuthn at all here: a secure page, with the API. */
export function passkeysInBrowser(): boolean {
  return typeof PublicKeyCredential === 'function' && globalThis.isSecureContext === true;
}

/** The text of the recovery codes' download. */
export function recoveryFile(codes: string[], origin: string): string {
  return [
    `easi.run recovery codes for ${origin}`,
    'Sign in with your password, then enter one of these codes in place of a code from your authenticator app or a',
    'passkey. Each code works once.',
    '',
    ...codes,
    '',
  ].join('\n');
}

/** Adds a passkey to this device for the admin; the recovery codes when it turned the second step on. */
export async function addPasskey(password: string): Promise<string[] | null> {
  const { options } = await api<{ options: { publicKey: Json } }>('POST', '/api/security/passkeys', { password });
  const created = (await navigator.credentials.create({ publicKey: creationOptions(options) })) as PublicKeyCredential;
  const answer = { credential: registrationJSON(created) };
  return (await api<{ recovery_codes: string[] | null }>('POST', '/api/security/passkeys/finish', answer)).recovery_codes;
}

/** Signs in with a passkey of this device: the login's CSRF token. */
export async function signInWithPasskey(): Promise<string> {
  const { ceremony, options } = await api<{ ceremony: string; options: { publicKey: Json } }>('POST', '/api/login/passkey');
  const signed = (await navigator.credentials.get({ publicKey: requestOptions(options) })) as PublicKeyCredential;
  const answer = { ceremony, credential: assertionJSON(signed) };
  return (await api<{ csrf: string }>('POST', '/api/login/passkey/finish', answer)).csrf;
}
