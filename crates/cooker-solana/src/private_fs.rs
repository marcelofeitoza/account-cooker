//! Owner-only file and directory primitives for the git-ignored key directories.
//!
//! Secret material never reaches a path that another local account can read, so every write
//! goes through these four functions. They were duplicated per module before; the caller's
//! method name is a parameter so one failure still says which call produced it.

use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use crate::RpcError;

/// Create a new `0600` file, failing if the path already exists.
#[cfg(unix)]
pub(crate) fn open_private_new(path: &Path, method: &'static str) -> Result<File, RpcError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| RpcError::invalid_input(method, format!("cannot create file: {error}")))
}

/// Refuse to create secret files on a platform without Unix permission bits.
#[cfg(not(unix))]
pub(crate) fn open_private_new(_path: &Path, method: &'static str) -> Result<File, RpcError> {
    Err(RpcError::invalid_input(
        method,
        "secure local persistence requires Unix",
    ))
}

/// Restrict a directory to `0700`.
#[cfg(unix)]
pub(crate) fn secure_directory(path: &Path, method: &'static str) -> Result<(), RpcError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
        RpcError::invalid_input(method, format!("cannot secure directory: {error}"))
    })
}

/// Refuse to secure directories on a platform without Unix permission bits.
#[cfg(not(unix))]
pub(crate) fn secure_directory(_path: &Path, method: &'static str) -> Result<(), RpcError> {
    Err(RpcError::invalid_input(
        method,
        "secure local persistence requires Unix",
    ))
}

/// Reject a file whose mode grants any group or other access.
#[cfg(unix)]
pub(crate) fn validate_private_permissions(
    metadata: &fs::Metadata,
    method: &'static str,
) -> Result<(), RpcError> {
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(RpcError::invalid_input(
            method,
            "file permissions must deny group and other access",
        ));
    }
    Ok(())
}

/// Refuse to validate permissions on a platform without Unix permission bits.
#[cfg(not(unix))]
pub(crate) fn validate_private_permissions(
    _metadata: &fs::Metadata,
    method: &'static str,
) -> Result<(), RpcError> {
    Err(RpcError::invalid_input(
        method,
        "secure local persistence requires Unix",
    ))
}

/// Flush a directory entry so a newly created file survives a crash.
pub(crate) fn sync_directory(path: &Path, method: &'static str) -> Result<(), RpcError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            RpcError::invalid_input(method, format!("cannot synchronize directory: {error}"))
        })
}
