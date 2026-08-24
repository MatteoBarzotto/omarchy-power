//! `/etc/omarchy-power/config.toml`.
//!
//! The mapping from a power profile to hardware settings is a matter of taste
//! and of the machine, so it lives in a file rather than in the code. Missing
//! file means built-in defaults, which are what the MSI hardware wants.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use omarchy_power_core::types::{FanMode, HwProfile, PowerLevel};
use serde::Deserialize;

pub const DEFAULT_PATH: &str = "/etc/omarchy-power/config.toml";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields, default)]
pub struct Config {
    /// What each power-profiles-daemon profile means in hardware terms.
    pub profiles: HashMap<String, HwProfile>,
    /// Applied on top of the profile while running on battery.
    pub on_battery: HwProfile,
    pub thermal: Thermal,
}

/// Keeps the fans from being held quiet while the machine cooks.
///
/// Two thresholds rather than one: a single one would flap the fan mode back
/// and forth every few seconds around the trip point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields, default)]
pub struct Thermal {
    pub enabled: bool,
    /// At or above this CPU temperature, force the fans up.
    pub high_c: u8,
    /// Release the override only once the temperature is back down to this.
    pub low_c: u8,
    /// ...and has stayed there this long.
    pub cooldown_s: u64,
}

impl Default for Thermal {
    fn default() -> Self {
        Self {
            enabled: true,
            high_c: 90,
            low_c: 80,
            cooldown_s: 30,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            profiles: HashMap::from([
                (
                    "performance".to_owned(),
                    HwProfile {
                        power_level: Some(PowerLevel::Performance),
                        fan_mode: Some(FanMode::Aggressive),
                        battery_saver: Some(false),
                        ..HwProfile::default()
                    },
                ),
                (
                    "balanced".to_owned(),
                    HwProfile {
                        power_level: Some(PowerLevel::Balanced),
                        fan_mode: Some(FanMode::Auto),
                        battery_saver: Some(false),
                        ..HwProfile::default()
                    },
                ),
                (
                    "power-saver".to_owned(),
                    HwProfile {
                        power_level: Some(PowerLevel::PowerSaver),
                        fan_mode: Some(FanMode::Silent),
                        battery_saver: Some(true),
                        ..HwProfile::default()
                    },
                ),
            ]),
            // Nothing by default: overriding the user's chosen profile the
            // moment they unplug is a surprise, not a feature. Opt in.
            on_battery: HwProfile::default(),
            thermal: Thermal::default(),
        }
    }
}

impl Config {
    /// Load the config, falling back to defaults when the file is absent.
    ///
    /// A malformed file is an error rather than a silent fallback: quietly
    /// ignoring a config the user just edited is the worst possible answer.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "no config file; using defaults");
                Ok(Self::default())
            }
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// The hardware settings for a PPD profile name.
    ///
    /// An unknown name means a newer power-profiles-daemon grew a profile we
    /// have never heard of; leaving the hardware alone beats guessing.
    pub fn for_profile(&self, name: &str) -> Option<HwProfile> {
        self.profiles.get(name).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_every_profile_power_profiles_daemon_ships() {
        let config = Config::default();
        for name in ["performance", "balanced", "power-saver"] {
            assert!(config.for_profile(name).is_some(), "{name} unmapped");
        }
    }

    #[test]
    fn an_unknown_profile_maps_to_nothing_rather_than_a_guess() {
        assert_eq!(Config::default().for_profile("turbo-plus"), None);
    }

    #[test]
    fn a_partial_config_keeps_the_defaults_for_everything_else() {
        let config: Config = toml::from_str(
            r#"
            [thermal]
            high-c = 95
            "#,
        )
        .unwrap();

        assert_eq!(config.thermal.high_c, 95);
        assert_eq!(config.thermal.low_c, Thermal::default().low_c);
        assert!(
            !config.profiles.is_empty(),
            "profiles should fall back to defaults"
        );
    }

    #[test]
    fn profiles_are_written_in_the_neutral_vocabulary() {
        let config: Config = toml::from_str(
            r#"
            [profiles.balanced]
            power-level = "balanced"
            fan-mode = "silent"
            "#,
        )
        .unwrap();

        assert_eq!(
            config.for_profile("balanced"),
            Some(HwProfile {
                power_level: Some(PowerLevel::Balanced),
                fan_mode: Some(FanMode::Silent),
                ..HwProfile::default()
            })
        );
    }

    #[test]
    fn a_typo_is_an_error_rather_than_a_setting_that_never_applies() {
        let bad = toml::from_str::<Config>(
            r#"
            [profiles.balanced]
            power_level = "balanced"
            "#,
        );
        assert!(bad.is_err(), "underscore spelling should be rejected");

        let worse = toml::from_str::<Config>(
            r#"
            [profiles.balanced]
            fan-mode = "quiet"
            "#,
        );
        assert!(worse.is_err(), "unknown fan mode should be rejected");
    }
}
