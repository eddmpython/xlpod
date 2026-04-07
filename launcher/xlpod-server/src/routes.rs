//! HTTP routes. Phase 1.1a exposes `/health` only.
//!
//! Authoritative schema: `proto/xlpod.openapi.yaml#/components/schemas/Health`.

use axum::{routing::get, Json, Router};
use serde::Serialize;

use crate::bind::{LAUNCHER_VERSION, PROTO};

#[derive(Serialize)]
struct Health {
    status: &'static str,
    launcher: &'static str,
    proto: u32,
}

pub fn router() -> Router {
    Router::new().route("/health", get(health))
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        launcher: LAUNCHER_VERSION,
        proto: PROTO,
    })
}
