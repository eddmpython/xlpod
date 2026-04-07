//! Production consent backend: Win32 `MessageBoxW` modal dialog.
//!
//! Lives in the launcher crate (not in `xlpod-server`) so the
//! `unsafe` FFI for the GUI call stays out of the library and never
//! ships into the integration test environment. The handshake handler
//! awaits a future this backend returns; the future does the actual
//! `MessageBoxW` call inside `tokio::task::spawn_blocking`, so the
//! tokio runtime keeps serving other requests while the user reads.
//!
//! The dialog is `MB_TOPMOST | MB_SYSTEMMODAL` so a slow user cannot
//! be tricked into approving by a popunder; on Korean Windows it
//! renders the body via the system font, which already supports the
//! Hangul we use in the rest of the project.
//!
//! Threat model entry: see `docs/threat-model.md` T29.

use std::path::Path;

use xlpod_server::{
    auth::Scope,
    consent::{ConsentBackend, ConsentFuture, ConsentRequest},
};

#[derive(Debug, Default, Clone, Copy)]
pub struct MessageBoxConsent;

impl ConsentBackend for MessageBoxConsent {
    fn request(&self, req: ConsentRequest) -> ConsentFuture {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || show_dialog(&req))
                .await
                .unwrap_or(false)
        })
    }
}

#[cfg(windows)]
fn show_dialog(req: &ConsentRequest) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_ICONQUESTION, MB_SYSTEMMODAL, MB_TOPMOST, MB_YESNO,
    };

    let title: Vec<u16> = "xlpod consent\0".encode_utf16().collect();

    let scopes = req
        .scopes
        .iter()
        .map(scope_label)
        .collect::<Vec<_>>()
        .join(", ");
    let roots = if req.fs_roots.is_empty() {
        "(none)".to_string()
    } else {
        req.fs_roots
            .iter()
            .map(|p| format_root(p))
            .collect::<Vec<_>>()
            .join("\n  ")
    };
    let body = format!(
        "Allow this website to talk to xlpod on your machine?\n\n\
         Origin:\n  {origin}\n\n\
         Scopes:\n  {scopes}\n\n\
         Filesystem roots:\n  {roots}\n\n\
         Click Yes only if you initiated this action.",
        origin = req.origin,
    );
    let mut body_w: Vec<u16> = body.encode_utf16().collect();
    body_w.push(0);

    // SAFETY:
    // - `MessageBoxW` reads NUL-terminated UTF-16 strings from the two
    //   pointers we pass; both `title` and `body_w` are owned `Vec<u16>`
    //   in this scope and end with an explicit NUL terminator.
    // - The hWnd parameter accepts 0 (no owner window).
    // - `style` is a documented bitmask combining MB_YESNO with the
    //   modal/topmost icon flags; the Win32 API ignores unknown bits.
    // - The function returns an i32 message-box identifier; we compare
    //   against the documented `IDYES` constant.
    // This is the second `unsafe` block in the workspace; see
    // `launcher/Cargo.toml` workspace lints and `docs/threat-model.md`.
    #[allow(unsafe_code)]
    let result = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body_w.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SYSTEMMODAL,
        )
    };
    result == IDYES
}

#[cfg(not(windows))]
fn show_dialog(_req: &ConsentRequest) -> bool {
    // No GUI on non-Windows hosts. The launcher binary only ships on
    // Windows in Phase 1; this stub keeps `cargo check` honest on a
    // dev box that happens to run Linux.
    false
}

fn scope_label(s: &Scope) -> &'static str {
    match s {
        Scope::FsRead => "fs:read",
        Scope::FsWrite => "fs:write",
        Scope::RunPython => "run:python",
        Scope::ExcelCom => "excel:com",
        Scope::AiProviderCall => "ai:provider:call",
        Scope::AiCodegenWrite => "ai:codegen:write",
        Scope::AiExecPython => "ai:exec:python",
    }
}

fn format_root(p: &Path) -> String {
    p.display().to_string()
}
