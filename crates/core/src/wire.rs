//! Shared encoding for the D-Bus interface.
//!
//! D-Bus has no optional type, and every reading here is optional: laptops
//! differ in what they expose, and a sensor that is missing must not be
//! confused with one reading zero. So a snapshot travels as `a{sv}` where a
//! key is simply absent when the machine does not report it.
//!
//! Both the daemon and the client use these functions, so the key names have
//! one definition rather than two that drift apart.

use std::collections::HashMap;

use zvariant::{OwnedValue, Value};

use crate::gpu::Gpu;
use crate::types::{Battery, Capabilities, FanMode, HwState, PowerLevel, Sensors};

pub const POWER_LEVEL: &str = "power-level";
pub const FAN_MODE: &str = "fan-mode";
pub const COOLER_BOOST: &str = "cooler-boost";
pub const BATTERY_SAVER: &str = "battery-saver";
pub const CPU_TEMP: &str = "cpu-temp-c";
pub const GPU_TEMP: &str = "gpu-temp-c";
pub const CPU_FAN: &str = "cpu-fan-percent";
pub const GPU_FAN: &str = "gpu-fan-percent";
pub const FAN_RPM: &str = "fan-rpm";
pub const BATTERY_CAPACITY: &str = "battery-capacity";
pub const CHARGE_END_THRESHOLD: &str = "charge-end-threshold";
pub const CHARGE_START_THRESHOLD: &str = "charge-start-threshold";
pub const ON_AC: &str = "on-ac";
/// The discrete GPU's own readings, which come from the driver rather than
/// from a backend — see [`crate::gpu`]. Absent on machines without one.
pub const GPU_POWER: &str = "gpu-power-w";
pub const GPU_POWER_LIMIT: &str = "gpu-power-limit-w";
pub const GPU_CLOCK: &str = "gpu-clock-mhz";

pub type Dict = HashMap<String, OwnedValue>;

/// Encode a snapshot, omitting everything the hardware does not report.
pub fn state_to_dict(state: &HwState) -> Dict {
    let mut dict = Dict::new();
    insert_opt(
        &mut dict,
        POWER_LEVEL,
        state.power_level.map(|v| v.as_str()),
    );
    insert_opt(&mut dict, FAN_MODE, state.fan_mode.map(|v| v.as_str()));
    insert_opt(&mut dict, COOLER_BOOST, state.cooler_boost);
    insert_opt(&mut dict, BATTERY_SAVER, state.battery_saver);
    insert_opt(&mut dict, CPU_TEMP, state.sensors.cpu_temp_c);
    insert_opt(&mut dict, GPU_TEMP, state.sensors.gpu_temp_c);
    insert_opt(&mut dict, CPU_FAN, state.sensors.cpu_fan_percent);
    insert_opt(&mut dict, GPU_FAN, state.sensors.gpu_fan_percent);
    insert_opt(&mut dict, BATTERY_CAPACITY, state.battery.capacity_percent);
    insert_opt(
        &mut dict,
        CHARGE_END_THRESHOLD,
        state.battery.charge_end_threshold,
    );
    insert_opt(
        &mut dict,
        CHARGE_START_THRESHOLD,
        state.battery.charge_start_threshold,
    );
    insert_opt(&mut dict, ON_AC, state.battery.on_ac);

    // An empty fan list is meaningful — "this machine has no tachometers" — so
    // it is sent rather than omitted.
    insert(&mut dict, FAN_RPM, state.sensors.fan_rpm.clone());
    dict
}

/// Decode a snapshot. Unknown keys are ignored and missing ones stay `None`,
/// so an older client keeps working against a newer daemon.
pub fn state_from_dict(dict: &Dict) -> HwState {
    HwState {
        power_level: get::<String>(dict, POWER_LEVEL).and_then(|v| v.parse::<PowerLevel>().ok()),
        fan_mode: get::<String>(dict, FAN_MODE).and_then(|v| v.parse::<FanMode>().ok()),
        cooler_boost: get(dict, COOLER_BOOST),
        battery_saver: get(dict, BATTERY_SAVER),
        sensors: Sensors {
            cpu_temp_c: get(dict, CPU_TEMP),
            gpu_temp_c: get(dict, GPU_TEMP),
            cpu_fan_percent: get(dict, CPU_FAN),
            gpu_fan_percent: get(dict, GPU_FAN),
            fan_rpm: get_u32_list(dict, FAN_RPM).unwrap_or_default(),
        },
        battery: Battery {
            capacity_percent: get(dict, BATTERY_CAPACITY),
            charge_end_threshold: get(dict, CHARGE_END_THRESHOLD),
            charge_start_threshold: get(dict, CHARGE_START_THRESHOLD),
            on_ac: get(dict, ON_AC),
        },
    }
}

/// Add the GPU's readings to a snapshot, omitting whatever it does not report.
///
/// Kept apart from [`state_to_dict`] because this is not backend data: the
/// backend has a sysfs root and nothing else, and no NVIDIA reading arrives
/// that way. The daemon calls both; the shared dictionary is where they meet.
pub fn gpu_into_dict(dict: &mut Dict, gpu: &Gpu) {
    insert_opt(dict, GPU_POWER, gpu.power_w);
    insert_opt(dict, GPU_POWER_LIMIT, gpu.power_limit_w);
    insert_opt(dict, GPU_CLOCK, gpu.clock_mhz);
}

/// Decode the GPU's readings. All absent on a machine without one.
pub fn gpu_from_dict(dict: &Dict) -> Gpu {
    Gpu {
        power_w: get(dict, GPU_POWER),
        power_limit_w: get(dict, GPU_POWER_LIMIT),
        clock_mhz: get(dict, GPU_CLOCK),
    }
}

/// Capabilities travel as `a{sb}`: a plain map of what this machine can do.
pub fn caps_to_dict(caps: &Capabilities) -> HashMap<String, bool> {
    HashMap::from([
        (POWER_LEVEL.to_owned(), caps.power_level),
        (FAN_MODE.to_owned(), caps.fan_mode),
        (COOLER_BOOST.to_owned(), caps.cooler_boost),
        (BATTERY_SAVER.to_owned(), caps.battery_saver),
        (CHARGE_END_THRESHOLD.to_owned(), caps.charge_threshold),
        (
            CHARGE_START_THRESHOLD.to_owned(),
            caps.charge_start_threshold,
        ),
    ])
}

pub fn caps_from_dict(dict: &HashMap<String, bool>) -> Capabilities {
    let flag = |key: &str| dict.get(key).copied().unwrap_or(false);
    Capabilities {
        power_level: flag(POWER_LEVEL),
        fan_mode: flag(FAN_MODE),
        cooler_boost: flag(COOLER_BOOST),
        battery_saver: flag(BATTERY_SAVER),
        charge_threshold: flag(CHARGE_END_THRESHOLD),
        charge_start_threshold: flag(CHARGE_START_THRESHOLD),
    }
}

fn insert<'a, T: Into<Value<'a>>>(dict: &mut Dict, key: &str, value: T) {
    // A value that cannot be turned into an owned one would be a programming
    // error in the conversions above, not something a caller can trigger.
    if let Ok(owned) = OwnedValue::try_from(value.into()) {
        dict.insert(key.to_owned(), owned);
    }
}

fn insert_opt<'a, T: Into<Value<'a>>>(dict: &mut Dict, key: &str, value: Option<T>) {
    if let Some(value) = value {
        insert(dict, key, value);
    }
}

/// Arrays need their own path: `TryFrom<&Value>` is not implemented for `Vec`,
/// so the elements are converted one at a time.
fn get_u32_list(dict: &Dict, key: &str) -> Option<Vec<u32>> {
    let array = zvariant::Array::try_from(dict.get(key)?.clone()).ok()?;
    array
        .iter()
        .map(|value| u32::try_from(value).ok())
        .collect()
}

fn get<T>(dict: &Dict, key: &str) -> Option<T>
where
    T: for<'a> TryFrom<&'a Value<'a>>,
{
    T::try_from(dict.get(key)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HwState {
        HwState {
            power_level: Some(PowerLevel::PowerSaver),
            fan_mode: Some(FanMode::Silent),
            cooler_boost: Some(false),
            battery_saver: Some(true),
            sensors: Sensors {
                cpu_temp_c: Some(66),
                gpu_temp_c: Some(51),
                cpu_fan_percent: Some(70),
                gpu_fan_percent: Some(40),
                fan_rpm: vec![3555, 3555, 0, 0],
            },
            battery: Battery {
                capacity_percent: Some(99),
                charge_end_threshold: Some(80),
                charge_start_threshold: Some(75),
                on_ac: Some(true),
            },
        }
    }

    #[test]
    fn a_full_snapshot_survives_the_round_trip() {
        let state = sample();
        assert_eq!(state_from_dict(&state_to_dict(&state)), state);
    }

    #[test]
    fn absent_readings_are_omitted_rather_than_sent_as_zero() {
        let dict = state_to_dict(&HwState::default());

        assert!(!dict.contains_key(CPU_TEMP));
        assert!(!dict.contains_key(POWER_LEVEL));
        // ...and they decode back to absent, not to a reading of zero.
        assert_eq!(state_from_dict(&dict), HwState::default());
    }

    #[test]
    fn a_machine_without_tachometers_says_so_explicitly() {
        let dict = state_to_dict(&HwState::default());
        assert!(dict.contains_key(FAN_RPM), "fan list should always be sent");
        assert!(state_from_dict(&dict).sensors.fan_rpm.is_empty());
    }

    #[test]
    fn unknown_keys_from_a_newer_daemon_are_ignored() {
        let mut dict = state_to_dict(&sample());
        insert(&mut dict, "gpu-power-limit-w", 115u32);

        assert_eq!(state_from_dict(&dict), sample());
    }

    #[test]
    fn capabilities_round_trip() {
        let caps = Capabilities {
            power_level: true,
            fan_mode: true,
            cooler_boost: false,
            battery_saver: true,
            charge_threshold: false,
            charge_start_threshold: true,
        };
        assert_eq!(caps_from_dict(&caps_to_dict(&caps)), caps);
    }
}
