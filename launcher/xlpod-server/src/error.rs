//! API error type. Serializes to the `Error` schema in
//! `proto/xlpod.openapi.yaml#/components/schemas/Error`.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // variants land as Phase 1.x routes are wired up
pub enum ApiError {
    OriginNotAllowed,
    HostNotAllowed,
    Unauthorized,
    ScopeDenied,
    RateLimited,
    BadRequest,
    ReservedScope,
    Internal,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'static str>,
}

impl ApiError {
    fn status(self) -> StatusCode {
        match self {
            ApiError::OriginNotAllowed | ApiError::HostNotAllowed | ApiError::ScopeDenied => {
                StatusCode::FORBIDDEN
            }
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ApiError::BadRequest | ApiError::ReservedScope => StatusCode::BAD_REQUEST,
            ApiError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn body(self) -> ErrorBody {
        match self {
            ApiError::OriginNotAllowed => ErrorBody {
                code: "origin_not_allowed",
                message: "request origin is not in the launcher allow-list",
                hint: Some("only https://addin.xlwings.org is permitted"),
            },
            ApiError::HostNotAllowed => ErrorBody {
                code: "host_not_allowed",
                message: "Host header must be the loopback bind",
                hint: Some("dns rebinding defense"),
            },
            ApiError::Unauthorized => ErrorBody {
                code: "unauthorized",
                message: "missing or invalid bearer token",
                hint: Some("call POST /auth/handshake first"),
            },
            ApiError::ScopeDenied => ErrorBody {
                code: "scope_denied",
                message: "token does not carry the required scope",
                hint: None,
            },
            ApiError::RateLimited => ErrorBody {
                code: "rate_limited",
                message: "too many requests for this token",
                hint: Some("100 req/s per token"),
            },
            ApiError::BadRequest => ErrorBody {
                code: "bad_request",
                message: "request payload is malformed",
                hint: None,
            },
            ApiError::ReservedScope => ErrorBody {
                code: "reserved_scope",
                message: "requested scope is reserved for a future phase",
                hint: Some("ai:* scopes are not active in Phase 1"),
            },
            ApiError::Internal => ErrorBody {
                code: "internal",
                message: "internal server error",
                hint: None,
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status(), Json(self.body())).into_response()
    }
}
