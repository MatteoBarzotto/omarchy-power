//! Laptops covered by the kernel's own interfaces, with no vendor driver of
//! ours in the middle.
//!
//! `platform_profile` is the ACPI performance switch that ThinkPads, Framework,
//! Dell, ASUS and a good part of HP already expose, and
//! `charge_control_end_threshold` is the standard name for a charge limit. A
//! machine with both needs no vendor code from us at all — which is the point
//! of this backend: it is the check on whether the vendor boundary is a real
//! abstraction or a shape traced around one laptop.
//!
//! What it deliberately does not do: fan modes, cooler boost and battery saver.
//! No standard kernel interface exposes them, and inventing one per vendor is
//! exactly what the MSI backend is for.

use std::path::{Path, PathBuf};

use crate::backend::{Backend, Error, Probe, Result};
use crate::sysfs;
use crate::types::{Battery, Capabilities, HwProfile, HwState, PowerLevel, Sensors};

/// The original single-file interface, still present on every kernel that has
/// the newer one.
const ACPI_PROFILE: &str = "firmware/acpi/platform_profile";
const ACPI_CHOICES: &str = "firmware/acpi/platform_profile_choices";

/// Since 6.14 the same thing also appears as a class, one device per handler.
const CLASS_DIR: &str = "class/platform-profile";

/// hwmon chips that report a CPU package temperature, in the order we prefer
/// them. All three are the standard in-tree drivers, not vendor extras.
const CPU_HWMON: &[&str] = &["coretemp", "k10temp", "zenpower"];
/// Discrete and integrated GPU chips that report a temperature the same way.
const GPU_HWMON: &[&str] = &["amdgpu", "nouveau", "i915", "xe"];

pub struct PlatformProfile {
    /// Kept because the sensors live all over the tree rather than under one
    /// vendor directory, and a backend never builds an absolute path.
    root: PathBuf,
    /// The attribute holding the current profile, whichever interface it came
    /// from — the two are the same switch, so only one is ever used.
    profile: PathBuf,
    /// What this firmware accepts. Read once at probe: the list is fixed by the
    /// firmware and never changes while the machine is running.
    choices: Vec<String>,
    battery: Option<PathBuf>,
    mains: Option<PathBuf>,
    caps: Capabilities,
}

impl PlatformProfile {
    /// Locate the profile attribute and its list of choices.
    ///
    /// The class interface is preferred where it exists: the ACPI file is kept
    /// for compatibility but is the older of the two, and on a machine with
    /// several handlers it is only ever the first one.
    fn locate(sysfs_root: &Path) -> Option<(PathBuf, PathBuf)> {
        if let Some(dir) = Self::class_device(sysfs_root) {
            let profile = dir.join("profile");
            if sysfs::exists(&profile) {
                return Some((profile, dir.join("choices")));
            }
        }
        let profile = sysfs_root.join(ACPI_PROFILE);
        sysfs::exists(&profile).then(|| (profile, sysfs_root.join(ACPI_CHOICES)))
    }

    /// The first platform-profile device, by name, so the choice is stable.
    fn class_device(sysfs_root: &Path) -> Option<PathBuf> {
        let mut devices: Vec<PathBuf> = std::fs::read_dir(sysfs_root.join(CLASS_DIR))
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .collect();
        devices.sort();
        devices.into_iter().next()
    }

    /// Firmware names for a neutral level, best first.
    ///
    /// Machines differ in which ones they offer — a ThinkPad has `low-power`,
    /// some HP firmware calls the same idea `quiet` or `cool` — so the write
    /// picks the first name this machine actually accepts rather than assuming.
    /// `balanced` closes every list because a firmware without it would have to
    /// be stranger than anything the kernel documents.
    fn candidates(level: PowerLevel) -> &'static [&'static str] {
        match level {
            PowerLevel::Performance => &["performance", "balanced-performance", "balanced"],
            PowerLevel::Balanced => &["balanced", "balanced-performance"],
            PowerLevel::PowerSaver => &["low-power", "quiet", "cool", "balanced"],
        }
    }

    /// Read a firmware name back as a neutral level.
    ///
    /// `balanced-performance` reads as balanced rather than as performance: it
    /// is the profile a machine sits in when nothing has asked for full power,
    /// and reporting it as `performance` would make the TUI disagree with the
    /// key the user just pressed.
    fn parse(raw: &str) -> Option<PowerLevel> {
        match raw {
            "performance" => Some(PowerLevel::Performance),
            "balanced" | "balanced-performance" => Some(PowerLevel::Balanced),
            "low-power" | "quiet" | "cool" => Some(PowerLevel::PowerSaver),
            // `custom` means the firmware is following its own settings, which
            // is none of our three; saying nothing beats guessing.
            _ => None,
        }
    }

    /// The name to write for a level, or `None` when this firmware has none.
    fn value_for(&self, level: PowerLevel) -> Option<&str> {
        Self::candidates(level)
            .iter()
            .find(|name| self.choices.iter().any(|choice| choice == *name))
            .copied()
    }

    /// hwmon reports millidegrees; the rest of the program speaks whole degrees.
    fn temp_from(dir: Option<PathBuf>) -> Option<u8> {
        let millidegrees: i32 = sysfs::read_parsed(&dir?.join("temp1_input"))?;
        u8::try_from(millidegrees / 1000).ok()
    }

    /// Every fan any hwmon chip reports, in a stable order.
    ///
    /// Unlike the MSI backend there is no single chip to ask: a ThinkPad puts
    /// its fan under `thinkpad`, a Framework under the EC driver, and a desktop
    /// board would list half a dozen.
    fn read_fan_rpm(sysfs_root: &Path) -> Vec<u32> {
        sysfs::hwmon_dirs(sysfs_root)
            .into_iter()
            .flat_map(|dir| {
                (1..).map_while(move |i| {
                    sysfs::read_parsed::<u32>(&dir.join(format!("fan{i}_input")))
                })
            })
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

impl Probe for PlatformProfile {
    fn probe(sysfs_root: &Path) -> Option<Self> {
        let (profile, choices_path) = Self::locate(sysfs_root)?;
        let choices: Vec<String> = sysfs::read_opt(&choices_path)
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        // A profile switch that accepts nothing we recognise is not a machine
        // we can drive, and claiming it would leave the user with a backend
        // whose every key press fails.
        if !choices.iter().any(|name| Self::parse(name).is_some()) {
            return None;
        }

        let battery = sysfs::power_supplies_of_type(sysfs_root, "Battery")
            .into_iter()
            .next();
        let caps = Capabilities {
            power_level: true,
            // Nothing standard exposes these; see the module comment.
            fan_mode: false,
            cooler_boost: false,
            battery_saver: false,
            charge_threshold: battery
                .as_ref()
                .is_some_and(|b| sysfs::exists(&b.join("charge_control_end_threshold"))),
        };

        Some(Self {
            root: sysfs_root.to_owned(),
            profile,
            choices,
            mains: sysfs::power_supplies_of_type(sysfs_root, "Mains")
                .into_iter()
                .next(),
            battery,
            caps,
        })
    }
}

impl Backend for PlatformProfile {
    fn name(&self) -> &'static str {
        "platform-profile"
    }

    /// The firmware handler's name — `thinkpad_acpi`, `amd-pmf`, `hp-wmi` — is
    /// what tells a bug report which driver is actually in play.
    fn model(&self) -> Option<String> {
        sysfs::read_opt(&self.profile.with_file_name("name"))
    }

    fn capabilities(&self) -> Capabilities {
        self.caps
    }

    fn read_state(&self) -> Result<HwState> {
        Ok(HwState {
            power_level: sysfs::read_opt(&self.profile).and_then(|v| Self::parse(&v)),
            fan_mode: None,
            cooler_boost: None,
            battery_saver: None,
            sensors: Sensors {
                cpu_temp_c: Self::temp_from(sysfs::hwmon_by_any_name(&self.root, CPU_HWMON)),
                gpu_temp_c: Self::temp_from(sysfs::hwmon_by_any_name(&self.root, GPU_HWMON)),
                // Duty percentages are a vendor EC idea; hwmon reports RPM.
                cpu_fan_percent: None,
                gpu_fan_percent: None,
                fan_rpm: Self::read_fan_rpm(&self.root),
            },
            battery: self.read_battery(),
        })
    }

    fn apply(&self, profile: &HwProfile) -> Result<()> {
        if let Some(level) = profile.power_level {
            let value = self
                .value_for(level)
                .ok_or(Error::Unsupported("this performance level"))?;
            sysfs::write(&self.profile, value)?;
        }
        if profile.fan_mode.is_some() {
            return Err(Error::Unsupported("fan modes"));
        }
        if profile.cooler_boost.is_some() {
            return Err(Error::Unsupported("cooler boost"));
        }
        if profile.battery_saver.is_some() {
            return Err(Error::Unsupported("battery saver"));
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
