//! Shared application state passed to every handler and middleware.

use std::sync::Arc;

use crate::{audit::AuditLog, auth::TokenStore, rate_limit::RateLimiter};

#[derive(Clone)]
pub struct AppState {
    pub tokens: Arc<TokenStore>,
    pub limiter: Arc<RateLimiter>,
    pub audit: AuditLog,
    /// Permitted Host header values (DNS-rebinding defense). In production
    /// this is `["127.0.0.1:7421", "[::1]:7421"]` from `config::allowed_hosts`;
    /// integration tests inject the actual bound address since they use a
    /// random port.
    pub allowed_hosts: Arc<Vec<String>>,
}
