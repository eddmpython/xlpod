//! API error type. Serializes to the `Error` schema in
//! `proto/xlpod.openapi.yaml#/components/schemas/Error`.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // variants land as routes are wired up
pub enum ApiError {
    OriginNotAllowed,
    HostNotAllowed,
    Unauthorized,
    ScopeDenied,
    ConsentDenied,
    RateLimited,
    BadRequest,
    ReservedScope,
    ForbiddenPath,
    PathTooLarge,
    NotAFile,
    PathNotFound,
    WorkerSpawnFailed,
    WorkerTimeout,
    WorkerCrashed,
    ExcelNotAvailable,
    ExcelNotRunning,
    ExcelFailed,
    AiProviderUnconfigured,
    AiProviderUpstream,
    AiToolDenied,
    AiConsentDenied,
    AiPlanOnlyViolation,
    AiSessionNotFound,
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
            ApiError::OriginNotAllowed
            | ApiError::HostNotAllowed
            | ApiError::ScopeDenied
            | ApiError::ConsentDenied
            | ApiError::AiToolDenied
            | ApiError::AiConsentDenied
            | ApiError::ForbiddenPath => StatusCode::FORBIDDEN,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ApiError::BadRequest | ApiError::ReservedScope | ApiError::PathTooLarge => {
                StatusCode::BAD_REQUEST
            }
            ApiError::NotAFile | ApiError::PathNotFound | ApiError::AiSessionNotFound => {
                StatusCode::NOT_FOUND
            }
            ApiError::AiProviderUnconfigured => StatusCode::PRECONDITION_FAILED,
            ApiError::AiProviderUpstream => StatusCode::BAD_GATEWAY,
            ApiError::AiPlanOnlyViolation => StatusCode::CONFLICT,
            ApiError::WorkerTimeout => StatusCode::GATEWAY_TIMEOUT,
            ApiError::ExcelNotAvailable | ApiError::ExcelNotRunning => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            ApiError::WorkerSpawnFailed
            | ApiError::WorkerCrashed
            | ApiError::ExcelFailed
            | ApiError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
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
            ApiError::ConsentDenied => ErrorBody {
                code: "consent_denied",
                message: "user denied the handshake at the consent dialog",
                hint: Some("ask the user to approve in the launcher tray"),
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
            ApiError::ForbiddenPath => ErrorBody {
                code: "forbidden_path",
                message: "path is not under any of the token's approved fs roots",
                hint: Some("widen fs_roots at handshake time"),
            },
            ApiError::PathTooLarge => ErrorBody {
                code: "path_too_large",
                message: "file exceeds the launcher's read size limit",
                hint: Some("Phase 3 cap is 10 MiB"),
            },
            ApiError::NotAFile => ErrorBody {
                code: "not_a_file",
                message: "path exists but is not a regular file",
                hint: None,
            },
            ApiError::PathNotFound => ErrorBody {
                code: "path_not_found",
                message: "path does not exist",
                hint: None,
            },
            ApiError::WorkerSpawnFailed => ErrorBody {
                code: "worker_spawn_failed",
                message: "could not start the python worker process",
                hint: Some("install python on PATH or set XLPOD_PYTHON"),
            },
            ApiError::WorkerTimeout => ErrorBody {
                code: "worker_timeout",
                message: "worker exceeded the wall-clock cap and was killed",
                hint: Some("default cap is 30 seconds; the next call gets a fresh worker"),
            },
            ApiError::WorkerCrashed => ErrorBody {
                code: "worker_crashed",
                message: "worker died mid-call",
                hint: Some("the next call will spawn a fresh worker"),
            },
            ApiError::ExcelNotAvailable => ErrorBody {
                code: "excel_not_available",
                message: "the worker's Python does not have pywin32 installed",
                hint: Some("pip install pywin32 into the worker interpreter, or use XLPOD_PYTHON"),
            },
            ApiError::ExcelNotRunning => ErrorBody {
                code: "excel_not_running",
                message: "no running Excel instance to attach to",
                hint: Some("open Excel and a workbook, then retry"),
            },
            ApiError::ExcelFailed => ErrorBody {
                code: "excel_failed",
                message: "Excel COM call raised an exception",
                hint: None,
            },
            ApiError::AiProviderUnconfigured => ErrorBody {
                code: "ai_provider_unconfigured",
                message: "no API key for the requested provider in the OS keychain",
                hint: Some("POST /ai/providers/key with the user's consent first"),
            },
            ApiError::AiProviderUpstream => ErrorBody {
                code: "ai_provider_upstream",
                message: "the AI provider returned an error",
                hint: None,
            },
            ApiError::AiToolDenied => ErrorBody {
                code: "ai_tool_denied",
                message: "the AI internal bearer does not carry the scope this tool requires",
                hint: Some("widen the user's scopes at session open time"),
            },
            ApiError::AiConsentDenied => ErrorBody {
                code: "ai_consent_denied",
                message: "user denied the per-call consent for an AI tool",
                hint: None,
            },
            ApiError::AiPlanOnlyViolation => ErrorBody {
                code: "ai_plan_only_violation",
                message: "model called a mutating tool while plan_only=true is set",
                hint: Some("clear plan_only to apply, or accept the planned diff"),
            },
            ApiError::AiSessionNotFound => ErrorBody {
                code: "ai_session_not_found",
                message: "no session with that id (expired or never created)",
                hint: None,
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
