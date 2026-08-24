//! MSI laptops, via the out-of-tree `msi-ec` driver.
//!
//! Note for anyone reproducing this: the in-kernel `msi_ec` does not expose
//! these attributes on most models. The AUR/DKMS build of `msi-ec` does.

use std::path::{Path, PathBuf};

use crate::backend::{Backend, Error, Probe, Result};
use crate::sysfs;
use crate::types::{Battery, Capabilities, FanMode, HwProfile, HwState, PowerLevel, Sensors};

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

    fn read_battery(&self) -> Battery {
        let mut battery = Battery {
            on_ac: self
                .mains
                .as_ref()
                .and_then(|m| sysfs::read_parsed::<u8>(&m.join("online")))
                .map(|v| v == 1),
            ..Battery::default()
        };
        if let Some(bat) = &self.battery {
            battery.capacity_percent = sysfs::read_parsed(&bat.join("capacity"));
            battery.charge_end_threshold =
                sysfs::read_parsed(&bat.join("charge_control_end_threshold"));
        }
        battery
    }

    /// Reject thresholds the kernel would refuse anyway, with a better message.
    fn check_threshold(value: u8) -> Result<()> {
        if (20..=100).contains(&value) {
            Ok(())
        } else {
            Err(Error::BadValue(
                "charge threshold",
                format!("{value} (expected 20-100)"),
            ))
        }
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
        let caps = Capabilities {
            power_level: true,
            fan_mode: sysfs::exists(&ec.join("fan_mode")),
            cooler_boost: sysfs::exists(&ec.join("cooler_boost")),
            battery_saver: sysfs::exists(&ec.join("super_battery")),
            charge_threshold: battery
                .as_ref()
                .is_some_and(|b| sysfs::exists(&b.join("charge_control_end_threshold"))),
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
            battery: self.read_battery(),
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
        if let Some(threshold) = profile.charge_end_threshold {
            let battery = self
                .battery
                .as_ref()
                .filter(|_| self.caps.charge_threshold)
                .ok_or(Error::Unsupported("charge thresholds"))?;
            Self::check_threshold(threshold)?;
            sysfs::write(
                &battery.join("charge_control_end_threshold"),
                &threshold.to_string(),
            )?;
        }
        Ok(())
    }
}
