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

/// Names used on the D-Bus interface and in the config file.
///
/// Kept here rather than in the daemon so that the wire vocabulary has exactly
/// one definition — a client and a server disagreeing about the spelling of
/// "power-saver" is a bug that only shows up at runtime.
impl PowerLevel {
    pub const ALL: [Self; 3] = [Self::Performance, Self::Balanced, Self::PowerSaver];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Performance => "performance",
            Self::Balanced => "balanced",
            Self::PowerSaver => "power-saver",
        }
    }
}

impl std::fmt::Display for PowerLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for PowerLevel {
    type Err = UnknownValue;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|level| level.as_str() == s)
            .ok_or(UnknownValue {
                kind: "power level",
                value: s.to_owned(),
            })
    }
}

impl FanMode {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Silent, Self::Aggressive];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Silent => "silent",
            Self::Aggressive => "aggressive",
        }
    }
}

impl std::fmt::Display for FanMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for FanMode {
    type Err = UnknownValue;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == s)
            .ok_or(UnknownValue {
                kind: "fan mode",
                value: s.to_owned(),
            })
    }
}

/// A string that arrived over the wire and did not name anything we know.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown {kind} `{value}`")]
pub struct UnknownValue {
    pub kind: &'static str,
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_round_trip() {
        for level in PowerLevel::ALL {
            assert_eq!(level.as_str().parse(), Ok(level));
        }
        for mode in FanMode::ALL {
            assert_eq!(mode.as_str().parse(), Ok(mode));
        }
    }

    #[test]
    fn unknown_wire_names_are_rejected_with_the_offending_value() {
        let err = "turbo".parse::<PowerLevel>().unwrap_err();
        // "turbo" is MSI's word, not ours: vendor vocabulary must not leak onto
        // the bus, and the error has to say what it saw.
        assert_eq!(err.to_string(), "unknown power level `turbo`");
    }

    #[test]
    fn an_empty_profile_is_recognised_as_a_no_op() {
        assert!(HwProfile::default().is_empty());
        assert!(
            !HwProfile {
                cooler_boost: Some(false),
                ..HwProfile::default()
            }
            .is_empty()
        );
    }
}
