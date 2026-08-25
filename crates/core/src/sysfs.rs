//! Thin sysfs helpers.
//!
//! Everything here takes a full path built from a backend's root, and every
//! error carries that path — a bare `ENOENT` from inside a driver tree is close
//! to useless in a bug report.

use std::fs;
use std::path::{Path, PathBuf};

use crate::backend::{Error, Result};

/// Read a sysfs attribute and trim the trailing newline.
pub fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map(|s| s.trim().to_owned())
        .map_err(|source| Error::Read {
            path: path.display().to_string(),
            source,
        })
}

/// Read an attribute, treating any failure as absence.
///
/// Used for optional readings where a missing or unreadable file simply means
/// the machine does not report that value.
pub fn read_opt(path: &Path) -> Option<String> {
    read(path).ok()
}

/// Read an attribute and parse it, treating any failure as absence.
pub fn read_parsed<T: std::str::FromStr>(path: &Path) -> Option<T> {
    read_opt(path)?.parse().ok()
}

/// Write a sysfs attribute.
pub fn write(path: &Path, value: &str) -> Result<()> {
    fs::write(path, value).map_err(|source| Error::Write {
        path: path.display().to_string(),
        source,
    })
}

/// True when the attribute exists and is writable by this process.
///
/// Capability probing uses existence rather than permissions: the daemon runs as
/// root, and reporting a capability as missing merely because an unprivileged
/// TUI cannot write it would be misleading.
pub fn exists(path: &Path) -> bool {
    path.exists()
}

/// Every hwmon directory, in a stable order.
///
/// Directory order from the kernel follows probe order, which changes between
/// boots; sorting keeps a snapshot comparable with the one before it.
pub fn hwmon_dirs(sysfs_root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(sysfs_root.join("class/hwmon")) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs
}

/// The first hwmon chip matching any of `wanted`, in the caller's order of
/// preference rather than the kernel's order of probing.
pub fn hwmon_by_any_name(sysfs_root: &Path, wanted: &[&str]) -> Option<PathBuf> {
    wanted
        .iter()
        .find_map(|name| hwmon_by_name(sysfs_root, name))
}

/// Find the hwmon directory whose `name` attribute matches, e.g. `msi_wmi_platform`.
pub fn hwmon_by_name(sysfs_root: &Path, wanted: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(sysfs_root.join("class/hwmon")).ok()?;
    entries
        .flatten()
        .map(|e| e.path())
        .find(|dir| read_opt(&dir.join("name")).is_some_and(|name| name == wanted))
}

/// List `power_supply` devices whose `type` attribute matches, e.g. `Battery`.
pub fn power_supplies_of_type(sysfs_root: &Path, wanted: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(sysfs_root.join("class/power_supply")) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|dir| read_opt(&dir.join("type")).is_some_and(|t| t == wanted))
        .collect();
    // Directory order is not stable across boots; sort so BAT0 always wins over BAT1.
    found.sort();
    found
}
