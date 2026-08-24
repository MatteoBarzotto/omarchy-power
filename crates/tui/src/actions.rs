//! Turning keypresses into intentions.
//!
//! Kept apart from the event loop and from the bus so it can be tested without
//! a terminal or a daemon: the interesting behaviour here is what a key means
//! given the current hardware state, not how it gets delivered.

use omarchy_power_core::types::{Capabilities, FanMode, HwState, PowerLevel};
use ratatui::crossterm::event::KeyCode;

/// How far one keypress moves the charge limit.
const CHARGE_STEP: u8 = 5;
/// Matches the range the backends accept.
const CHARGE_MIN: u8 = 20;
const CHARGE_MAX: u8 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Refresh,
    SetPowerLevel(PowerLevel),
    SetFanMode(FanMode),
    SetCoolerBoost(bool),
    SetBatterySaver(bool),
    SetChargeThreshold(u8),
    /// The key maps to something this machine cannot do.
    Unsupported(&'static str),
    /// The key means nothing here.
    Ignored,
}

pub fn for_key(code: KeyCode, state: &HwState, caps: Capabilities) -> Action {
    match code {
        KeyCode::Char('q' | 'Q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('r' | 'R') => Action::Refresh,

        KeyCode::Char('p' | 'P') => gated(caps.power_level, "power levels", || {
            Action::SetPowerLevel(next(&PowerLevel::ALL, state.power_level))
        }),
        KeyCode::Char('f' | 'F') => gated(caps.fan_mode, "fan modes", || {
            Action::SetFanMode(next(&FanMode::ALL, state.fan_mode))
        }),
        KeyCode::Char('b' | 'B') => gated(caps.cooler_boost, "cooler boost", || {
            Action::SetCoolerBoost(!state.cooler_boost.unwrap_or(false))
        }),
        KeyCode::Char('s' | 'S') => gated(caps.battery_saver, "battery saver", || {
            Action::SetBatterySaver(!state.battery_saver.unwrap_or(false))
        }),

        KeyCode::Char('+' | '=') => charge(state, caps, CHARGE_STEP as i16),
        KeyCode::Char('-' | '_') => charge(state, caps, -(CHARGE_STEP as i16)),

        _ => Action::Ignored,
    }
}

fn gated(supported: bool, what: &'static str, action: impl FnOnce() -> Action) -> Action {
    if supported {
        action()
    } else {
        Action::Unsupported(what)
    }
}

/// Step the charge limit, staying inside the accepted range.
///
/// Without a current reading there is nothing to step from, so the key is
/// ignored rather than guessing a starting point.
fn charge(state: &HwState, caps: Capabilities, delta: i16) -> Action {
    if !caps.charge_threshold {
        return Action::Unsupported("charge thresholds");
    }
    let Some(current) = state.battery.charge_end_threshold else {
        return Action::Ignored;
    };
    let next = (i16::from(current) + delta).clamp(CHARGE_MIN.into(), CHARGE_MAX.into());
    Action::SetChargeThreshold(next as u8)
}

/// The next value in a fixed cycle, wrapping around.
///
/// An unknown current value starts the cycle from the beginning, which is what
/// a laptop reporting something we do not recognise should do on a keypress.
fn next<T: Copy + PartialEq>(all: &[T], current: Option<T>) -> T {
    let index = current
        .and_then(|c| all.iter().position(|v| *v == c))
        .map_or(0, |i| (i + 1) % all.len());
    all[index]
}

#[cfg(test)]
mod tests {
    use super::*;
    use omarchy_power_core::types::Battery;

    fn caps() -> Capabilities {
        Capabilities {
            power_level: true,
            fan_mode: true,
            cooler_boost: true,
            battery_saver: true,
            charge_threshold: true,
        }
    }

    fn state() -> HwState {
        HwState {
            power_level: Some(PowerLevel::Balanced),
            fan_mode: Some(FanMode::Auto),
            cooler_boost: Some(false),
            battery_saver: Some(false),
            battery: Battery {
                charge_end_threshold: Some(80),
                ..Battery::default()
            },
            ..HwState::default()
        }
    }

    fn press(c: char, state: &HwState, caps: Capabilities) -> Action {
        for_key(KeyCode::Char(c), state, caps)
    }

    #[test]
    fn cycling_advances_from_the_current_value_and_wraps() {
        let mut state = state();
        assert_eq!(
            press('p', &state, caps()),
            Action::SetPowerLevel(PowerLevel::PowerSaver)
        );

        state.power_level = Some(PowerLevel::PowerSaver);
        assert_eq!(
            press('p', &state, caps()),
            Action::SetPowerLevel(PowerLevel::Performance),
            "cycle should wrap around"
        );
    }

    #[test]
    fn toggles_flip_whatever_the_hardware_reports() {
        let mut state = state();
        assert_eq!(press('b', &state, caps()), Action::SetCoolerBoost(true));

        state.cooler_boost = Some(true);
        assert_eq!(press('b', &state, caps()), Action::SetCoolerBoost(false));
    }

    #[test]
    fn keys_for_missing_hardware_report_that_rather_than_failing_later() {
        let state = state();
        let none = Capabilities::default();

        assert_eq!(
            press('b', &state, none),
            Action::Unsupported("cooler boost")
        );
        assert_eq!(press('f', &state, none), Action::Unsupported("fan modes"));
        assert_eq!(
            press('+', &state, none),
            Action::Unsupported("charge thresholds")
        );
    }

    #[test]
    fn the_charge_limit_steps_and_stops_at_the_ends() {
        let mut state = state();
        assert_eq!(press('+', &state, caps()), Action::SetChargeThreshold(85));
        assert_eq!(press('-', &state, caps()), Action::SetChargeThreshold(75));

        state.battery.charge_end_threshold = Some(100);
        assert_eq!(
            press('+', &state, caps()),
            Action::SetChargeThreshold(100),
            "should clamp instead of overflowing"
        );

        state.battery.charge_end_threshold = Some(20);
        assert_eq!(press('-', &state, caps()), Action::SetChargeThreshold(20));
    }

    #[test]
    fn an_unknown_current_value_starts_the_cycle_rather_than_doing_nothing() {
        let state = HwState::default();
        assert_eq!(
            press('p', &state, caps()),
            Action::SetPowerLevel(PowerLevel::Performance)
        );
    }

    #[test]
    fn unrelated_keys_are_ignored() {
        let state = state();
        assert_eq!(press('x', &state, caps()), Action::Ignored);
        assert_eq!(for_key(KeyCode::Enter, &state, caps()), Action::Ignored);
    }
}
