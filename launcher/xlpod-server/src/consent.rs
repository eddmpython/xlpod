//! User consent backend.
//!
//! The launcher's handshake handler MUST consult a [`ConsentBackend`]
//! before issuing a token. The backend is the only place a human (or a
//! human-equivalent policy in tests) gets to say "yes, this origin may
//! act on my machine with these scopes and these filesystem roots".
//! Everything else in the security model — TLS, origin allow-list,
//! bearer tokens, scope guards — protects an *already approved*
//! conversation. This trait is what makes that initial approval
//! happen.
//!
//! Three implementations live in the workspace:
//!
//! - [`AutoApproveConsent`] — always yes. Default for the standalone
//!   `xlpod-server` dev binary and for integration tests, because
//!   forcing a real human to click "Yes" inside `cargo test` would be
//!   absurd. **Never use this in a binary that ships to users.**
//! - [`DenyAllConsent`] — always no. Used by the dedicated regression
//!   test that proves the deny path actually short-circuits the
//!   handshake before a token is minted.
//! - `MessageBoxConsent` — lives in the `xlpod-launcher` crate so the
//!   `unsafe` Win32 FFI for `MessageBoxW` stays out of the library.
//!   This is the production backend; the tray binary always uses it.
//!
//! Backends return a future because a real consent dialog is async by
//! nature: the user takes time to read and click. The handshake
//! handler awaits that future inside its own request task, so a slow
//! user does not block other requests — only the one that asked.

use std::{future::Future, path::PathBuf, pin::Pin};

use crate::auth::Scope;

/// What the user is being asked to approve.
#[derive(Debug, Clone)]
pub struct ConsentRequest {
    /// The exact value of the `Origin` header. Always one of
    /// `ALLOWED_ORIGINS` because `origin_guard` already filtered it.
    pub origin: String,
    /// Scopes the caller asked for.
    pub scopes: Vec<Scope>,
    /// Canonicalized filesystem roots that would be attached to the
    /// token. May be empty if no `fs:*` scope is requested.
    pub fs_roots: Vec<PathBuf>,
}

/// Future returned by every backend. Boxed so the trait stays
/// dyn-compatible.
pub type ConsentFuture = Pin<Box<dyn Future<Output = bool> + Send>>;

pub trait ConsentBackend: Send + Sync + 'static {
    fn request(&self, req: ConsentRequest) -> ConsentFuture;
}

/// Always approve. Dev / test only.
#[derive(Debug, Default, Clone, Copy)]
pub struct AutoApproveConsent;

impl ConsentBackend for AutoApproveConsent {
    fn request(&self, _req: ConsentRequest) -> ConsentFuture {
        Box::pin(async { true })
    }
}

/// Always deny. Used by the integration test that proves the deny
/// path actually short-circuits handshake.
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyAllConsent;

impl ConsentBackend for DenyAllConsent {
    fn request(&self, _req: ConsentRequest) -> ConsentFuture {
        Box::pin(async { false })
    }
}
