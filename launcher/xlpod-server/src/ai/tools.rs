//! Tool registry — exposes existing launcher routes as MCP-style
//! tools the model can call.
//!
//! Phase 8 ships a static list of four tools wrapping the existing
//! routes. Each ToolSpec carries:
//!   - the JSON Schema the model receives (hand-written to match
//!     the route's request body — keeping it explicit avoids the
//!     `schemars` derive dance and matches the SSOT in
//!     `proto/xlpod.openapi.yaml`)
//!   - the underlying xlpod route the launcher dispatches to
//!   - the scope the *AI internal bearer* must carry to use it
//!   - a `mutates` flag that triggers the consent gate
//!
//! Note the asymmetry on `run_python`: a *user* needs `run:python`
//! to call `POST /run/python` directly, but a *model* needs the
//! distinct `ai:exec:python` scope. So a token can hold `run:python`
//! without ever letting an AI run code — and vice versa. The
//! intersection enforced at session-open time uses these scopes.

use serde_json::json;

use crate::ai::types::ToolSpec;
use crate::auth::Scope;

pub fn builtin_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "fs_read".to_string(),
            description: "Read a file under one of the token's approved fs roots. \
                 The path is canonicalized server-side; paths outside the \
                 approved set are rejected. Returns base64 content."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path; must lie under an approved fs root."
                    }
                }
            }),
            xlpod_route: "GET /fs/read".to_string(),
            required_scope: Scope::FsRead,
            mutates: false,
        },
        ToolSpec {
            name: "excel_workbooks".to_string(),
            description: "List workbooks open in the user's running Excel instance.".to_string(),
            input_schema: json!({"type": "object", "properties": {}}),
            xlpod_route: "GET /excel/workbooks".to_string(),
            required_scope: Scope::ExcelCom,
            mutates: false,
        },
        ToolSpec {
            name: "excel_range_read".to_string(),
            description: "Read a range from a workbook in the user's running Excel \
                 instance. Returns a 2-D array of cell values."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["workbook", "sheet", "range"],
                "properties": {
                    "workbook": {"type": "string"},
                    "sheet": {"type": "string"},
                    "range": {"type": "string"}
                }
            }),
            xlpod_route: "POST /excel/range/read".to_string(),
            required_scope: Scope::ExcelCom,
            mutates: false,
        },
        ToolSpec {
            name: "run_python".to_string(),
            description: "Execute a Python snippet inside the launcher's worker. \
                 The worker is shared across calls in this session; \
                 setting `_result` returns its repr() in the response. \
                 Stdout and stderr are captured."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["code"],
                "properties": {
                    "code": {"type": "string"}
                }
            }),
            xlpod_route: "POST /run/python".to_string(),
            // Distinct from the user-facing run:python so a token can
            // hold one without the other.
            required_scope: Scope::AiExecPython,
            mutates: true,
        },
    ]
}

/// Look up a tool by name. Used by `dispatch::execute_tool_use`.
pub fn find(name: &str) -> Option<ToolSpec> {
    builtin_tools().into_iter().find(|t| t.name == name)
}
