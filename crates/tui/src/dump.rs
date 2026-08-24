//! `omarchy-power dump-fixture` — capture this machine's sysfs attributes.
//!
//! The point is to let someone with unsupported hardware contribute support for
//! it without writing any Rust: the captured tree drops straight into
//! `fixtures/` and the test suite can then drive their laptop on machines that
//! do not have one.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use omarchy_power_core::detect;

/// Attributes worth capturing, relative to the sysfs root.
///
/// An explicit list rather than a recursive copy: sysfs is full of things that
/// identify a machine (serial numbers, MAC addresses, UUIDs) and a fixture ends
/// up in a public repository.
const EC_ATTRS: &[&str] = &[
    "shift_mode",
    "fan_mode",
    "cooler_boost",
    "super_battery",
    "fw_version",
    "available_shift_modes",
    "available_fan_modes",
    "cpu/realtime_temperature",
    "cpu/realtime_fan_speed",
    "gpu/realtime_temperature",
    "gpu/realtime_fan_speed",
];

const POWER_SUPPLY_ATTRS: &[&str] = &[
    "type",
    "online",
    "capacity",
    "charge_control_end_threshold",
    "charge_control_start_threshold",
];

const HWMON_ATTRS: &[&str] = &[
    "name",
    "fan1_input",
    "fan2_input",
    "fan3_input",
    "fan4_input",
];

pub fn run(dest: &Path) -> Result<()> {
    let root = omarchy_power_core::detect::sysfs_root();

    // Capture even when detection fails — that case is exactly the one worth
    // sending in, so a missing backend must not block the dump.
    let backend = detect();
    match &backend {
        Ok(b) => println!("detected backend: {}", b.name()),
        Err(e) => println!("no backend detected ({e}) — capturing anyway"),
    }

    fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;

    let mut captured = 0usize;
    for attr in EC_ATTRS {
        captured += copy(
            &root.join("devices/platform/msi-ec").join(attr),
            &dest.join("devices/platform/msi-ec").join(attr),
        )?;
    }
    captured += copy_children(&root, dest, "class/hwmon", HWMON_ATTRS)?;
    captured += copy_children(&root, dest, "class/power_supply", POWER_SUPPLY_ATTRS)?;

    println!("captured {captured} attributes into {}", dest.display());
    println!("review it, then attach it to an issue:");
    println!("  tar czf fixture.tar.gz -C {} .", dest.display());
    Ok(())
}

/// Copy one attribute if it exists. Returns how many files were written.
fn copy(from: &Path, to: &Path) -> Result<usize> {
    let Ok(contents) = fs::read_to_string(from) else {
        return Ok(0);
    };
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(to, contents).with_context(|| format!("writing {}", to.display()))?;
    Ok(1)
}

/// Copy a fixed set of attributes from every device under a sysfs class.
fn copy_children(root: &Path, dest: &Path, class: &str, attrs: &[&str]) -> Result<usize> {
    let Ok(entries) = fs::read_dir(root.join(class)) else {
        return Ok(0);
    };
    let mut captured = 0;
    for device in entries.flatten() {
        let name = device.file_name();
        for attr in attrs {
            captured += copy(
                &device.path().join(attr),
                &dest.join(class).join(&name).join(attr),
            )?;
        }
    }
    Ok(captured)
}
