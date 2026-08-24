//! Vendor-neutral model of laptop power state.
//!
//! Backends translate their own vocabulary into these types. MSI calls its
//! performance levels `turbo`/`comfort`/`eco`; ASUS and Lenovo use other words
//! for the same three ideas. Everything above the backend layer speaks only the
//! neutral names.

use serde::{Deserialize, Serialize};

/// How hard the machine is allowed to work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerLevel {
    Performance,
    Balanced,
    PowerSaver,
}

/// Fan behaviour, independent of the performance level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FanMode {
    /// Firmware decides.
    Auto,
    /// Quiet at the cost of higher temperatures.
    Silent,
    /// Spins up earlier and harder than `Auto`.
    Aggressive,
}

/// What a given machine can actually do.
///
/// Every field is checked before a write is attempted, so asking a laptop for
/// something it does not have fails with a clear error instead of an EIO from
/// deep inside sysfs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub power_level: bool,
    pub fan_mode: bool,
    pub cooler_boost: bool,
    pub battery_saver: bool,
    pub charge_threshold: bool,
}

/// Live sensor readings. Every field is optional because coverage varies wildly
/// between vendors and even between firmware revisions of one model.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sensors {
    pub cpu_temp_c: Option<u8>,
    pub gpu_temp_c: Option<u8>,
    /// Fan duty as reported by the EC, in percent.
    pub cpu_fan_percent: Option<u8>,
    pub gpu_fan_percent: Option<u8>,
    /// Measured fan speeds in RPM, in whatever order hwmon lists them.
    /// Zeroed entries are kept: a fan reporting 0 RPM is information, not noise.
    pub fan_rpm: Vec<u32>,
}

/// Battery state relevant to charging policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battery {
    pub capacity_percent: Option<u8>,
    /// Charging stops at this percentage.
    pub charge_end_threshold: Option<u8>,
    pub on_ac: Option<bool>,
}

/// A full snapshot of what the hardware is doing right now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HwState {
    pub power_level: Option<PowerLevel>,
    pub fan_mode: Option<FanMode>,
    pub cooler_boost: Option<bool>,
    pub battery_saver: Option<bool>,
    pub sensors: Sensors,
    pub battery: Battery,
}

/// A requested change. `None` means "leave this alone" — never "turn it off".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HwProfile {
    pub power_level: Option<PowerLevel>,
    pub fan_mode: Option<FanMode>,
    pub cooler_boost: Option<bool>,
    pub battery_saver: Option<bool>,
    pub charge_end_threshold: Option<u8>,
}

impl HwProfile {
    /// True when the profile asks for nothing at all.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}
