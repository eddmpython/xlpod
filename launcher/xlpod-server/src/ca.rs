//! Self-issued local Certificate Authority.
//!
//! Replaces the Phase 0 mkcert dependency. On first launch the launcher
//! generates an ECDSA P-256 root CA, persists it under
//! `%LOCALAPPDATA%\xlpod\ca\`, and registers it in the **current user's**
//! Trusted Root Certification Authorities store via `certutil` (no UAC,
//! no machine-wide trust). Every subsequent launch loads the existing CA
//! and issues a fresh server cert valid for `127.0.0.1`, `::1`, and
//! `localhost`. Server keys never touch disk in production paths beyond
//! the cert/key file pair the TLS layer reads.
//!
//! Uninstall (Phase 1.4) will: delete the CA files, run
//! `certutil -user -delstore Root <thumbprint>`, and clear `audit.log`.
//!
//! Threat model notes:
//! - The CA private key is the highest-value asset on the machine. It is
//!   written with the default user ACL (which on Windows already restricts
//!   the file to the owning user). A future revision should additionally
//!   apply an explicit DACL to deny other users.
//! - The CA is constrained at issue time: it only ever signs server certs
//!   for loopback names. If the key is exfiltrated the attacker can MITM
//!   any localhost service trusted by this user — same level as mkcert.
//!   We accept this risk because the alternative (no trusted localhost
//!   TLS) is worse, and the user already trusts arbitrary code on their
//!   own account.

use std::{
    fs,
    path::{Path, PathBuf},
};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose,
    SanType,
};
use sha1::{Digest, Sha1};

#[derive(Debug)]
pub struct CaPaths {
    pub root_dir: PathBuf,
    pub ca_cert: PathBuf,
    pub ca_key: PathBuf,
    pub server_cert: PathBuf,
    pub server_key: PathBuf,
    /// Hex SHA-1 thumbprint of the CA cert (uppercase). Used by certutil.
    pub thumbprint_file: PathBuf,
}

impl CaPaths {
    pub fn default_in_localappdata() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root_dir = base.join("xlpod").join("ca");
        Self {
            ca_cert: root_dir.join("rootCA.pem"),
            ca_key: root_dir.join("rootCA-key.pem"),
            server_cert: root_dir.join("server.pem"),
            server_key: root_dir.join("server-key.pem"),
            thumbprint_file: root_dir.join("rootCA.thumbprint"),
            root_dir,
        }
    }
}

#[derive(Debug)]
pub enum CaError {
    Io(std::io::Error),
    Rcgen(rcgen::Error),
    CertutilFailed(String),
}

impl From<std::io::Error> for CaError {
    fn from(e: std::io::Error) -> Self {
        CaError::Io(e)
    }
}

impl From<rcgen::Error> for CaError {
    fn from(e: rcgen::Error) -> Self {
        CaError::Rcgen(e)
    }
}

impl std::fmt::Display for CaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaError::Io(e) => write!(f, "io error: {e}"),
            CaError::Rcgen(e) => write!(f, "cert generation: {e}"),
            CaError::CertutilFailed(s) => write!(f, "certutil: {s}"),
        }
    }
}

impl std::error::Error for CaError {}

/// Ensure the local CA exists, then issue a fresh server cert. Returns the
/// paths the TLS layer should load.
pub fn ensure(paths: &CaPaths) -> Result<(PathBuf, PathBuf), CaError> {
    fs::create_dir_all(&paths.root_dir)?;

    let (ca_cert_pem, ca_key_pem, ca_kp, der) = if paths.ca_cert.exists() && paths.ca_key.exists()
    {
        let cert_pem = fs::read_to_string(&paths.ca_cert)?;
        let key_pem = fs::read_to_string(&paths.ca_key)?;
        let kp = KeyPair::from_pem(&key_pem)?;
        let der = pem_to_der(&cert_pem)?;
        (cert_pem, key_pem, kp, der)
    } else {
        eprintln!("ca: generating new local CA...");
        let (cert_pem, key_pem, kp) = generate_root_ca()?;
        fs::write(&paths.ca_cert, &cert_pem)?;
        fs::write(&paths.ca_key, &key_pem)?;
        let der = pem_to_der(&cert_pem)?;
        let thumb = sha1_hex(&der);
        fs::write(&paths.thumbprint_file, &thumb)?;
        eprintln!("ca: requesting trust for CA {thumb}");
        eprintln!("ca: NOTE - Windows may show a one-time security dialog");
        eprintln!("    asking you to confirm installing the xlpod local CA.");
        eprintln!("    Click 'Yes' to trust. This happens only on first launch.");
        install_user_root(&der)?;
        eprintln!("ca: CA installed in user root store");
        (cert_pem, key_pem, kp, der)
    };

    // Best-effort: re-install on every launch is silent and idempotent
    // because we use CERT_STORE_ADD_REPLACE_EXISTING. This recovers from
    // a user manually deleting the trust entry without losing the CA key.
    if let Err(e) = install_user_root(&der) {
        eprintln!("ca: warning: re-install check failed: {e}");
    }

    let _ = ca_key_pem; // already written; future PRs may zero this in memory

    let ca_params_for_signing = parse_ca_params(&ca_cert_pem)?;
    let ca_self = ca_params_for_signing.self_signed(&ca_kp)?;

    let (server_cert_pem, server_key_pem) = issue_server_cert(&ca_self, &ca_kp)?;
    fs::write(&paths.server_cert, server_cert_pem)?;
    fs::write(&paths.server_key, server_key_pem)?;

    Ok((paths.server_cert.clone(), paths.server_key.clone()))
}

fn generate_root_ca() -> Result<(String, String, KeyPair), rcgen::Error> {
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "xlpod local CA");
    dn.push(DnType::OrganizationName, "xlpod");
    params.distinguished_name = dn;
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::minutes(1);
    params.not_after = now + time::Duration::days(365 * 5);
    let kp = KeyPair::generate()?;
    let cert = params.self_signed(&kp)?;
    Ok((cert.pem(), kp.serialize_pem(), kp))
}

fn parse_ca_params(_pem: &str) -> Result<CertificateParams, rcgen::Error> {
    // We re-create the same params we used to mint the CA. rcgen 0.13 does
    // not parse a PEM back into CertificateParams, but for re-issuing
    // *server* certs we only need the CA's KeyPair (which we already
    // loaded) and the CA's own DN — we rebuild a matching params struct.
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "xlpod local CA");
    dn.push(DnType::OrganizationName, "xlpod");
    params.distinguished_name = dn;
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::minutes(1);
    params.not_after = now + time::Duration::days(365 * 5);
    Ok(params)
}

fn issue_server_cert(
    ca: &rcgen::Certificate,
    ca_kp: &KeyPair,
) -> Result<(String, String), rcgen::Error> {
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params.subject_alt_names = vec![
        SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        SanType::IpAddress(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)),
        SanType::DnsName("localhost".try_into()?),
    ];
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "xlpod loopback");
    params.distinguished_name = dn;
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::minutes(1);
    params.not_after = now + time::Duration::days(90);
    let server_kp = KeyPair::generate()?;
    let cert = params.signed_by(&server_kp, ca, ca_kp)?;
    Ok((cert.pem(), server_kp.serialize_pem()))
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>, std::io::Error> {
    let mut buf = Vec::new();
    let mut in_block = false;
    for line in pem.lines() {
        if line.starts_with("-----BEGIN") {
            in_block = true;
            continue;
        }
        if line.starts_with("-----END") {
            break;
        }
        if in_block {
            buf.extend(line.trim().as_bytes());
        }
    }
    base64_decode::decode_b64(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn sha1_hex(der: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(der);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02X}")).collect()
}

#[cfg(windows)]
fn install_user_root(der: &[u8]) -> Result<(), CaError> {
    use windows_sys::Win32::Security::Cryptography::{
        CertAddEncodedCertificateToStore, CertCloseStore, CertOpenStore,
        CERT_STORE_ADD_REPLACE_EXISTING, CERT_STORE_PROV_SYSTEM_W,
        CERT_SYSTEM_STORE_CURRENT_USER_ID, CERT_SYSTEM_STORE_LOCATION_SHIFT,
        PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };
    let store_name: Vec<u16> = "Root\0".encode_utf16().collect();
    let store_flags = CERT_SYSTEM_STORE_CURRENT_USER_ID << CERT_SYSTEM_STORE_LOCATION_SHIFT;
    // SAFETY:
    // - `CertOpenStore` with `CERT_STORE_PROV_SYSTEM_W` requires the
    //   provider parameter to point to a NUL-terminated UTF-16 string;
    //   `store_name` satisfies that and outlives the call.
    // - The encoding type argument is unused for the system provider, so
    //   we pass 0 per the API contract.
    // - The hCryptProv parameter accepts 0 to use a default provider.
    // - On success the returned handle is valid until `CertCloseStore`,
    //   which we always call (even on error from the add) before exiting.
    // - `CertAddEncodedCertificateToStore` reads `der.len()` bytes from
    //   `der.as_ptr()`; the slice is borrowed for the entire unsafe block.
    // - We pass a null `pCertContext` because we do not need the resulting
    //   context handle.
    // This is the only `unsafe` block in the workspace; see
    // launcher/Cargo.toml `[workspace.lints.rust]` and docs/threat-model.md.
    #[allow(unsafe_code)]
    unsafe {
        let store = CertOpenStore(
            CERT_STORE_PROV_SYSTEM_W,
            0,
            0,
            store_flags,
            store_name.as_ptr() as *const std::ffi::c_void,
        );
        if store.is_null() {
            return Err(CaError::CertutilFailed(
                "CertOpenStore returned null".into(),
            ));
        }
        let added = CertAddEncodedCertificateToStore(
            store,
            X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            der.as_ptr(),
            der.len() as u32,
            CERT_STORE_ADD_REPLACE_EXISTING,
            std::ptr::null_mut(),
        );
        let _ = CertCloseStore(store, 0);
        if added == 0 {
            return Err(CaError::CertutilFailed(
                "CertAddEncodedCertificateToStore failed".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn install_user_root(_der: &[u8]) -> Result<(), CaError> {
    // No-op on non-Windows hosts (only used by integration tests today).
    Ok(())
}

#[allow(dead_code)] // path used by future inspection tooling
fn _path_compat(p: &Path) -> &Path {
    p
}

// --- tiny inline base64 decoder so we don't pull a whole crate just for
//     thumbprint extraction. Standard alphabet, padding-tolerant. -----------
mod base64_decode {
    pub fn decode_b64(input: &[u8]) -> Result<Vec<u8>, &'static str> {
        const TABLE: [i8; 128] = {
            let mut t = [-1i8; 128];
            let mut i = 0;
            while i < 26 {
                t[b'A' as usize + i] = i as i8;
                t[b'a' as usize + i] = (i + 26) as i8;
                i += 1;
            }
            let mut i = 0;
            while i < 10 {
                t[b'0' as usize + i] = (i + 52) as i8;
                i += 1;
            }
            t[b'+' as usize] = 62;
            t[b'/' as usize] = 63;
            t
        };
        let mut buf = 0u32;
        let mut bits = 0u32;
        let mut out = Vec::with_capacity(input.len() * 3 / 4);
        for &c in input {
            if c == b'=' || c == b'\r' || c == b'\n' || c == b' ' {
                continue;
            }
            if (c as usize) >= TABLE.len() {
                return Err("invalid base64 char");
            }
            let v = TABLE[c as usize];
            if v < 0 {
                return Err("invalid base64 char");
            }
            buf = (buf << 6) | v as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
                buf &= (1 << bits) - 1;
            }
        }
        Ok(out)
    }
}
