//! Compile-time loopback binding constants.
//!
//! `0.0.0.0` and any non-loopback bind is **forbidden**. The single source of
//! truth for these values is `proto/xlpod.openapi.yaml` under
//! `info.x-xlpod-bind`. CI (Phase 1.4) will diff these constants against the
//! spec and fail the build on drift.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// IPv4 loopback. Must equal `127.0.0.1`.
pub const BIND_V4: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// IPv6 loopback. Must equal `::1`.
pub const BIND_V6: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);

/// Default port. Reserved fallbacks 7422..=7430 will be added in Phase 1.3.
pub const PORT: u16 = 7421;

/// Proto version exposed via `X-XLPod-Proto` and the `/health` body.
/// Must equal `info.x-xlpod-proto` in the spec.
pub const PROTO: u32 = 1;

/// Launcher binary semver. Tracks `Cargo.toml` `package.version`.
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[inline]
pub const fn addr_v4() -> SocketAddr {
    SocketAddr::new(BIND_V4, PORT)
}

#[inline]
#[allow(dead_code)] // wired up in Phase 1.3 (dual-stack bind)
pub const fn addr_v6() -> SocketAddr {
    SocketAddr::new(BIND_V6, PORT)
}

// --- Compile-time guard against accidental non-loopback bind ----------------
// If anyone edits the constants above to a routable address, the build fails.

const _: () = {
    let v4 = match BIND_V4 {
        IpAddr::V4(a) => a,
        IpAddr::V6(_) => panic!("BIND_V4 must be IPv4"),
    };
    let octets = v4.octets();
    if octets[0] != 127 {
        panic!("BIND_V4 must be in 127.0.0.0/8 (loopback only)");
    }
};
