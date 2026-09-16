//! Syncthing device IDs: SHA-256 of the certificate DER, base32 encoded,
//! with a Luhn mod-32 check character per 13-character group, chunked in 7s.
//! Mirrors `lib/protocol/deviceid.go` and `luhn.go` in Syncthing.

use sha2::{Digest, Sha256};

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn from_cert_der(cert_der: &[u8]) -> String {
    let hash = Sha256::digest(cert_der);
    let b32 = base32_nopad(hash.as_slice());
    debug_assert_eq!(b32.len(), 52);

    let mut with_check = String::with_capacity(56);
    for group in b32.as_bytes().chunks(13) {
        with_check.push_str(std::str::from_utf8(group).unwrap());
        with_check.push(luhn32(group) as char);
    }
    with_check
        .as_bytes()
        .chunks(7)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("-")
}

fn base32_nopad(data: &[u8]) -> String {
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &byte in data {
        buffer = (buffer << 8) | byte as u32;
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

fn luhn32(group: &[u8]) -> u8 {
    const N: u32 = 32;
    let mut factor = 1;
    let mut sum = 0;
    for &c in group {
        let codepoint = ALPHABET.iter().position(|&a| a == c).unwrap() as u32;
        let mut addend = factor * codepoint;
        factor = if factor == 2 { 1 } else { 2 };
        addend = addend / N + addend % N;
        sum += addend;
    }
    let check = (N - sum % N) % N;
    ALPHABET[check as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_matches_rfc4648() {
        assert_eq!(base32_nopad(b"foobar"), "MZXW6YTBOI");
    }

    #[test]
    fn device_id_shape() {
        let id = from_cert_der(b"not a real certificate");
        assert_eq!(id.len(), 63);
        assert_eq!(id.matches('-').count(), 7);
    }
}
