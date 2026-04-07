//! Python worker process for `/run/python`.
//!
//! Phase 5 ships a single shared worker per launcher run. The worker
//! is a `python -c <embedded loop>` child process that reads
//! line-delimited JSON-RPC requests on stdin and writes one JSON line
//! per response on stdout. Calls are serialized through a tokio
//! `Mutex` so only one snippet executes at a time against the worker's
//! globals namespace.
//!
//! Lifecycle:
//! - Spawned lazily on first call to [`PythonWorker::exec`].
//! - Reused for every subsequent call until the worker dies, the call
//!   times out, or the launcher shuts down (worker is killed via
//!   `kill_on_drop`).
//! - On timeout the launcher kills the worker process and clears its
//!   slot; the *next* call spawns a fresh worker. This is essential:
//!   without it a runaway snippet could leave the next caller blocked
//!   waiting for a response that will never come.
//!
//! Discovery: the launcher tries `XLPOD_PYTHON` env var, then
//! `python`, then `python3` on PATH. The future tray launcher will
//! eventually point this at the embedded `python.org` distribution
//! (`docs/design.md` §3.2).
//!
//! What this module is **not**:
//! - It is not a sandbox. Snippets run with the launcher process's
//!   own privileges. The trust boundary is the consent dialog at
//!   handshake (`docs/threat-model.md` T29) plus the audit log; the
//!   worker is just an execution mechanism.
//! - It does not enforce per-snippet memory limits yet (Phase 5.x).
//! - It does not isolate per-workbook namespaces yet (Phase 5.x).

use std::{process::Stdio, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

use crate::error::ApiError;

/// Python source for the worker loop. Loaded at compile time so the
/// launcher binary is self-contained — there is no path to a script
/// file at runtime.
const WORKER_SOURCE: &str = include_str!("worker/python_worker.py");

/// Default wall-clock cap per `/run/python` call. The integration
/// tests override this with a much smaller value via [`PythonWorker::with_timeout`].
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ExecResult {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

struct WorkerInner {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

#[derive(Clone)]
pub struct PythonWorker {
    inner: Arc<Mutex<Option<WorkerInner>>>,
    timeout: Duration,
}

impl PythonWorker {
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_millis(DEFAULT_TIMEOUT_MS))
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            timeout,
        }
    }

    /// Generic JSON-RPC call. Used by every typed wrapper below.
    /// `params` is a `serde_json::Value` so callers can build any
    /// shape the worker dispatch expects without inventing a new
    /// Rust type per method.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        let mut guard = self.inner.lock().await;

        // (Re)spawn if we have no live worker.
        if guard.is_none() {
            *guard = Some(spawn_worker().await?);
        }

        // The expects below are guarded by the `is_none` branch above
        // and the `*guard = None` resets on every error path; the
        // only way to reach them is right after a successful spawn or
        // while still holding a previously-good worker.
        #[allow(clippy::expect_used)]
        let id = {
            let inner = guard.as_mut().expect("worker present after spawn check");
            let id = inner.next_id;
            inner.next_id += 1;
            id
        };

        let req = json!({"id": id, "method": method, "params": params});
        let mut line = serde_json::to_string(&req).map_err(|_| ApiError::Internal)?;
        line.push('\n');

        #[allow(clippy::expect_used)]
        let inner = guard.as_mut().expect("worker present");
        if inner.stdin.write_all(line.as_bytes()).await.is_err() {
            *guard = None;
            return Err(ApiError::WorkerCrashed);
        }
        if inner.stdin.flush().await.is_err() {
            *guard = None;
            return Err(ApiError::WorkerCrashed);
        }

        let mut buf = String::new();
        let read_fut = inner.stdout.read_line(&mut buf);
        let read_result = tokio::time::timeout(self.timeout, read_fut).await;

        match read_result {
            Ok(Ok(0)) => {
                // EOF before a response arrived: worker died.
                *guard = None;
                Err(ApiError::WorkerCrashed)
            }
            Ok(Ok(_)) => {
                let parsed: Value =
                    serde_json::from_str(buf.trim()).map_err(|_| ApiError::Internal)?;
                Ok(parsed)
            }
            Ok(Err(_)) => {
                *guard = None;
                Err(ApiError::WorkerCrashed)
            }
            Err(_) => {
                // Timeout: kill the worker so the next call gets a
                // fresh process. The killed child cannot deliver its
                // (eventual) response into someone else's request.
                if let Some(mut dead) = guard.take() {
                    let _ = dead.child.start_kill();
                    let _ = dead.child.wait().await;
                }
                Err(ApiError::WorkerTimeout)
            }
        }
    }

    pub async fn exec(&self, code: &str) -> Result<ExecResult, ApiError> {
        let value = self.call("exec", json!({"code": code})).await?;
        serde_json::from_value(value).map_err(|_| ApiError::Internal)
    }

    /// Helper used by every Excel route: send the request, then
    /// translate the worker's structured `error_code` (if any) into
    /// the matching `ApiError` variant. Returns the raw JSON value on
    /// success so the route handler can pull out method-specific
    /// fields.
    pub async fn excel_call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        let resp = self.call(method, params).await?;
        if resp
            .get("ok")
            .and_then(Value::as_bool)
            .map(|b| !b)
            .unwrap_or(false)
        {
            let code = resp.get("error_code").and_then(Value::as_str).unwrap_or("");
            return Err(match code {
                "excel_not_available" => ApiError::ExcelNotAvailable,
                "excel_not_running" => ApiError::ExcelNotRunning,
                _ => ApiError::ExcelFailed,
            });
        }
        Ok(resp)
    }
}

impl Default for PythonWorker {
    fn default() -> Self {
        Self::new()
    }
}

async fn spawn_worker() -> Result<WorkerInner, ApiError> {
    let executable = find_python();
    for python in executable {
        match try_spawn(python).await {
            Ok(inner) => return Ok(inner),
            Err(()) => continue,
        }
    }
    Err(ApiError::WorkerSpawnFailed)
}

async fn try_spawn(python: &str) -> Result<WorkerInner, ()> {
    // Phase 11: tell the worker where the xlpod client package
    // lives so its bundle_read / bundle_write methods can `import
    // xlpod.bundle`. The launcher honours an explicit override
    // first; otherwise it tries the repo-relative path
    // (`<repo>/client`) so dev runs from `cargo run` find it.
    let mut cmd = Command::new(python);
    cmd.arg("-c")
        .arg(WORKER_SOURCE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Ok(custom) = std::env::var("XLPOD_CLIENT_PATH") {
        if !custom.is_empty() {
            cmd.env("XLPOD_CLIENT_PATH", custom);
        }
    } else {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let candidate = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("client"));
        if let Some(client_dir) = candidate {
            if client_dir.exists() {
                cmd.env("XLPOD_CLIENT_PATH", client_dir);
            }
        }
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Err(()),
    };
    let stdin = child.stdin.take().ok_or(())?;
    let stdout = child.stdout.take().ok_or(())?;
    Ok(WorkerInner {
        child,
        stdin,
        stdout: BufReader::new(stdout),
        next_id: 1,
    })
}

fn find_python() -> Vec<&'static str> {
    // Honour an explicit override first; otherwise try the two
    // common interpreter names. We can't use static lifetimes for an
    // env-derived value, so the override is leaked into a Box::leak
    // — acceptable because the launcher only resolves Python once
    // per process lifetime.
    let mut out: Vec<&'static str> = Vec::new();
    if let Ok(custom) = std::env::var("XLPOD_PYTHON") {
        if !custom.is_empty() {
            let leaked: &'static str = Box::leak(custom.into_boxed_str());
            out.push(leaked);
        }
    }
    out.push("python");
    out.push("python3");
    out
}
