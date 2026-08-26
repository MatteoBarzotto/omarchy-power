//! MSI laptops, via the out-of-tree `msi-ec` driver.
//!
//! Note for anyone reproducing this: the in-kernel `msi_ec` does not expose
//! these attributes on most models. The AUR/DKMS build of `msi-ec` does.
//!
//! The charge thresholds on this firmware are **coupled**: writing the start
//! threshold moves the end one to keep a fixed ten-point gap, and vice versa.
//! Measured on a Katana with msi-ec 0.13 — start 75 leaves end at 85, start 60
//! leaves end at 70. Nothing here enforces or undoes that; the pair is read
//! back from the hardware after every write, so both rows on screen simply
//! show what the firmware settled on. A fixture cannot reproduce it, since a
//! file in a captured tree has no driver behind it to move its neighbour.

use std::path::{Path, PathBuf};

use crate::backend::{Backend, Error, Probe, Result};
use crate::types::{Capabilities, FanMode, HwProfile, HwState, PowerLevel, Sensors};
use crate::{battery, sysfs};

const EC: &str = "devices/platform/msi-ec";
const HWMON: &str = "msi_wmi_platform";

pub struct Msi {
    ec: PathBuf,
    /// Provides real RPM readings; the EC only reports duty percentages.
    hwmon: Option<PathBuf>,
    battery: Option<PathBuf>,
    mains: Option<PathBuf>,
    caps: Capabilities,
}

impl Msi {
    fn attr(&self, name: &str) -> PathBuf {
        self.ec.join(name)
    }

    /// Map a neutral power level onto MSI's shift modes.
    fn shift_mode_value(level: PowerLevel) -> &'static str {
        match level {
            PowerLevel::Performance => "turbo",
            PowerLevel::Balanced => "comfort",
            PowerLevel::PowerSaver => "eco",
        }
    }

    fn parse_shift_mode(raw: &str) -> Option<PowerLevel> {
        match raw {
            "turbo" => Some(PowerLevel::Performance),
            "comfort" => Some(PowerLevel::Balanced),
            "eco" => Some(PowerLevel::PowerSaver),
            _ => None,
        }
    }

    fn fan_mode_value(mode: FanMode) -> &'static str {
        match mode {
            FanMode::Auto => "auto",
            FanMode::Silent => "silent",
            FanMode::Aggressive => "advanced",
        }
    }

    fn parse_fan_mode(raw: &str) -> Option<FanMode> {
        match raw {
            "auto" => Some(FanMode::Auto),
            "silent" => Some(FanMode::Silent),
            "advanced" => Some(FanMode::Aggressive),
            _ => None,
        }
    }

    /// MSI spells booleans `on` and `off`.
    fn parse_switch(raw: &str) -> Option<bool> {
        match raw {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        }
    }

    fn read_fan_rpm(&self) -> Vec<u32> {
        let Some(hwmon) = &self.hwmon else {
            return Vec::new();
        };
        // Fans are numbered from 1 and the count varies by model; stop at the
        // first gap rather than guessing an upper bound.
        (1..)
            .map_while(|i| sysfs::read_parsed::<u32>(&hwmon.join(format!("fan{i}_input"))))
            .collect()
    }
}

impl Probe for Msi {
    fn probe(sysfs_root: &Path) -> Option<Self> {
        let ec = sysfs_root.join(EC);
        // shift_mode is the attribute the whole backend is built around; without
        // it we are looking at a kernel driver too old to be useful.
        if !sysfs::exists(&ec.join("shift_mode")) {
            return None;
        }

        let battery = sysfs::power_supplies_of_type(sysfs_root, "Battery")
            .into_iter()
            .next();
        let (can_end, can_start) = battery::capabilities(battery.as_deref());
        let caps = Capabilities {
            power_level: true,
            fan_mode: sysfs::exists(&ec.join("fan_mode")),
            cooler_boost: sysfs::exists(&ec.join("cooler_boost")),
            battery_saver: sysfs::exists(&ec.join("super_battery")),
            charge_threshold: can_end,
            charge_start_threshold: can_start,
        };

        Some(Self {
            hwmon: sysfs::hwmon_by_name(sysfs_root, HWMON),
            mains: sysfs::power_supplies_of_type(sysfs_root, "Mains")
                .into_iter()
                .next(),
            battery,
            ec,
            caps,
        })
    }
}

impl Backend for Msi {
    fn name(&self) -> &'static str {
        "msi-ec"
    }

    fn model(&self) -> Option<String> {
        sysfs::read_opt(&self.attr("fw_version"))
    }

    fn capabilities(&self) -> Capabilities {
        self.caps
    }

    fn read_state(&self) -> Result<HwState> {
        Ok(HwState {
            power_level: sysfs::read_opt(&self.attr("shift_mode"))
                .and_then(|v| Self::parse_shift_mode(&v)),
            fan_mode: sysfs::read_opt(&self.attr("fan_mode"))
                .and_then(|v| Self::parse_fan_mode(&v)),
            cooler_boost: sysfs::read_opt(&self.attr("cooler_boost"))
                .and_then(|v| Self::parse_switch(&v)),
            battery_saver: sysfs::read_opt(&self.attr("super_battery"))
                .and_then(|v| Self::parse_switch(&v)),
            sensors: Sensors {
                cpu_temp_c: sysfs::read_parsed(&self.attr("cpu/realtime_temperature")),
                gpu_temp_c: sysfs::read_parsed(&self.attr("gpu/realtime_temperature")),
                cpu_fan_percent: sysfs::read_parsed(&self.attr("cpu/realtime_fan_speed")),
                gpu_fan_percent: sysfs::read_parsed(&self.attr("gpu/realtime_fan_speed")),
                fan_rpm: self.read_fan_rpm(),
            },
            battery: battery::read(self.battery.as_deref(), self.mains.as_deref()),
        })
    }

    fn apply(&self, profile: &HwProfile) -> Result<()> {
        if let Some(level) = profile.power_level {
            sysfs::write(&self.attr("shift_mode"), Self::shift_mode_value(level))?;
        }
        if let Some(mode) = profile.fan_mode {
            if !self.caps.fan_mode {
                return Err(Error::Unsupported("fan modes"));
            }
            sysfs::write(&self.attr("fan_mode"), Self::fan_mode_value(mode))?;
        }
        if let Some(on) = profile.cooler_boost {
            if !self.caps.cooler_boost {
                return Err(Error::Unsupported("cooler boost"));
            }
            sysfs::write(&self.attr("cooler_boost"), if on { "on" } else { "off" })?;
        }
        if let Some(on) = profile.battery_saver {
            if !self.caps.battery_saver {
                return Err(Error::Unsupported("battery saver"));
            }
            sysfs::write(&self.attr("super_battery"), if on { "on" } else { "off" })?;
        }
        battery::apply(
            self.battery.as_deref(),
            (self.caps.charge_threshold, self.caps.charge_start_threshold),
            profile,
        )
    }
}
