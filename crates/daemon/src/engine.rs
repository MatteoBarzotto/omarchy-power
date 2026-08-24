//! Deciding what the hardware should be doing, and putting it there.
//!
//! Three inputs feed one decision: the profile power-profiles-daemon reports,
//! whether we are on battery, and whether the thermal guard has tripped. The
//! decision itself is a pure function so the interesting cases can be tested
//! without hardware, a bus, or a wait for the machine to actually get hot.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use omarchy_power_core::types::{FanMode, HwProfile};
use omarchy_power_core::{Backend, Result};

use crate::config::{Config, Thermal};

/// Everything the decision depends on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inputs {
    /// The active power-profiles-daemon profile, when it is running.
    pub ppd_profile: Option<String>,
    pub on_battery: bool,
    pub thermal_tripped: bool,
}

/// What the hardware should look like, given the inputs.
pub fn decide(config: &Config, inputs: &Inputs) -> HwProfile {
    let mut profile = inputs
        .ppd_profile
        .as_deref()
        .and_then(|name| config.for_profile(name))
        .unwrap_or_default();

    if inputs.on_battery {
        overlay(&mut profile, &config.on_battery);
    }

    // Last word, on purpose: no profile and no config setting may keep the fans
    // quiet while the machine is overheating.
    if inputs.thermal_tripped {
        profile.fan_mode = Some(FanMode::Aggressive);
    }

    profile
}

/// Copy every field the overlay sets, leaving the rest alone.
fn overlay(base: &mut HwProfile, over: &HwProfile) {
    if over.power_level.is_some() {
        base.power_level = over.power_level;
    }
    if over.fan_mode.is_some() {
        base.fan_mode = over.fan_mode;
    }
    if over.cooler_boost.is_some() {
        base.cooler_boost = over.cooler_boost;
    }
    if over.battery_saver.is_some() {
        base.battery_saver = over.battery_saver;
    }
    if over.charge_end_threshold.is_some() {
        base.charge_end_threshold = over.charge_end_threshold;
    }
}

/// Trips when the CPU gets hot, and refuses to untrip until it has properly
/// cooled down and stayed that way.
#[derive(Debug)]
pub struct ThermalGuard {
    config: Thermal,
    tripped: bool,
    /// When the temperature first dropped to the safe level.
    cool_since: Option<Instant>,
}

impl ThermalGuard {
    pub fn new(config: Thermal) -> Self {
        Self {
            config,
            tripped: false,
            cool_since: None,
        }
    }

    /// Feed in a temperature reading; returns whether the guard is tripped.
    ///
    /// `now` is a parameter so tests do not have to sleep.
    pub fn update(&mut self, cpu_temp_c: Option<u8>, now: Instant) -> bool {
        if !self.config.enabled {
            return false;
        }
        // A machine that stopped reporting temperatures is not a machine that
        // got cold; hold whatever the guard already decided.
        let Some(temp) = cpu_temp_c else {
            return self.tripped;
        };

        if temp >= self.config.high_c {
            if !self.tripped {
                tracing::warn!(
                    temp,
                    threshold = self.config.high_c,
                    "thermal guard tripped"
                );
            }
            self.tripped = true;
            self.cool_since = None;
        } else if self.tripped {
            if temp <= self.config.low_c {
                let since = *self.cool_since.get_or_insert(now);
                if now.duration_since(since) >= Duration::from_secs(self.config.cooldown_s) {
                    tracing::info!(temp, "thermal guard released");
                    self.tripped = false;
                    self.cool_since = None;
                }
            } else {
                // Back above the low mark: the cooldown starts over.
                self.cool_since = None;
            }
        }

        self.tripped
    }
}

/// Owns the hardware and the current picture of the world.
pub struct Engine {
    backend: Box<dyn Backend>,
    config: Config,
    state: Mutex<State>,
}

struct State {
    inputs: Inputs,
    guard: ThermalGuard,
    /// The last profile we applied, so unchanged decisions cost no sysfs writes
    /// and a manual change through D-Bus is not immediately undone.
    applied: Option<HwProfile>,
}

impl Engine {
    pub fn new(backend: Box<dyn Backend>, config: Config) -> Self {
        let guard = ThermalGuard::new(config.thermal);
        Self {
            backend,
            config,
            state: Mutex::new(State {
                inputs: Inputs::default(),
                guard,
                applied: None,
            }),
        }
    }

    pub fn backend(&self) -> &dyn Backend {
        self.backend.as_ref()
    }

    /// Apply a profile a client asked for, bypassing the decision entirely.
    ///
    /// The automation does not fight it: the next reconcile only writes if the
    /// decided profile has changed since it was last applied.
    pub fn apply_manual(&self, profile: &HwProfile) -> Result<()> {
        tracing::info!(?profile, "applying (manual)");
        self.backend.apply(profile)
    }

    pub fn set_ppd_profile(&self, name: Option<String>) {
        self.with_inputs(|inputs| inputs.ppd_profile = name);
    }

    pub fn set_on_battery(&self, on_battery: bool) {
        self.with_inputs(|inputs| inputs.on_battery = on_battery);
    }

    /// Read the temperature and let the guard decide, then reconcile.
    pub fn tick_thermal(&self) {
        let temp = self
            .backend
            .read_state()
            .ok()
            .and_then(|state| state.sensors.cpu_temp_c);

        {
            let mut state = self.state.lock().expect("engine state");
            let tripped = state.guard.update(temp, Instant::now());
            state.inputs.thermal_tripped = tripped;
        }
        self.reconcile();
    }

    fn with_inputs(&self, edit: impl FnOnce(&mut Inputs)) {
        {
            let mut state = self.state.lock().expect("engine state");
            edit(&mut state.inputs);
        }
        self.reconcile();
    }

    /// Bring the hardware in line with the current decision, if it has moved.
    pub fn reconcile(&self) {
        let profile = {
            let mut state = self.state.lock().expect("engine state");
            let profile = decide(&self.config, &state.inputs);

            if profile.is_empty() || state.applied == Some(profile) {
                return;
            }
            state.applied = Some(profile);
            profile
        };

        tracing::info!(?profile, "applying (automatic)");
        if let Err(e) = self.backend.apply(&profile) {
            tracing::error!(error = %e, "could not apply profile");
            // Forget it, so the next event tries again rather than assuming the
            // hardware is in a state it never reached.
            self.state.lock().expect("engine state").applied = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omarchy_power_core::types::PowerLevel;

    fn config() -> Config {
        Config::default()
    }

    #[test]
    fn a_profile_maps_straight_through_when_nothing_else_applies() {
        let profile = decide(
            &config(),
            &Inputs {
                ppd_profile: Some("power-saver".to_owned()),
                ..Inputs::default()
            },
        );

        assert_eq!(profile.power_level, Some(PowerLevel::PowerSaver));
        assert_eq!(profile.fan_mode, Some(FanMode::Silent));
        assert_eq!(profile.battery_saver, Some(true));
    }

    #[test]
    fn the_battery_overlay_only_replaces_what_it_sets() {
        let mut config = config();
        config.on_battery = HwProfile {
            power_level: Some(PowerLevel::PowerSaver),
            ..HwProfile::default()
        };

        let profile = decide(
            &config,
            &Inputs {
                ppd_profile: Some("performance".to_owned()),
                on_battery: true,
                ..Inputs::default()
            },
        );

        assert_eq!(profile.power_level, Some(PowerLevel::PowerSaver));
        assert_eq!(
            profile.fan_mode,
            Some(FanMode::Aggressive),
            "the profile's own fan mode should survive"
        );
    }

    #[test]
    fn nothing_may_keep_the_fans_quiet_while_overheating() {
        let profile = decide(
            &config(),
            &Inputs {
                ppd_profile: Some("power-saver".to_owned()),
                thermal_tripped: true,
                ..Inputs::default()
            },
        );

        assert_eq!(profile.fan_mode, Some(FanMode::Aggressive));
        // ...but the rest of the quiet profile is left alone.
        assert_eq!(profile.power_level, Some(PowerLevel::PowerSaver));
    }

    #[test]
    fn an_unknown_profile_changes_nothing() {
        let profile = decide(
            &config(),
            &Inputs {
                ppd_profile: Some("hyperdrive".to_owned()),
                ..Inputs::default()
            },
        );
        assert!(profile.is_empty());
    }

    fn guard() -> ThermalGuard {
        ThermalGuard::new(Thermal {
            enabled: true,
            high_c: 90,
            low_c: 80,
            cooldown_s: 30,
        })
    }

    #[test]
    fn the_guard_trips_at_the_high_mark() {
        let mut guard = guard();
        let now = Instant::now();

        assert!(!guard.update(Some(89), now));
        assert!(guard.update(Some(90), now));
    }

    #[test]
    fn the_guard_holds_until_it_has_been_cool_for_long_enough() {
        let mut guard = guard();
        let start = Instant::now();
        guard.update(Some(95), start);

        // Cool enough, but not for long enough yet.
        assert!(guard.update(Some(75), start));
        assert!(guard.update(Some(75), start + Duration::from_secs(29)));
        assert!(!guard.update(Some(75), start + Duration::from_secs(30)));
    }

    #[test]
    fn a_temperature_between_the_marks_does_not_start_the_cooldown() {
        let mut guard = guard();
        let start = Instant::now();
        guard.update(Some(95), start);

        // 85 is below the trip point but above the release point: the classic
        // spot where a single-threshold guard would flap.
        assert!(guard.update(Some(85), start));
        assert!(guard.update(Some(85), start + Duration::from_secs(600)));
    }

    #[test]
    fn a_brief_dip_does_not_count_towards_the_cooldown() {
        let mut guard = guard();
        let start = Instant::now();
        guard.update(Some(95), start);

        assert!(guard.update(Some(75), start));
        assert!(guard.update(Some(88), start + Duration::from_secs(10)));
        // The clock restarts from here, so the original deadline is not enough.
        assert!(guard.update(Some(75), start + Duration::from_secs(20)));
        assert!(guard.update(Some(75), start + Duration::from_secs(45)));
        assert!(!guard.update(Some(75), start + Duration::from_secs(51)));
    }

    #[test]
    fn losing_the_sensor_holds_the_current_verdict() {
        let mut guard = guard();
        let now = Instant::now();
        guard.update(Some(95), now);

        assert!(
            guard.update(None, now + Duration::from_secs(3600)),
            "a missing reading is not evidence of cooling"
        );
    }

    #[test]
    fn a_disabled_guard_never_trips() {
        let mut guard = ThermalGuard::new(Thermal {
            enabled: false,
            ..Thermal::default()
        });
        assert!(!guard.update(Some(120), Instant::now()));
    }
}
