//! Tool dispatch — translate a model `tool_use` block into a real
//! launcher route call.
//!
//! Phase 8 ships an in-process dispatcher: instead of going through
//! HTTP / TLS, we call the launcher's *internal* worker and route
//! handlers directly with the same scope+consent enforcement that
//! HTTP requests get. This avoids re-creating an HTTP client inside
//! the same process and is the model an in-process MCP server
//! would use anyway.
//!
//! The four key invariants Phase 8 enforces:
//!
//! 1. The tool name MUST exist in `tools::find()` — unknown tools
//!    return `ai_tool_denied`. (We never proxy arbitrary names.)
//! 2. The session's internal bearer MUST hold the tool's
//!    `required_scope`. The intersection was baked at session-open
//!    time so the model cannot have escalated; this check is
//!    belt-and-suspenders.
//! 3. Mutating tools MUST traverse the consent gate before
//!    execution. Phase 8 asks every time; Phase 9 adds trust
//!    windows that can skip the dialog.
//! 4. Errors from the tool's underlying call become structured
//!    `tool_result` blocks with `ok: false`, *not* HTTP errors.
//!    The model needs to see them so it can recover or report.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::session::Session;
use crate::ai::tools;
use crate::ai::trust_window::TrustWindowStore;
use crate::ai::types::{ApprovedVia, ContentBlock};
use crate::auth::Scope;
use crate::consent::{ConsentBackend, ConsentRequest};
use crate::error::ApiError;
use crate::fs_read;
use crate::python_worker::PythonWorker;
use crate::state::AppState;

pub struct DispatchCtx<'a> {
    pub state: &'a AppState,
    pub ai_consent: &'a Arc<dyn ConsentBackend>,
    pub trust_windows: &'a TrustWindowStore,
    pub session: &'a Session,
    pub plan_only: bool,
}

pub async fn execute_tool_use(
    ctx: &DispatchCtx<'_>,
    tool_use_id: &str,
    name: &str,
    input: &Value,
) -> ContentBlock {
    let result = run_one(ctx, name, input).await;
    match result {
        Ok((value, approved_via)) => ContentBlock::ToolResult {
            tool_use_id: tool_use_id.to_string(),
            ok: true,
            output: value,
            approved_via: Some(approved_via),
        },
        Err(err) => ContentBlock::ToolResult {
            tool_use_id: tool_use_id.to_string(),
            ok: false,
            output: error_payload(&err),
            approved_via: None,
        },
    }
}

fn error_payload(err: &ApiError) -> Value {
    json!({
        "error": format!("{:?}", err),
        "hint": "tool execution failed; see launcher audit log for details"
    })
}

async fn run_one(
    ctx: &DispatchCtx<'_>,
    name: &str,
    input: &Value,
) -> Result<(Value, ApprovedVia), ApiError> {
    let spec = tools::find(name).ok_or(ApiError::AiToolDenied)?;

    if !ctx.session.granted_scopes.contains(&spec.required_scope) {
        return Err(ApiError::AiToolDenied);
    }

    let approved_via = if spec.mutates {
        if ctx.plan_only {
            return Err(ApiError::AiPlanOnlyViolation);
        }
        // Trust window first — if the user already approved this
        // tool for this session, skip the per-call dialog. Audit
        // still records the call (the audit middleware runs around
        // the route, and the route in turn called dispatch).
        if ctx.trust_windows.covers(ctx.session.id, name) {
            ApprovedVia::TrustWindow
        } else {
            let req = ConsentRequest {
                origin: format!("ai://{}/{}", ctx.session.provider, ctx.session.id),
                scopes: vec![spec.required_scope],
                fs_roots: vec![],
            };
            let ok = ctx.ai_consent.request(req).await;
            if !ok {
                return Err(ApiError::AiConsentDenied);
            }
            ApprovedVia::Dialog
        }
    } else {
        ApprovedVia::Auto
    };

    let value = match spec.required_scope {
        Scope::FsRead => run_fs_read(ctx, input)?,
        Scope::ExcelCom => run_excel(ctx, name, input).await?,
        Scope::AiExecPython => run_python(&ctx.state.worker, input).await?,
        _ => return Err(ApiError::AiToolDenied),
    };

    Ok((value, approved_via))
}

fn run_fs_read(ctx: &DispatchCtx<'_>, input: &Value) -> Result<Value, ApiError> {
    let path = input
        .get("path")
        .and_then(Value::as_str)
        .ok_or(ApiError::BadRequest)?;
    // Phase 8: dispatch is in-process; we use the *user's* original
    // fs_roots from the calling token. The launcher must look those
    // up via the session's parent token. Phase 8 simplification: we
    // pass the canonicalized roots from the session's parent token,
    // which the route handler stores on the session record. (This
    // hook is the integration point for Phase 11 bundle support.)
    //
    // For now, dispatch reuses the same fs_read::read_under_roots
    // helper directly. The session-side guarantee is that the AI's
    // internal bearer was minted with the user's full fs_roots set.
    let roots = &ctx.session.fs_roots_for_dispatch();
    let result = fs_read::read_under_roots(std::path::Path::new(path), roots)?;
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let encoded = BASE64.encode(&result.bytes);
    Ok(json!({
        "path": result.canonical.display().to_string(),
        "size": result.bytes.len(),
        "encoding": "base64",
        "content": encoded,
    }))
}

async fn run_excel(ctx: &DispatchCtx<'_>, name: &str, input: &Value) -> Result<Value, ApiError> {
    match name {
        "excel_workbooks" => {
            let resp = ctx
                .state
                .worker
                .excel_call("excel_workbooks", json!({}))
                .await?;
            Ok(resp.get("workbooks").cloned().unwrap_or_else(|| json!([])))
        }
        "excel_range_read" => {
            let workbook = input.get("workbook").and_then(Value::as_str).unwrap_or("");
            let sheet = input.get("sheet").and_then(Value::as_str).unwrap_or("");
            let range = input.get("range").and_then(Value::as_str).unwrap_or("");
            let resp = ctx
                .state
                .worker
                .excel_call(
                    "excel_range_read",
                    json!({"workbook": workbook, "sheet": sheet, "range": range}),
                )
                .await?;
            Ok(json!({
                "address": resp.get("address").cloned().unwrap_or_else(|| json!("")),
                "values": resp.get("values").cloned().unwrap_or_else(|| json!([])),
            }))
        }
        _ => Err(ApiError::AiToolDenied),
    }
}

async fn run_python(worker: &PythonWorker, input: &Value) -> Result<Value, ApiError> {
    let code = input
        .get("code")
        .and_then(Value::as_str)
        .ok_or(ApiError::BadRequest)?;
    let result = worker.exec(code).await?;
    Ok(serde_json::to_value(result).unwrap_or_else(|_| json!({})))
}
