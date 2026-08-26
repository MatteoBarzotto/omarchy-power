//! The kernel-interface backend, exercised against two hand-built sysfs trees.
//!
//! Unlike `fixtures/msi-katana` these were written from the kernel's
//! documentation rather than captured off a machine — the whole point of this
//! backend is that it is driven by interfaces the kernel defines, so the
//! documentation is the authority. A real capture from a ThinkPad or Framework
//! is still welcome and would replace them.

use std::fs;
use std::path::{Path, PathBuf};

use omarchy_power_core::backend::{Backend, Error, Probe};
use omarchy_power_core::backends::platform_profile::PlatformProfile;
use omarchy_power_core::types::{FanMode, HwProfile, PowerLevel};
use omarchy_power_core::{detect_in, sysfs};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

/// Copy the ACPI fixture somewhere writable, so `apply` — and edits to the
/// tree itself — can be tested for real.
///
/// `label` is the calling test's own name: tests run in parallel and a shared
/// destination would have them editing each other's files.
fn writable_copy(label: &str) -> PathBuf {
    let dest =
        std::env::temp_dir().join(format!("omarchy-power-test-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dest);
    copy_tree(&fixture("platform-profile-acpi"), &dest);
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
fn detects_a_machine_the_kernel_already_covers() {
    let backend = detect_in(&fixture("platform-profile-acpi")).expect("should be recognised");
    assert_eq!(backend.name(), "platform-profile");
}

#[test]
fn the_firmware_handler_is_reported_as_the_model() {
    // Only the class interface names its handler; the ACPI file has no such
    // attribute, and inventing one would put a guess into bug reports.
    let class = detect_in(&fixture("platform-profile-class")).unwrap();
    assert_eq!(class.model().as_deref(), Some("amd-pmf"));

    let acpi = detect_in(&fixture("platform-profile-acpi")).unwrap();
    assert_eq!(acpi.model(), None);
}

#[test]
fn the_newer_class_interface_wins_over_the_file_kept_for_compatibility() {
    // Both are present in this fixture and deliberately disagree.
    let state = detect_in(&fixture("platform-profile-class"))
        .unwrap()
        .read_state()
        .unwrap();
    assert_eq!(state.power_level, Some(PowerLevel::PowerSaver));
}

#[test]
fn only_what_the_kernel_defines_is_claimed_as_a_capability() {
    let caps = PlatformProfile::probe(&fixture("platform-profile-acpi"))
        .unwrap()
        .capabilities();
    assert!(caps.power_level);
    assert!(caps.charge_threshold);
    assert!(caps.charge_start_threshold);
    // No standard interface exposes these, so they must not be offered.
    assert!(!caps.fan_mode);
    assert!(!caps.cooler_boost);
    assert!(!caps.battery_saver);
}

#[test]
fn a_battery_without_a_threshold_file_loses_only_that_capability() {
    let caps = PlatformProfile::probe(&fixture("platform-profile-class"))
        .unwrap()
        .capabilities();
    assert!(caps.power_level);
    assert!(!caps.charge_threshold);
}

#[test]
fn sensors_come_from_hwmon_in_whole_degrees() {
    let state = detect_in(&fixture("platform-profile-acpi"))
        .unwrap()
        .read_state()
        .unwrap();

    // 48000 millidegrees from coretemp.
    assert_eq!(state.sensors.cpu_temp_c, Some(48));
    assert_eq!(state.sensors.gpu_temp_c, None, "no GPU chip in this tree");
    assert_eq!(state.sensors.fan_rpm, vec![2412]);
    // Duty percentages are a vendor EC idea and must stay absent rather than
    // being reported as zero.
    assert_eq!(state.sensors.cpu_fan_percent, None);
    assert_eq!(state.battery.capacity_percent, Some(71));
    assert_eq!(state.battery.charge_end_threshold, Some(80));
    assert_eq!(state.battery.on_ac, Some(false));
}

#[test]
fn applying_a_level_writes_a_name_this_firmware_actually_offers() {
    let root = writable_copy("applies-a-level");
    let backend = PlatformProfile::probe(&root).unwrap();

    backend
        .apply(&HwProfile {
            power_level: Some(PowerLevel::PowerSaver),
            ..HwProfile::default()
        })
        .unwrap();
    let profile = root.join("firmware/acpi/platform_profile");
    assert_eq!(sysfs::read(&profile).unwrap(), "low-power");

    backend
        .apply(&HwProfile {
            power_level: Some(PowerLevel::Performance),
            ..HwProfile::default()
        })
        .unwrap();
    assert_eq!(sysfs::read(&profile).unwrap(), "performance");
}

#[test]
fn a_firmware_without_the_obvious_name_gets_the_next_best_one() {
    // Some firmware offers `quiet` and nothing called `low-power`; asking for
    // a power saver must still land somewhere sensible rather than failing.
    let root = writable_copy("next-best-name");
    fs::write(
        root.join("firmware/acpi/platform_profile_choices"),
        "quiet balanced performance\n",
    )
    .unwrap();

    let backend = PlatformProfile::probe(&root).unwrap();
    backend
        .apply(&HwProfile {
            power_level: Some(PowerLevel::PowerSaver),
            ..HwProfile::default()
        })
        .unwrap();

    assert_eq!(
        sysfs::read(&root.join("firmware/acpi/platform_profile")).unwrap(),
        "quiet"
    );
}

#[test]
fn balanced_performance_reads_as_balanced_rather_than_as_full_power() {
    let root = writable_copy("balanced-performance");
    fs::write(
        root.join("firmware/acpi/platform_profile"),
        "balanced-performance\n",
    )
    .unwrap();

    let state = PlatformProfile::probe(&root).unwrap().read_state().unwrap();
    assert_eq!(state.power_level, Some(PowerLevel::Balanced));
}

#[test]
fn a_profile_we_have_no_name_for_reads_as_nothing_rather_than_as_a_guess() {
    let root = writable_copy("unknown-profile");
    // `custom` means the firmware follows its own settings.
    fs::write(root.join("firmware/acpi/platform_profile"), "custom\n").unwrap();

    let state = PlatformProfile::probe(&root).unwrap().read_state().unwrap();
    assert_eq!(state.power_level, None);
}

#[test]
fn asking_for_what_this_layer_cannot_do_fails_before_any_write() {
    let root = writable_copy("unsupported");
    let backend = PlatformProfile::probe(&root).unwrap();

    let result = backend.apply(&HwProfile {
        fan_mode: Some(FanMode::Silent),
        ..HwProfile::default()
    });
    assert!(matches!(result, Err(Error::Unsupported("fan modes"))));
}

#[test]
fn a_switch_offering_nothing_we_recognise_is_not_claimed() {
    let root = writable_copy("unrecognised-choices");
    fs::write(
        root.join("firmware/acpi/platform_profile_choices"),
        "mystery-mode another-one\n",
    )
    .unwrap();

    assert!(
        PlatformProfile::probe(&root).is_none(),
        "a backend whose every key press would fail is worse than none"
    );
}

#[test]
fn a_vendor_driver_is_preferred_over_the_generic_interface() {
    // The MSI fixture has no platform_profile, so the order only matters on a
    // machine where both could match; build that case explicitly.
    let root = writable_copy("vendor-wins");
    let ec = root.join("devices/platform/msi-ec");
    fs::create_dir_all(&ec).unwrap();
    fs::write(ec.join("shift_mode"), "comfort\n").unwrap();

    let backend = detect_in(&root).unwrap();
    assert_eq!(
        backend.name(),
        "msi-ec",
        "the vendor driver reaches fan modes the generic one cannot"
    );
}
