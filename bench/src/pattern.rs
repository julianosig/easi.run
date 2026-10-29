//! The bytes of the fixture host's `/big` downloads, which the download tests check each one they receive against.

/// Byte `i` of every `/big` download.
pub fn big_byte(i: u64) -> u8 {
    (i.wrapping_mul(7) ^ (i >> 8) ^ (i >> 24)) as u8
}
