//! xlpod loopback HTTPS server — library entry point.
//!
//! The same crate produces a `xlpod-server` binary (for direct dev use) and
//! a library that the future tray launcher and the integration tests
//! consume. The binary stays a thin shell over `serve()`; everything
//! testable lives here.

pub mod audit;
pub mod auth;
pub mod bind;
pub mod config;
pub mod error;
pub mod middleware;
pub mod rate_limit;
pub mod routes;
pub mod state;
pub mod tls;

pub use routes::router as make_app;
