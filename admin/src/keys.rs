//! The keys that keep saved profiles unreadable at rest (see `profiles`).
//!
//! The key-encryption key (KEK) is Argon2id over the administrator's password (the parameters the password hash uses)
//! with a random salt of the installation's own (`key-salt`, beside the password hash), so it is independent of that
//! hash. It is derived whenever the password is checked at a login (or is set), and held only in the admin's memory:
//! never written, never logged, gone when the admin stops. Each profile has a random 256-bit data key, stored only
//! wrapped by the KEK with XChaCha20-Poly1305, the profile's id as associated data, so a wrapped key moved to another
//! profile opens nothing. A profile's archive is sealed with its data key by XChaCha20-Poly1305 in the STREAM
//! construction (`aead::stream`, big-endian 32-bit counter, the last chunk flagged): chunks of [`CHUNK`] bytes, the
//! profile's id as every chunk's associated data, after a random nonce prefix drawn for each save. A chunk changed,
//! moved, dropped or added, or an archive cut at any byte, fails to open.

use chacha20poly1305::aead::stream::{DecryptorBE32, EncryptorBE32};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;
use zeroize::Zeroizing;

pub const SALT: usize = 16;
const KEY: usize = 32;
const NONCE: usize = 24;
const TAG: usize = 16;
/// A wrapped data key: its nonce, then the key sealed with its tag.
pub const WRAPPED: usize = NONCE + KEY + TAG;
/// The plaintext of one archive chunk; every chunk but the last holds exactly this much.
pub const CHUNK: usize = 64 * 1024;
/// One sealed chunk at most: a whole chunk and its tag.
pub const UNIT: usize = CHUNK + TAG;
/// The nonce prefix an archive starts with: XChaCha20's 24-byte nonce less STREAM's counter and last-chunk flag.
pub const PREFIX: usize = NONCE - 5;

/// The key-encryption key, wiped from memory when dropped.
#[derive(Clone)]
pub struct Kek(Zeroizing<[u8; KEY]>);

/// Never shows the key itself.
impl std::fmt::Debug for Kek {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Kek(..)")
    }
}

/// Compared in constant time, for the tests of whoever holds one.
impl PartialEq for Kek {
    fn eq(&self, other: &Kek) -> bool {
        use subtle::ConstantTimeEq;
        self.0.ct_eq(&*other.0).into()
    }
}

impl Eq for Kek {}

/// A profile's data key, wiped from memory when dropped.
#[derive(Clone)]
pub struct DataKey(Zeroizing<[u8; KEY]>);

impl DataKey {
    pub fn random() -> DataKey {
        let mut key = Zeroizing::new([0u8; KEY]);
        rand::rng().fill_bytes(&mut *key);
        DataKey(key)
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&*self.0).into())
    }
}

pub fn new_salt() -> [u8; SALT] {
    rand::random()
}

/// The KEK of `password` (slow: an Argon2id hash, on a blocking thread).
pub fn derive(password: &str, salt: &[u8; SALT]) -> Kek {
    let mut key = Zeroizing::new([0u8; KEY]);
    argon2::Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut *key)
        .expect("a 16-byte salt and a 32-byte output are within Argon2's bounds");
    Kek(key)
}

/// `key`, wrapped by `kek` for the profile `id`.
pub fn wrap(kek: &Kek, id: &str, key: &DataKey) -> [u8; WRAPPED] {
    let nonce: [u8; NONCE] = rand::random();
    let cipher = XChaCha20Poly1305::new((&*kek.0).into());
    let sealed = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: &*key.0, aad: id.as_bytes() })
        .expect("a 32-byte message seals");
    let mut wrapped = [0u8; WRAPPED];
    wrapped[..NONCE].copy_from_slice(&nonce);
    wrapped[NONCE..].copy_from_slice(&sealed);
    wrapped
}

/// The data key `wrapped` holds for the profile `id`, unless `kek` is not the one that wrapped it (another password)
/// or it was changed.
pub fn unwrap(kek: &Kek, id: &str, wrapped: &[u8]) -> Option<DataKey> {
    if wrapped.len() != WRAPPED {
        return None;
    }
    let cipher = XChaCha20Poly1305::new((&*kek.0).into());
    let opened = Zeroizing::new(
        cipher
            .decrypt(XNonce::from_slice(&wrapped[..NONCE]), Payload { msg: &wrapped[NONCE..], aad: id.as_bytes() })
            .ok()?,
    );
    let mut key = Zeroizing::new([0u8; KEY]);
    key.copy_from_slice(opened.get(..KEY)?);
    Some(DataKey(key))
}

/// Seals an archive as it streams: [`Sealer::push`] takes its bytes in any pieces, and every whole chunk is sealed as
/// soon as a byte after it arrives, so the last one, whatever its length, is sealed by [`Sealer::finish`].
pub struct Sealer {
    stream: EncryptorBE32<XChaCha20Poly1305>,
    pending: Zeroizing<Vec<u8>>,
    aad: Vec<u8>,
}

impl Sealer {
    /// A sealer for the profile `id`, and the nonce prefix the archive starts with.
    pub fn new(key: &DataKey, id: &str) -> (Sealer, [u8; PREFIX]) {
        let prefix: [u8; PREFIX] = rand::random();
        let stream = EncryptorBE32::from_aead(key.cipher(), (&prefix).into());
        (Sealer { stream, pending: Zeroizing::new(Vec::with_capacity(CHUNK)), aad: id.as_bytes().to_vec() }, prefix)
    }

    /// Takes `bytes`, appending to `sealed` every chunk they complete.
    pub fn push(&mut self, mut bytes: &[u8], sealed: &mut Vec<u8>) {
        while !bytes.is_empty() {
            if self.pending.len() == CHUNK {
                let chunk = Payload { msg: &self.pending[..], aad: &self.aad };
                sealed.extend(self.stream.encrypt_next(chunk).expect("a chunk seals"));
                self.pending.clear();
            }
            let take = bytes.len().min(CHUNK - self.pending.len());
            self.pending.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
        }
    }

    /// Appends the last chunk to `sealed`.
    pub fn finish(self, sealed: &mut Vec<u8>) {
        let last = Payload { msg: &self.pending[..], aad: &self.aad };
        sealed.extend(self.stream.encrypt_last(last).expect("a chunk seals"));
    }
}

/// Opens an archive's chunks in order.
pub struct Opener {
    stream: DecryptorBE32<XChaCha20Poly1305>,
    aad: Vec<u8>,
}

impl Opener {
    /// An opener for the archive of the profile `id` that starts with `prefix`.
    pub fn new(key: &DataKey, id: &str, prefix: &[u8; PREFIX]) -> Opener {
        Opener { stream: DecryptorBE32::from_aead(key.cipher(), prefix.into()), aad: id.as_bytes().to_vec() }
    }

    /// The plaintext of the next sealed chunk, `last` when the archive ends with it, or none when it does not open.
    pub fn open(&mut self, sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        self.stream.decrypt_next(Payload { msg: sealed, aad: &self.aad }).ok().map(Zeroizing::new)
    }

    /// The plaintext of the archive's last chunk, or none when it does not open as the last.
    pub fn open_last(self, sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        self.stream.decrypt_last(Payload { msg: sealed, aad: &self.aad }).ok().map(Zeroizing::new)
    }
}

/// The longest sealed archive of at most `plain` bytes: its prefix, and every chunk with its tag.
pub fn sealed_length(plain: u64) -> u64 {
    PREFIX as u64 + plain + (plain / CHUNK as u64 + 1) * TAG as u64
}

/// The sealed chunks of an archive of `length` bytes (its prefix included), each as `(offset, length)`, or none when no
/// archive is that long: every chunk but the last is a whole [`UNIT`], and the last holds its tag at least. The caller
/// bounds `length` first (see [`sealed_length`]).
pub fn units(length: u64) -> Option<Vec<(u64, usize)>> {
    let body = length.checked_sub(PREFIX as u64)?;
    let count = body.div_ceil(UNIT as u64).max(1);
    let last = body - (count - 1) * UNIT as u64;
    if last < TAG as u64 {
        return None;
    }
    Some(
        (0..count)
            .map(|i| (PREFIX as u64 + i * UNIT as u64, if i + 1 == count { last as usize } else { UNIT }))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sealed archive of `plain`, pushed in pieces of `piece` bytes.
    fn seal(key: &DataKey, id: &str, plain: &[u8], piece: usize) -> Vec<u8> {
        let (mut sealer, prefix) = Sealer::new(key, id);
        let mut sealed = prefix.to_vec();
        for part in plain.chunks(piece.max(1)) {
            sealer.push(part, &mut sealed);
        }
        sealer.finish(&mut sealed);
        sealed
    }

    fn open(key: &DataKey, id: &str, sealed: &[u8]) -> Option<Vec<u8>> {
        let units = units(sealed.len() as u64)?;
        let mut opener = Opener::new(key, id, sealed[..PREFIX].try_into().unwrap());
        let mut plain = Vec::new();
        let (last, rest) = units.split_last().unwrap();
        for &(at, length) in rest {
            plain.extend_from_slice(&opener.open(&sealed[at as usize..at as usize + length])?);
        }
        plain.extend_from_slice(&opener.open_last(&sealed[last.0 as usize..last.0 as usize + last.1])?);
        Some(plain)
    }

    #[test]
    fn a_data_key_unwraps_only_with_the_same_password_and_profile() {
        let salt = new_salt();
        let kek = derive("correct horse battery", &salt);
        assert_eq!(kek, derive("correct horse battery", &salt), "the same password derives the same KEK");
        assert_ne!(kek, derive("correct horse battery", &new_salt()), "another installation's salt another");
        let key = DataKey::random();
        let wrapped = wrap(&kek, "0123456789abcdef", &key);
        let unwrapped = unwrap(&kek, "0123456789abcdef", &wrapped).expect("the same KEK unwraps it");
        assert_eq!(*unwrapped.0, *key.0);
        assert!(unwrap(&derive("wrong horse battery", &salt), "0123456789abcdef", &wrapped).is_none());
        assert!(unwrap(&kek, "fedcba9876543210", &wrapped).is_none(), "another profile's id opens nothing");
        let mut changed = wrapped;
        changed[WRAPPED - 1] ^= 1;
        assert!(unwrap(&kek, "0123456789abcdef", &changed).is_none());
        assert!(unwrap(&kek, "0123456789abcdef", &wrapped[..WRAPPED - 1]).is_none());
        assert_ne!(wrap(&kek, "0123456789abcdef", &key), wrapped, "each wrap draws its own nonce");
    }

    #[test]
    fn an_archive_opens_whole_whatever_its_length_and_pieces() {
        let key = DataKey::random();
        let plain: Vec<u8> = (0..3 * CHUNK + 7).map(|i| (i * 31 % 251) as u8).collect();
        for length in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, 2 * CHUNK, 3 * CHUNK + 7] {
            for piece in [1000, CHUNK, 3 * CHUNK + 7] {
                let sealed = seal(&key, "id", &plain[..length], piece);
                assert_eq!(open(&key, "id", &sealed).as_deref(), Some(&plain[..length]), "{length} by {piece}");
            }
        }
        let sealed = seal(&key, "id", &plain, CHUNK);
        assert!(open(&DataKey::random(), "id", &sealed).is_none(), "another key");
        assert!(open(&key, "other", &sealed).is_none(), "another profile's id");
        assert_ne!(seal(&key, "id", &plain, CHUNK), sealed, "each save draws its own nonce prefix");
    }

    #[test]
    fn an_archive_cut_changed_or_reordered_does_not_open() {
        let key = DataKey::random();
        let plain = vec![7u8; 2 * CHUNK + 100];
        let sealed = seal(&key, "id", &plain, CHUNK);
        // Cut at a chunk's end (the earlier chunk is then taken for the last), and anywhere else.
        for cut in [PREFIX + UNIT, PREFIX + 2 * UNIT, sealed.len() - 1, PREFIX + 10, PREFIX] {
            assert!(open(&key, "id", &sealed[..cut]).is_none(), "cut at {cut}");
        }
        for at in [0, PREFIX, PREFIX + UNIT + 5, sealed.len() - 1] {
            let mut changed = sealed.clone();
            changed[at] ^= 0x80;
            assert!(open(&key, "id", &changed).is_none(), "byte {at} changed");
        }
        let mut swapped = sealed.clone();
        swapped[PREFIX..PREFIX + 2 * UNIT].rotate_left(UNIT);
        assert!(open(&key, "id", &swapped).is_none(), "two chunks swapped");
        let mut longer = sealed.clone();
        longer.extend_from_slice(&sealed[PREFIX..PREFIX + UNIT]);
        assert!(open(&key, "id", &longer).is_none(), "a chunk added");
    }

    #[test]
    fn every_archive_length_splits_into_its_chunks_or_none() {
        assert_eq!(units(PREFIX as u64 + TAG as u64), Some(vec![(PREFIX as u64, TAG)]), "an empty archive");
        assert_eq!(units(PREFIX as u64 + UNIT as u64), Some(vec![(PREFIX as u64, UNIT)]));
        let two = units((PREFIX + UNIT + TAG) as u64).unwrap();
        assert_eq!(two, vec![(PREFIX as u64, UNIT), ((PREFIX + UNIT) as u64, TAG)]);
        for short in [0, PREFIX, PREFIX + TAG - 1, PREFIX + UNIT + TAG - 1] {
            assert!(units(short as u64).is_none(), "{short}");
        }
        let key = DataKey::random();
        for plain in [0, 1, CHUNK, CHUNK + 1, 3 * CHUNK] {
            let sealed = seal(&key, "id", &vec![1; plain], CHUNK);
            assert!(sealed.len() as u64 <= sealed_length(plain as u64), "{plain}");
        }
    }
}
