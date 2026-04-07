//! Shared application state passed to every handler and middleware.

use std::sync::Arc;

use crate::{audit::AuditLog, auth::TokenStore, rate_limit::RateLimiter};

#[derive(Clone)]
pub struct AppState {
    pub tokens: Arc<TokenStore>,
    pub limiter: Arc<RateLimiter>,
    pub audit: AuditLog,
}
