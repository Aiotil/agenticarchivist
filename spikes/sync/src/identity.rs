//! Device identities.
//!
//! Syncthing 2.x uses Ed25519 certificates. Ed25519 signatures are
//! deterministic, so a certificate built from a fixed key and fixed fields is
//! byte-identical every time. That lets an invite carry only a 32-byte seat
//! secret: sender and recipient both derive the same certificate, and so the
//! same device ID, without exchanging anything else.

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
#[cfg(test)]
use ed25519_dalek::Signer;
use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use ed25519_dalek::{SigningKey, Verifier, VerifyingKey};
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SerialNumber, date_time_ymd,
};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::deviceid;

pub struct DeviceIdentity {
    pub signing_key: SigningKey,
    pub cert_der: Vec<u8>,
    pub device_id: String,
}

impl DeviceIdentity {
    /// Derive a Syncthing identity from an invite seat secret.
    pub fn from_seat_secret(secret: &[u8; 32]) -> Result<Self> {
        let seed: [u8; 32] = Sha256::new()
            .chain_update(b"agenticarchivist/seat-device-key/v1")
            .chain_update(secret)
            .finalize()
            .as_slice()
            .try_into()?;
        let signing_key = SigningKey::from_bytes(&seed);
        let serial: [u8; 8] = Sha256::new()
            .chain_update(b"agenticarchivist/seat-cert-serial/v1")
            .chain_update(secret)
            .finalize()
            .as_slice()[..8]
            .try_into()?;
        let cert_der = self_signed_cert(&signing_key, serial)?;
        let device_id = deviceid::from_cert_der(&cert_der);
        Ok(Self {
            signing_key,
            cert_der,
            device_id,
        })
    }

    /// Load the identity Syncthing generated in a home directory.
    pub fn load_from_home(home: &Path) -> Result<Self> {
        let key_pem = std::fs::read_to_string(home.join("key.pem"))?;
        let signing_key = SigningKey::from_pkcs8_pem(&key_pem)
            .map_err(|e| anyhow!("key.pem is not an Ed25519 PKCS#8 key: {e}"))?;
        let cert_pem = std::fs::read_to_string(home.join("cert.pem"))?;
        let cert_der = pem_body(&cert_pem, "CERTIFICATE")?;
        let device_id = deviceid::from_cert_der(&cert_der);
        Ok(Self {
            signing_key,
            cert_der,
            device_id,
        })
    }

    /// Write `cert.pem` and `key.pem` where Syncthing expects them.
    pub fn write_to_home(&self, home: &Path) -> Result<()> {
        std::fs::create_dir_all(home)?;
        let key_der = self
            .signing_key
            .to_pkcs8_der()
            .map_err(|e| anyhow!("encoding key: {e}"))?;
        std::fs::write(home.join("key.pem"), pem("PRIVATE KEY", key_der.as_bytes()))?;
        std::fs::write(home.join("cert.pem"), pem("CERTIFICATE", &self.cert_der))?;
        Ok(())
    }

    #[cfg(test)]
    pub fn sign(&self, message: &[u8]) -> String {
        STANDARD.encode(self.signing_key.sign(message).to_bytes())
    }
}

/// Verify a signature against the Ed25519 key inside a certificate.
pub fn verify_with_cert(cert_der: &[u8], message: &[u8], signature_b64: &str) -> Result<()> {
    let public = ed25519_public_key_from_cert(cert_der)?;
    let key = VerifyingKey::from_bytes(&public)?;
    let sig_bytes: [u8; 64] = STANDARD
        .decode(signature_b64)?
        .try_into()
        .map_err(|_| anyhow!("signature is not 64 bytes"))?;
    key.verify(message, &ed25519_dalek::Signature::from_bytes(&sig_bytes))?;
    Ok(())
}

fn self_signed_cert(key: &SigningKey, serial: [u8; 8]) -> Result<Vec<u8>> {
    let pkcs8 = key
        .to_pkcs8_der()
        .map_err(|e| anyhow!("encoding key: {e}"))?;
    let key_pair = KeyPair::try_from(pkcs8.as_bytes()).context("loading key into rcgen")?;

    // Same subject and extensions Syncthing uses for its own certificates.
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::OrganizationName, "Syncthing");
    dn.push(DnType::OrganizationalUnitName, "Automatically Generated");
    dn.push(DnType::CommonName, "syncthing");
    params.distinguished_name = dn;
    params.serial_number = Some(SerialNumber::from_slice(&serial));
    params.not_before = date_time_ymd(2026, 1, 1);
    params.not_after = date_time_ymd(2046, 1, 1);
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    let cert = params.self_signed(&key_pair)?;
    Ok(cert.der().to_vec())
}

/// Extract the 32-byte Ed25519 key from a certificate's SubjectPublicKeyInfo.
/// The SPKI for Ed25519 is always `30 2a 30 05 06 03 2b 65 70 03 21 00 <32 bytes>`.
fn ed25519_public_key_from_cert(cert_der: &[u8]) -> Result<[u8; 32]> {
    const SPKI_PREFIX: [u8; 12] = [
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    ];
    let pos = cert_der
        .windows(SPKI_PREFIX.len())
        .position(|w| w == SPKI_PREFIX)
        .ok_or_else(|| anyhow!("no Ed25519 public key in certificate"))?;
    let start = pos + SPKI_PREFIX.len();
    Ok(cert_der[start..start + 32].try_into()?)
}

fn pem(label: &str, der: &[u8]) -> String {
    let b64 = STANDARD.encode(der);
    let body = b64
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    format!("-----BEGIN {label}-----\n{body}\n-----END {label}-----\n")
}

fn pem_body(text: &str, label: &str) -> Result<Vec<u8>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let start = text
        .find(&begin)
        .ok_or_else(|| anyhow!("missing {begin}"))?
        + begin.len();
    let stop = text.find(&end).ok_or_else(|| anyhow!("missing {end}"))?;
    let b64: String = text[start..stop]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    Ok(STANDARD.decode(b64)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seat_identity_is_deterministic() {
        let secret = [7u8; 32];
        let a = DeviceIdentity::from_seat_secret(&secret).unwrap();
        let b = DeviceIdentity::from_seat_secret(&secret).unwrap();
        assert_eq!(a.cert_der, b.cert_der);
        assert_eq!(a.device_id, b.device_id);
        let c = DeviceIdentity::from_seat_secret(&[8u8; 32]).unwrap();
        assert_ne!(a.device_id, c.device_id);
    }

    #[test]
    fn signatures_verify_against_certificate() {
        let id = DeviceIdentity::from_seat_secret(&[1u8; 32]).unwrap();
        let sig = id.sign(b"hello");
        verify_with_cert(&id.cert_der, b"hello", &sig).unwrap();
        assert!(verify_with_cert(&id.cert_der, b"tampered", &sig).is_err());
    }
}
