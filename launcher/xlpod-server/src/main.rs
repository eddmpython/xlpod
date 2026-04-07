//! xlpod-server binary entry point. Thin shim over `xlpod_server::serve`.
//! The tray launcher in `xlpod-launcher` calls the same function.

use std::process::ExitCode;

use xlpod_server::{
    bind::{LAUNCHER_VERSION, PROTO},
    serve, ServeOptions,
};

#[tokio::main]
async fn main() -> ExitCode {
    eprintln!("xlpod-server v{LAUNCHER_VERSION} (proto {PROTO})");
    let opts = ServeOptions::from_env();
    eprintln!("  cert:  {}", opts.tls.cert.display());
    eprintln!("  key:   {}", opts.tls.key.display());
    eprintln!("  audit: {}", opts.audit_path.display());
    eprintln!(
        "listening on https://{} (loopback only)",
        xlpod_server::bind::addr_v4()
    );
    if let Err(e) = serve(opts).await {
        eprintln!("error: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
