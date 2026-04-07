//! API key storage with OS-keychain backing.
//!
//! Trait so tests can inject [`InMemoryKeychain`]; production uses
//! [`WindowsCredentialKeychain`] which calls Win32 Credential Manager
//! via `windows-sys`. This is the third `unsafe` block in the
//! workspace; the first two are the local CA install (`ca.rs`) and
//! the consent dialog (`consent_messagebox.rs`). See
//! `docs/threat-model.md` T45 / T48 / *new T46* for the safety proof
//! requirements.
//!
//! Keys never appear in audit logs. The trait surface intentionally
//! returns `Option<String>` so the *absence* of a key is a normal
//! state, not an error — the launcher reports it via the
//! `ai_provider_unconfigured` (412) status when a chat is attempted.

use std::sync::Mutex;

#[derive(Debug)]
pub enum KeychainError {
    Io(String),
}

impl std::fmt::Display for KeychainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeychainError::Io(s) => write!(f, "keychain io: {s}"),
        }
    }
}

impl std::error::Error for KeychainError {}

pub trait Keychain: Send + Sync + 'static {
    fn read(&self, name: &str) -> Result<Option<String>, KeychainError>;
    fn write(&self, name: &str, value: &str) -> Result<(), KeychainError>;
    fn delete(&self, name: &str) -> Result<(), KeychainError>;
}

// ---- in-memory fake (tests + non-Windows builds) --------------------------

#[derive(Default)]
pub struct InMemoryKeychain {
    inner: Mutex<std::collections::HashMap<String, String>>,
}

impl InMemoryKeychain {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Keychain for InMemoryKeychain {
    fn read(&self, name: &str) -> Result<Option<String>, KeychainError> {
        let g = self
            .inner
            .lock()
            .map_err(|_| KeychainError::Io("poisoned".into()))?;
        Ok(g.get(name).cloned())
    }

    fn write(&self, name: &str, value: &str) -> Result<(), KeychainError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| KeychainError::Io("poisoned".into()))?;
        g.insert(name.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), KeychainError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| KeychainError::Io("poisoned".into()))?;
        g.remove(name);
        Ok(())
    }
}

// ---- Windows Credential Manager production backend ------------------------

#[cfg(windows)]
pub struct WindowsCredentialKeychain;

#[cfg(windows)]
impl WindowsCredentialKeychain {
    const TARGET_PREFIX: &'static str = "xlpod/";

    fn target_for(name: &str) -> Vec<u16> {
        let mut s = Self::TARGET_PREFIX.to_string();
        s.push_str(name);
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
impl Keychain for WindowsCredentialKeychain {
    fn read(&self, name: &str) -> Result<Option<String>, KeychainError> {
        use windows_sys::Win32::Security::Credentials::{
            CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
        };
        let target = Self::target_for(name);
        // SAFETY:
        // - `CredReadW` writes a freshly-allocated `CREDENTIALW*`
        //   into `out_ptr` on success, or returns 0 with
        //   GetLastError describing the failure.
        // - We pass a NUL-terminated UTF-16 target name owned by
        //   `target`, which lives for the entire unsafe block.
        // - On success we read `cred.CredentialBlob`/`CredentialBlobSize`
        //   *before* calling `CredFree`, then we free immediately.
        // - We never alias the buffer outside the block.
        // This is one of the workspace's small set of unsafe FFI
        // surfaces; see launcher/Cargo.toml workspace lints and
        // docs/threat-model.md T46.
        #[allow(unsafe_code)]
        unsafe {
            let mut out_ptr: *mut CREDENTIALW = std::ptr::null_mut();
            let ok = CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut out_ptr);
            if ok == 0 {
                // Most common case is "not found"; we report None
                // rather than an error so the route handler can
                // return ai_provider_unconfigured cleanly.
                return Ok(None);
            }
            if out_ptr.is_null() {
                return Ok(None);
            }
            let cred = &*out_ptr;
            let blob_len = cred.CredentialBlobSize as usize;
            let value = if cred.CredentialBlob.is_null() || blob_len == 0 {
                None
            } else {
                let slice = std::slice::from_raw_parts(cred.CredentialBlob, blob_len);
                Some(String::from_utf8_lossy(slice).into_owned())
            };
            CredFree(out_ptr as *const std::ffi::c_void);
            Ok(value)
        }
    }

    fn write(&self, name: &str, value: &str) -> Result<(), KeychainError> {
        use windows_sys::Win32::Security::Credentials::{
            CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        };
        let target = Self::target_for(name);
        let bytes = value.as_bytes();
        // SAFETY:
        // - `CredWriteW` reads from `cred.CredentialBlob` for
        //   `CredentialBlobSize` bytes; we own `bytes` for the
        //   entire call.
        // - `cred.TargetName` points into `target`, also owned for
        //   the entire call.
        // - We zero out optional fields and use the documented
        //   `CRED_TYPE_GENERIC` + `CRED_PERSIST_LOCAL_MACHINE`
        //   constants.
        #[allow(unsafe_code)]
        unsafe {
            let mut cred: CREDENTIALW = std::mem::zeroed();
            cred.Type = CRED_TYPE_GENERIC;
            cred.TargetName = target.as_ptr() as *mut u16;
            cred.CredentialBlobSize = bytes.len() as u32;
            cred.CredentialBlob = bytes.as_ptr() as *mut u8;
            cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
            let ok = CredWriteW(&cred, 0);
            if ok == 0 {
                return Err(KeychainError::Io("CredWriteW failed".into()));
            }
        }
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), KeychainError> {
        use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
        let target = Self::target_for(name);
        // SAFETY:
        // - `CredDeleteW` reads `target` as a NUL-terminated UTF-16
        //   string, owned by us for the call.
        // - Returns 0 on failure (e.g. not found); we treat that as
        //   success because deleting a non-existent key is a no-op
        //   from the caller's perspective.
        #[allow(unsafe_code)]
        unsafe {
            let _ = CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0);
        }
        Ok(())
    }
}
