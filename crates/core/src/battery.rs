//! Charge thresholds and battery readings, shared by every backend.
//!
//! `charge_control_end_threshold` and its `start` counterpart are kernel names,
//! not vendor ones: the MSI EC driver and a ThinkPad expose the same two files
//! in the same `power_supply` directory. Both backends were already carrying
//! identical copies of this, and a third reading would have made it three.

use std::path::Path;

use crate::backend::{Error, Result};
use crate::sysfs;
use crate::types::{Battery, HwProfile};

const END: &str = "charge_control_end_threshold";
const START: &str = "charge_control_start_threshold";

/// Everything the battery and the mains adapter report.
pub fn read(battery: Option<&Path>, mains: Option<&Path>) -> Battery {
    let mut state = Battery {
        on_ac: mains
            .and_then(|m| sysfs::read_parsed::<u8>(&m.join("online")))
            .map(|v| v == 1),
        ..Battery::default()
    };
    if let Some(dir) = battery {
        state.capacity_percent = sysfs::read_parsed(&dir.join("capacity"));
        state.charge_end_threshold = sysfs::read_parsed(&dir.join(END));
        state.charge_start_threshold = sysfs::read_parsed(&dir.join(START));
    }
    state
}

/// Which of the two thresholds this machine actually exposes.
///
/// Kept separate because they are separate files: a driver offering only the
/// end threshold is common, and claiming both would put a key on screen that
/// fails when pressed.
pub fn capabilities(battery: Option<&Path>) -> (bool, bool) {
    let has = |name: &str| battery.is_some_and(|dir| sysfs::exists(&dir.join(name)));
    (has(END), has(START))
}

/// Write whichever thresholds the profile asks for.
///
/// The kernel refuses a start threshold at or above the end one, so the order
/// of the two writes matters: raising the pair has to move the end first, and
/// lowering it has to move the start first. Doing it the other way round makes
/// the firmware reject a change that is perfectly valid once both have landed.
pub fn apply(
    battery: Option<&Path>,
    (can_end, can_start): (bool, bool),
    profile: &HwProfile,
) -> Result<()> {
    if profile.charge_end_threshold.is_none() && profile.charge_start_threshold.is_none() {
        return Ok(());
    }
    let dir = battery.ok_or(Error::Unsupported("charge thresholds"))?;

    if let Some(value) = profile.charge_end_threshold {
        if !can_end {
            return Err(Error::Unsupported("charge thresholds"));
        }
        check_range(value)?;
    }
    if let Some(value) = profile.charge_start_threshold {
        if !can_start {
            return Err(Error::Unsupported("a start threshold"));
        }
        check_range(value)?;
    }
    check_order(dir, profile)?;

    let raising = profile
        .charge_end_threshold
        .zip(sysfs::read_parsed::<u8>(&dir.join(END)))
        .is_none_or(|(wanted, current)| wanted > current);

    let writes: [(&str, Option<u8>); 2] = if raising {
        [
            (END, profile.charge_end_threshold),
            (START, profile.charge_start_threshold),
        ]
    } else {
        [
            (START, profile.charge_start_threshold),
            (END, profile.charge_end_threshold),
        ]
    };
    for (name, value) in writes {
        if let Some(value) = value {
            sysfs::write(&dir.join(name), &value.to_string())?;
        }
    }
    Ok(())
}

/// Reject thresholds the kernel would refuse anyway, with a better message.
fn check_range(value: u8) -> Result<()> {
    if (20..=100).contains(&value) {
        Ok(())
    } else {
        Err(Error::BadValue(
            "charge threshold",
            format!("{value} (expected 20-100)"),
        ))
    }
}

/// A start threshold must stay below the end one, counting whichever half of
/// the pair the caller left alone.
fn check_order(dir: &Path, profile: &HwProfile) -> Result<()> {
    let end = profile
        .charge_end_threshold
        .or_else(|| sysfs::read_parsed(&dir.join(END)));
    let start = profile
        .charge_start_threshold
        .or_else(|| sysfs::read_parsed(&dir.join(START)));
    match (start, end) {
        (Some(start), Some(end)) if start >= end => Err(Error::BadValue(
            "charge start threshold",
            format!("{start} (must be below the end threshold of {end})"),
        )),
        _ => Ok(()),
    }
}
