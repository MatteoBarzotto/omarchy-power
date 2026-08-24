//! The MSI backend, exercised against a captured sysfs tree.
//!
//! `fixtures/msi-katana` came off a real machine, so these tests cover the
//! actual attribute vocabulary of the driver rather than one invented for the
//! test. Nothing here needs MSI hardware — or root.

use std::fs;
use std::path::{Path, PathBuf};

use omarchy_power_core::backend::{Backend, Error, Probe};
use omarchy_power_core::backends::msi::Msi;
use omarchy_power_core::types::{FanMode, HwProfile, PowerLevel};
use omarchy_power_core::{detect_in, sysfs};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/msi-katana")
}

/// Copy a fixture somewhere writable, so `apply` can be tested for real.
fn writable_copy(name: &str) -> PathBuf {
    let dest =
        std::env::temp_dir().join(format!("omarchy-power-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dest);
    copy_tree(&fixture(), &dest);
    dest
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[test]
fn detects_msi_from_a_captured_tree() {
    let backend = detect_in(&fixture()).expect("fixture should be recognised");
    assert_eq!(backend.name(), "msi-ec");
    assert_eq!(backend.model().as_deref(), Some("1587EMS1.106"));
}

#[test]
fn reports_no_backend_for_unrelated_hardware() {
    let empty = std::env::temp_dir().join("omarchy-power-test-empty");
    fs::create_dir_all(&empty).unwrap();
    assert!(matches!(detect_in(&empty), Err(Error::NoBackend)));
}

#[test]
fn capabilities_follow_the_attributes_that_exist() {
    let caps = Msi::probe(&fixture()).unwrap().capabilities();
    assert!(caps.power_level);
    assert!(caps.fan_mode);
    assert!(caps.cooler_boost);
    assert!(caps.battery_saver);
    assert!(caps.charge_threshold);
}

#[test]
fn missing_attributes_turn_into_missing_capabilities() {
    let root = writable_copy("nocooler");
    fs::remove_file(root.join("devices/platform/msi-ec/cooler_boost")).unwrap();

    let backend = Msi::probe(&root).unwrap();
    assert!(!backend.capabilities().cooler_boost);

    // ...and asking for it fails loudly instead of writing to a missing file.
    let err = backend
        .apply(&HwProfile {
            cooler_boost: Some(true),
            ..HwProfile::default()
        })
        .unwrap_err();
    assert!(matches!(err, Error::Unsupported("cooler boost")));
}

#[test]
fn reads_the_vendor_vocabulary_into_neutral_terms() {
    let state = Msi::probe(&fixture()).unwrap().read_state().unwrap();

    assert_eq!(state.power_level, Some(PowerLevel::Balanced)); // "comfort"
    assert_eq!(state.fan_mode, Some(FanMode::Auto));
    assert_eq!(state.cooler_boost, Some(false)); // "off"
    assert_eq!(state.battery_saver, Some(false));
    assert_eq!(state.sensors.cpu_temp_c, Some(66));
    assert_eq!(state.sensors.gpu_temp_c, Some(51));
    assert_eq!(state.sensors.cpu_fan_percent, Some(70));
    assert_eq!(state.battery.capacity_percent, Some(99));
    assert_eq!(state.battery.charge_end_threshold, Some(80));
    assert_eq!(state.battery.on_ac, Some(true));
}

#[test]
fn fan_rpm_keeps_idle_fans_and_stops_at_the_first_gap() {
    let state = Msi::probe(&fixture()).unwrap().read_state().unwrap();
    // Four fans exist; the two idle ones report 0 and must not be dropped.
    assert_eq!(state.sensors.fan_rpm, vec![3555, 3555, 0, 0]);
}

#[test]
fn applying_a_profile_writes_the_vendor_words() {
    let root = writable_copy("apply");
    let backend = Msi::probe(&root).unwrap();

    backend
        .apply(&HwProfile {
            power_level: Some(PowerLevel::PowerSaver),
            fan_mode: Some(FanMode::Aggressive),
            battery_saver: Some(true),
            charge_end_threshold: Some(75),
            ..HwProfile::default()
        })
        .unwrap();

    let ec = root.join("devices/platform/msi-ec");
    assert_eq!(sysfs::read(&ec.join("shift_mode")).unwrap(), "eco");
    assert_eq!(sysfs::read(&ec.join("fan_mode")).unwrap(), "advanced");
    assert_eq!(sysfs::read(&ec.join("super_battery")).unwrap(), "on");
    assert_eq!(
        sysfs::read(&root.join("class/power_supply/BAT1/charge_control_end_threshold")).unwrap(),
        "75"
    );
}

#[test]
fn fields_left_unset_are_not_touched() {
    let root = writable_copy("untouched");
    let backend = Msi::probe(&root).unwrap();

    backend
        .apply(&HwProfile {
            power_level: Some(PowerLevel::Performance),
            ..HwProfile::default()
        })
        .unwrap();

    let ec = root.join("devices/platform/msi-ec");
    assert_eq!(sysfs::read(&ec.join("shift_mode")).unwrap(), "turbo");
    assert_eq!(sysfs::read(&ec.join("fan_mode")).unwrap(), "auto");
    assert_eq!(sysfs::read(&ec.join("cooler_boost")).unwrap(), "off");
}

#[test]
fn absurd_charge_thresholds_are_refused_before_reaching_the_kernel() {
    let root = writable_copy("threshold");
    let backend = Msi::probe(&root).unwrap();

    let err = backend
        .apply(&HwProfile {
            charge_end_threshold: Some(5),
            ..HwProfile::default()
        })
        .unwrap_err();
    assert!(matches!(err, Error::BadValue("charge threshold", _)));

    // The old value survives a rejected write.
    assert_eq!(
        sysfs::read(&root.join("class/power_supply/BAT1/charge_control_end_threshold")).unwrap(),
        "80"
    );
}
