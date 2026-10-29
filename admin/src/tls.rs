//! The HTTPS identity: `tls-cert.pem` and `tls-key.pem` in the state directory, the operator's own or a self-signed
//! certificate for `ADMIN_HOSTNAMES` that the admin generates when there is none. Its SHA-256 fingerprint is logged so
//! the person can verify it in the browser.
//!
//! The admin records the names and the fingerprint of a certificate it generated in `tls-names.json`, and generates a
//! new one when those names are no longer `ADMIN_HOSTNAMES`, or when the certificate in place is still the one a cut
//! short generation was replacing. A certificate with any other fingerprint (one the operator put there) is never
//! replaced: without its key, the start stops and says so.

use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::auth::hex;

/// A certificate the admin generated: its names, and the SHA-256 of its DER in hex; and the SHA-256 of the one it
/// replaced, if any, which stays the admin's own (and is replaced again) should the new one never take its place.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Generated {
    hostnames: Vec<String>,
    sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replacing: Option<String>,
}

/// The mode of a new certificate the admin writes: public, as `std::fs::write` made it under Docker's default umask
/// (022). One it replaces keeps the mode it had.
const CERT_MODE: u32 = 0o644;

pub fn server_config(state: &Path, hostnames: &[String]) -> Result<Arc<rustls::ServerConfig>, String> {
    let cert_path = state.join("tls-cert.pem");
    let key_path = state.join("tls-key.pem");
    let names_path = state.join("tls-names.json");
    if !cert_path.exists() {
        generate(&cert_path, &key_path, &names_path, hostnames)?;
    } else if !key_path.exists() {
        // The admin's own certificate whose key went (a generation cut short): made again. An operator's is theirs to
        // complete, never replaced.
        if !own(&cert_path, &names_path) {
            return Err(format!("{} has no tls-key.pem beside it; put its key there", cert_path.display()));
        }
        generate(&cert_path, &key_path, &names_path, hostnames)?;
    } else if let Some(why) = outdated(&cert_path, &names_path, hostnames) {
        tracing::info!(?hostnames, "{why}; generating a new self-signed certificate");
        generate(&cert_path, &key_path, &names_path, hostnames)?;
    }
    let cert_pem = std::fs::read(&cert_path).map_err(|e| e.to_string())?;
    let key_pem = std::fs::read(&key_path).map_err(|e| e.to_string())?;
    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls::pki_types::CertificateDer::pem_slice_iter(&cert_pem)
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
    let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(&key_pem).map_err(|e| e.to_string())?;
    if let Some(first) = certs.first() {
        let fingerprint: Vec<String> = Sha256::digest(first.as_ref()).iter().map(|b| format!("{b:02X}")).collect();
        tracing::info!(fingerprint = %fingerprint.join(":"), "HTTPS certificate (SHA-256)");
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| e.to_string())?;
    // HTTP/2 for a client that asks for it (see `server`), else HTTP/1.1.
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// The SHA-256 of the first certificate in a PEM file, in hex.
fn fingerprint(cert_pem: &[u8]) -> Option<String> {
    let first = rustls::pki_types::CertificateDer::pem_slice_iter(cert_pem).next()?.ok()?;
    Some(hex(&Sha256::digest(first.as_ref())))
}

fn same_names(a: &[String], b: &[String]) -> bool {
    let (mut a, mut b) = (a.to_vec(), b.to_vec());
    a.sort();
    b.sort();
    a == b
}

/// The admin's record of the certificate it generated, and the certificate's fingerprint, when both can be read.
fn recorded(cert_path: &Path, names_path: &Path) -> Option<(Generated, String)> {
    let generated = crate::private::read(names_path)
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_slice::<Generated>(&text).ok())?;
    Some((generated, std::fs::read(cert_path).ok().and_then(|pem| fingerprint(&pem))?))
}

/// Whether the certificate is one the admin generated, as its record of it says.
fn own(cert_path: &Path, names_path: &Path) -> bool {
    recorded(cert_path, names_path).is_some_and(|(g, c)| g.sha256 == c || g.replacing.as_deref() == Some(c.as_str()))
}

/// Why the certificate, one the admin generated, must be generated again: it is for other names than these, or it is
/// the one a generation cut short was replacing. Anything it cannot read as its own record of that certificate leaves
/// the certificate alone.
fn outdated(cert_path: &Path, names_path: &Path, hostnames: &[String]) -> Option<&'static str> {
    let (generated, current) = recorded(cert_path, names_path)?;
    if current == generated.sha256 {
        (!same_names(&generated.hostnames, hostnames)).then_some("ADMIN_HOSTNAMES changed")
    } else {
        (generated.replacing.as_deref() == Some(current.as_str()))
            .then_some("the last certificate generation was cut short")
    }
}

/// A new self-signed certificate and key for `hostnames`, and the record that the admin generated it, each written
/// whole (a temporary file renamed over the last). The record goes first and names the certificate being replaced too,
/// then the key, then the certificate: a start cut short at any point leaves the previous set as it was, or a record
/// that still claims the previous certificate as one to replace (whatever key is beside it), or a file missing, and
/// the next start generates again; once the certificate is in place, the set is whole.
fn generate(cert_path: &Path, key_path: &Path, names_path: &Path, hostnames: &[String]) -> Result<(), String> {
    let certified = rcgen::generate_simple_self_signed(hostnames.to_vec()).map_err(|e| e.to_string())?;
    let cert_pem = certified.cert.pem();
    let sha256 = fingerprint(cert_pem.as_bytes()).ok_or("the generated certificate does not parse")?;
    let replacing = std::fs::read(cert_path).ok().and_then(|pem| fingerprint(&pem)).filter(|old| *old != sha256);
    let record = Generated { hostnames: hostnames.to_vec(), sha256, replacing };
    crate::private::replace(names_path, &serde_json::to_vec(&record).map_err(|e| e.to_string())?)?;
    crate::private::replace(key_path, certified.signing_key.serialize_pem().as_bytes())?;
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(cert_path).map_or(CERT_MODE, |m| m.permissions().mode() & 0o7777);
    Ok(crate::private::replace_with_mode(cert_path, cert_pem.as_bytes(), mode)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn a_generated_certificate_follows_the_names_and_an_operator_s_is_never_replaced() {
        let dir = crate::private::test_dir("tls");
        let cert = || std::fs::read(dir.join("tls-cert.pem")).unwrap();
        server_config(&dir, &names(&["localhost", "127.0.0.1"])).unwrap();
        let first = cert();
        server_config(&dir, &names(&["127.0.0.1", "localhost"])).unwrap();
        assert_eq!(cert(), first, "the same names, in any order, keep the certificate");
        use std::os::unix::fs::PermissionsExt;
        let mode = |mode| std::fs::set_permissions(dir.join("tls-cert.pem"), std::fs::Permissions::from_mode(mode));
        mode(0o640).unwrap();
        server_config(&dir, &names(&["localhost", "127.0.0.1", "box.lan"])).unwrap();
        let renamed = cert();
        assert_ne!(renamed, first, "new names, a new certificate");
        let kept = std::fs::metadata(dir.join("tls-cert.pem")).unwrap().permissions().mode() & 0o777;
        assert_eq!(kept, 0o640, "in the mode the last one had");

        // The operator's own certificate and key, with the admin's earlier record still beside them.
        let operator = rcgen::generate_simple_self_signed(names(&["operator.example"])).unwrap();
        std::fs::write(dir.join("tls-cert.pem"), operator.cert.pem()).unwrap();
        std::fs::write(dir.join("tls-key.pem"), operator.signing_key.serialize_pem()).unwrap();
        server_config(&dir, &names(&["localhost"])).unwrap();
        assert_eq!(cert(), operator.cert.pem().into_bytes(), "never replaced");
        // Nor when its key is missing: the start stops and says so.
        std::fs::remove_file(dir.join("tls-key.pem")).unwrap();
        let refused = server_config(&dir, &names(&["localhost"])).expect_err("refused");
        assert!(refused.contains("has no tls-key.pem"), "{refused}");
        assert_eq!(cert(), operator.cert.pem().into_bytes(), "never replaced, even without its key");
    }

    #[test]
    fn a_generation_cut_short_at_any_write_is_completed_by_the_next_start() {
        use std::os::unix::fs::PermissionsExt;
        let (old, new) = (names(&["localhost"]), names(&["localhost", "box.lan"]));
        for written in 0..3 {
            let dir = crate::private::test_dir("tls-cut");
            let (cert, key, record) = (dir.join("tls-cert.pem"), dir.join("tls-key.pem"), dir.join("tls-names.json"));
            server_config(&dir, &old).unwrap();
            let previous = std::fs::read(&cert).unwrap();
            // The writes of a generation for the new names, in order, the first `written` of them done.
            let certified = rcgen::generate_simple_self_signed(new.clone()).unwrap();
            let sha256 = fingerprint(certified.cert.pem().as_bytes()).unwrap();
            let replacing = fingerprint(&previous);
            let generated = Generated { hostnames: new.clone(), sha256, replacing };
            if written > 0 {
                crate::private::replace(&record, &serde_json::to_vec(&generated).unwrap()).unwrap();
            }
            if written > 1 {
                crate::private::replace(&key, certified.signing_key.serialize_pem().as_bytes()).unwrap();
            }
            server_config(&dir, &new).unwrap_or_else(|e| panic!("cut after {written} writes: {e}"));
            let now = std::fs::read(&cert).unwrap();
            assert_ne!(now, previous, "cut after {written} writes: generated again");
            let kept: Generated = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
            assert_eq!(
                (kept.hostnames, Some(kept.sha256)),
                (new.clone(), fingerprint(&now)),
                "its own, for the new names"
            );
            assert_eq!(std::fs::metadata(&cert).unwrap().permissions().mode() & 0o777, CERT_MODE);
            server_config(&dir, &new).unwrap();
            assert_eq!(std::fs::read(&cert).unwrap(), now, "and kept from then on");
        }
    }
}
