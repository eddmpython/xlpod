//! Filesystem read primitive used by `GET /fs/read`.
//!
//! Path safety contract (every check is mandatory):
//!
//! 1. Canonicalize the requested path. This resolves `..`, `.`, and
//!    symlinks against the real filesystem. A canonicalize failure
//!    means the path does not exist; return `path_not_found`.
//! 2. The canonical result must lie under at least one of the token's
//!    granted `fs_roots` (themselves canonicalized at handshake time).
//!    Otherwise return `forbidden_path` — the launcher must never read
//!    a byte outside the approved set.
//! 3. The path must be a regular file (not a directory, FIFO, device,
//!    or socket). Otherwise `not_a_file`.
//! 4. The file size must be <= `FS_READ_MAX_BYTES`. Larger files are
//!    rejected *before* the read with `path_too_large`, so a hostile
//!    caller cannot force the launcher to allocate a 4 GiB buffer.
//!
//! Symlinks are followed deliberately. The user's approved roots are
//! the trust boundary; symlinks inside them are part of "normal" file
//! access. A future revision may grow a "no symlink crossing" mode for
//! callers who want stricter containment.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{config::FS_READ_MAX_BYTES, error::ApiError};

#[derive(Debug)]
pub struct ReadOk {
    pub canonical: PathBuf,
    pub bytes: Vec<u8>,
}

pub fn read_under_roots(requested: &Path, roots: &[PathBuf]) -> Result<ReadOk, ApiError> {
    if roots.is_empty() {
        // The token does not carry any fs roots, so even with the
        // `fs:read` scope it cannot reach a single byte.
        return Err(ApiError::ForbiddenPath);
    }

    let canonical = match fs::canonicalize(requested) {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::PathNotFound);
        }
        Err(_) => return Err(ApiError::PathNotFound),
    };

    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err(ApiError::ForbiddenPath);
    }

    let metadata = match fs::metadata(&canonical) {
        Ok(m) => m,
        Err(_) => return Err(ApiError::PathNotFound),
    };
    if !metadata.is_file() {
        return Err(ApiError::NotAFile);
    }
    if metadata.len() > FS_READ_MAX_BYTES {
        return Err(ApiError::PathTooLarge);
    }

    match fs::read(&canonical) {
        Ok(bytes) => Ok(ReadOk { canonical, bytes }),
        Err(_) => Err(ApiError::PathNotFound),
    }
}

/// Canonicalize a list of user-supplied root paths at handshake time.
/// Roots that do not exist or are not directories are silently dropped
/// — the handshake refuses the issue if every root is invalid, but a
/// partial set is honoured (caller learns from `granted_fs_roots`).
pub fn canonicalize_roots(input: &[String]) -> Vec<PathBuf> {
    input
        .iter()
        .filter_map(|raw| fs::canonicalize(raw).ok())
        .filter(|p| p.is_dir())
        .collect()
}
