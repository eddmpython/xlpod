//! xlpod-launcher — tray-resident desktop process that hosts the xlpod
//! loopback HTTPS server.
//!
//! Architecture:
//! - **Main thread** owns the platform event loop (`tao`) and the tray
//!   icon. Windows requires that any process with a tray icon pump its
//!   message queue on the thread that created the icon, so the tray
//!   *must* run on main.
//! - **Worker thread** owns a tokio multi-thread runtime and runs
//!   `xlpod_server::serve()` to completion. The single source of truth
//!   for everything the server does lives in `xlpod-server`; this binary
//!   adds zero new HTTP behaviour.
//!
//! On `Quit` from the tray menu we exit the process, which terminates the
//! server thread along with the runtime. A future Phase 1.4 revision will
//! gracefully shut the server down via a CancellationToken before exit.

#![allow(clippy::expect_used)] // binary entry point — failure is fatal anyway

use std::thread;

use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIconBuilder,
};
use xlpod_server::{
    bind::{addr_v4, LAUNCHER_VERSION, PROTO},
    serve, ServeOptions,
};

fn main() {
    eprintln!("xlpod-launcher v{LAUNCHER_VERSION} (proto {PROTO})");

    // Spawn the HTTPS server on its own thread + tokio runtime.
    let _server = thread::Builder::new()
        .name("xlpod-server".into())
        .spawn(run_server_thread)
        .expect("spawn server thread");

    // Build the tray (must happen on the same thread as the event loop).
    let icon = make_icon();
    let menu = Menu::new();
    let title = MenuItem::new(format!("xlpod v{LAUNCHER_VERSION}"), false, None);
    let endpoint = MenuItem::new(
        format!("https://{} (loopback)", addr_v4()),
        false,
        None,
    );
    let sep = PredefinedMenuItem::separator();
    let quit = MenuItem::new("Quit", true, None);
    menu.append(&title).expect("menu append title");
    menu.append(&endpoint).expect("menu append endpoint");
    menu.append(&sep).expect("menu append sep");
    menu.append(&quit).expect("menu append quit");

    let _tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip(format!("xlpod v{LAUNCHER_VERSION} — loopback launcher"))
        .with_menu(Box::new(menu))
        .build()
        .expect("build tray icon");

    let menu_rx = MenuEvent::receiver();
    let quit_id = quit.id().clone();

    let event_loop = EventLoopBuilder::new().build();
    event_loop.run(move |_event, _target, control_flow| {
        *control_flow = ControlFlow::Poll;
        if let Ok(event) = menu_rx.try_recv() {
            if event.id == quit_id {
                eprintln!("xlpod-launcher: quit requested via tray");
                *control_flow = ControlFlow::Exit;
            }
        }
    });
}

fn run_server_thread() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async {
        let opts = ServeOptions::from_env();
        eprintln!("xlpod-server: cert  {}", opts.tls.cert.display());
        eprintln!("xlpod-server: audit {}", opts.audit_path.display());
        eprintln!("xlpod-server: listening on https://{}", addr_v4());
        if let Err(e) = serve(opts).await {
            eprintln!("xlpod-server: exited with error: {e}");
        }
    });
}

/// Build a 16x16 RGBA tray icon programmatically so we don't need to
/// ship a .ico file. Solid xlpod blue (`#1e40af`) — the brand identity
/// can be refined when there is one.
fn make_icon() -> tray_icon::Icon {
    const SIZE: u32 = 16;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for _ in 0..(SIZE * SIZE) {
        rgba.extend_from_slice(&[0x1e, 0x40, 0xaf, 0xff]);
    }
    tray_icon::Icon::from_rgba(rgba, SIZE, SIZE).expect("build tray icon rgba")
}
