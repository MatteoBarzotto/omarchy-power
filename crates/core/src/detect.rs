//! Hardware detection.
//!
//! Backends are tried in registration order and the first one that recognises
//! the tree wins. Nothing outside this module names a concrete vendor type.

use std::path::{Path, PathBuf};

use crate::backend::{Backend, Error, Probe, Result};
use crate::backends::msi::Msi;

/// Where sysfs lives on a running system.
pub const DEFAULT_SYSFS_ROOT: &str = "/sys";

/// Points detection at a captured tree instead of the real one.
///
/// This is how a fixture from someone else's laptop gets exercised against the
/// real binaries, not just from unit tests.
pub const SYSFS_ROOT_ENV: &str = "OMARCHY_POWER_SYSFS";

/// The sysfs root to use, honouring [`SYSFS_ROOT_ENV`].
pub fn sysfs_root() -> PathBuf {
    std::env::var_os(SYSFS_ROOT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SYSFS_ROOT))
}

/// Detect the backend for this machine.
pub fn detect() -> Result<Box<dyn Backend>> {
    detect_in(&sysfs_root())
}

/// Detect the backend for an arbitrary sysfs tree.
pub fn detect_in(sysfs_root: &Path) -> Result<Box<dyn Backend>> {
    // Add new vendors here; order matters only if two drivers could match the
    // same machine, which so far none do.
    if let Some(backend) = Msi::probe(sysfs_root) {
        return Ok(Box::new(backend));
    }
    Err(Error::NoBackend)
}
