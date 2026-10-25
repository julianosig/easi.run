//! One-time codes from an authenticator app (RFC 6238, TOTP): HMAC-SHA-1 of the number of 30-second steps since the
//! Unix epoch, cut to 6 digits by RFC 4226's dynamic truncation, which is what every authenticator app makes from an
//! `otpauth://totp/` address with no other parameters. A code is taken for its own step or one either side, which
//! covers a phone's clock a little off and a code typed as it changed. The caller keeps the step it took last and
//! passes it back: a code of that step or an earlier one is refused, so each code signs in once, and an older code
//! seen over someone's shoulder never works after a newer one.

use rand::RngCore;
use subtle::ConstantTimeEq;

/// The secret's length: 160 bits, the HMAC-SHA-1 block RFC 4226 recommends.
pub const SECRET_BYTES: usize = 20;
const STEP_SECONDS: u64 = 30;
const DIGITS: usize = 6;
/// What the app shows the code as, before the account's name.
const ISSUER: &str = "easi.run";

pub fn new_secret() -> [u8; SECRET_BYTES] {
    let mut secret = [0u8; SECRET_BYTES];
    rand::rng().fill_bytes(&mut secret);
    secret
}

/// RFC 4648 base32 without padding, the form authenticator apps take a secret in (typed or in the address).
pub fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// The address an authenticator app adds the secret from, as its QR code carries it.
pub fn uri(secret: &[u8]) -> String {
    format!("otpauth://totp/{ISSUER}:admin?secret={}&issuer={ISSUER}", base32(secret))
}

/// The step of a Unix time.
pub fn step(unix_seconds: u64) -> u64 {
    unix_seconds / STEP_SECONDS
}

/// The code of `counter` with `digits` digits (RFC 4226, section 5.3).
fn code(secret: &[u8], counter: u64, digits: u32) -> String {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let mac = ring::hmac::sign(&key, &counter.to_be_bytes());
    let mac = mac.as_ref();
    let offset = usize::from(mac[mac.len() - 1] & 0x0f);
    let truncated = u32::from_be_bytes([mac[offset], mac[offset + 1], mac[offset + 2], mac[offset + 3]]) & 0x7fff_ffff;
    format!("{:0width$}", truncated % 10u32.pow(digits), width = digits as usize)
}

/// The code an app shows at `unix_seconds`.
#[cfg(test)]
pub fn code_at(secret: &[u8], unix_seconds: u64) -> String {
    code(secret, step(unix_seconds), DIGITS as u32)
}

/// The step `code` was made for, when it is `now`'s or one either side and later than `last` (the step taken last,
/// if any): the step to keep as the new `last`. Anything but exactly six digits is refused.
pub fn accept(secret: &[u8], code: &str, now: u64, last: Option<u64>) -> Option<u64> {
    if code.len() != DIGITS || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut found = None;
    // Every candidate is compared, in constant time, whichever matches.
    for candidate in step(now).saturating_sub(1)..=step(now) + 1 {
        let expected = self::code(secret, candidate, DIGITS as u32);
        if bool::from(expected.as_bytes().ct_eq(code.as_bytes())) && last.is_none_or(|last| candidate > last) {
            found = Some(candidate);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The secret of RFC 6238's SHA-1 vectors (Appendix B) and RFC 4226's (Appendix D).
    const RFC_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn codes_match_rfc_6238_and_rfc_4226() {
        for (time, expected) in [
            (59, "94287082"),
            (1_111_111_109, "07081804"),
            (1_111_111_111, "14050471"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
            (20_000_000_000, "65353130"),
        ] {
            assert_eq!(code(RFC_SECRET, step(time), 8), expected, "at {time}");
            // Six digits are the last six of the eight.
            assert_eq!(code(RFC_SECRET, step(time), 6), expected[2..], "at {time}");
        }
        let hotp = ["755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583", "399871", "520489"];
        for (counter, expected) in hotp.iter().enumerate() {
            assert_eq!(code(RFC_SECRET, counter as u64, 6), *expected, "counter {counter}");
        }
    }

    #[test]
    fn base32_matches_rfc_4648_without_padding() {
        for (bytes, expected) in [
            (&b""[..], ""),
            (b"f", "MY"),
            (b"fo", "MZXQ"),
            (b"foo", "MZXW6"),
            (b"foob", "MZXW6YQ"),
            (b"fooba", "MZXW6YTB"),
            (b"foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32(bytes), expected);
        }
        assert_eq!(base32(RFC_SECRET), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(
            uri(RFC_SECRET),
            "otpauth://totp/easi.run:admin?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=easi.run"
        );
    }

    #[test]
    fn a_code_is_taken_one_step_either_side_and_only_after_the_last_taken() {
        let now = 1_111_111_111;
        let at = |offset: i64| code(RFC_SECRET, step(now).checked_add_signed(offset).unwrap(), 6);
        assert_eq!(accept(RFC_SECRET, &at(0), now, None), Some(step(now)));
        assert_eq!(accept(RFC_SECRET, &at(-1), now, None), Some(step(now) - 1));
        assert_eq!(accept(RFC_SECRET, &at(1), now, None), Some(step(now) + 1));
        assert_eq!(accept(RFC_SECRET, &at(-2), now, None), None, "too old");
        assert_eq!(accept(RFC_SECRET, &at(2), now, None), None, "too far ahead");
        // Taken once, the same code is refused, and so is an older one; a newer one is taken.
        assert_eq!(accept(RFC_SECRET, &at(0), now, Some(step(now))), None, "a replay");
        assert_eq!(accept(RFC_SECRET, &at(-1), now, Some(step(now))), None, "older than the last taken");
        assert_eq!(accept(RFC_SECRET, &at(1), now, Some(step(now))), Some(step(now) + 1));
        for malformed in ["", "12345", "1234567", "12345a", " 123456", "１２３４５６"] {
            assert_eq!(accept(RFC_SECRET, malformed, now, None), None, "{malformed:?}");
        }
        assert_eq!(accept(&new_secret(), &at(0), now, None), None, "another secret's code");
    }
}
