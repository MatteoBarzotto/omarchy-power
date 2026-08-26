//! Charge thresholds, which both backends share through `core::battery`.
//!
//! The pair is the interesting part. An end threshold alone leaves a battery on
//! mains cycling 79-80-79 forever, which is the wear the limit was set to
//! avoid; the start threshold is what stops that, and the kernel constrains the
//! two against each other.

use std::fs;
use std::path::{Path, PathBuf};

use omarchy_power_core::backend::{Backend, Error, Probe};
use omarchy_power_core::backends::msi::Msi;
use omarchy_power_core::sysfs;
use omarchy_power_core::types::HwProfile;

const BAT: &str = "class/power_supply/BAT1";
const END: &str = "charge_control_end_threshold";
const START: &str = "charge_control_start_threshold";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/msi-katana")
}

/// A writable copy, named after the calling test: these run in parallel and a
/// shared destination would have them editing each other's files.
fn writable_copy(label: &str) -> PathBuf {
    let dest =
        std::env::temp_dir().join(format!("omarchy-power-test-{label}-{}", std::process::id()));
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

fn read(root: &Path, attr: &str) -> String {
    sysfs::read(&root.join(BAT).join(attr)).unwrap()
}

#[test]
fn both_halves_of_the_limit_are_reported() {
    let state = Msi::probe(&fixture()).unwrap().read_state().unwrap();
    assert_eq!(state.battery.charge_end_threshold, Some(80));
    assert_eq!(state.battery.charge_start_threshold, Some(70));
}

#[test]
fn a_driver_without_a_start_file_loses_only_that_capability() {
    let root = writable_copy("no-start-file");
    fs::remove_file(root.join(BAT).join(START)).unwrap();

    let caps = Msi::probe(&root).unwrap().capabilities();
    assert!(caps.charge_threshold, "the end threshold is still there");
    assert!(!caps.charge_start_threshold);
}

#[test]
fn asking_for_a_start_threshold_the_driver_lacks_fails_rather_than_writing() {
    let root = writable_copy("start-unsupported");
    fs::remove_file(root.join(BAT).join(START)).unwrap();
    let backend = Msi::probe(&root).unwrap();

    let result = backend.apply(&HwProfile {
        charge_start_threshold: Some(70),
        ..HwProfile::default()
    });
    assert!(matches!(
        result,
        Err(Error::Unsupported("a start threshold"))
    ));
}

#[test]
fn raising_the_pair_moves_the_end_first() {
    // Writing start=85 while the end is still 80 is what the kernel refuses,
    // so the order is the whole behaviour here.
    let root = writable_copy("raising");
    let backend = Msi::probe(&root).unwrap();

    backend
        .apply(&HwProfile {
            charge_end_threshold: Some(95),
            charge_start_threshold: Some(85),
            ..HwProfile::default()
        })
        .unwrap();

    assert_eq!(read(&root, END), "95");
    assert_eq!(read(&root, START), "85");
}

#[test]
fn lowering_the_pair_moves_the_start_first() {
    let root = writable_copy("lowering");
    let backend = Msi::probe(&root).unwrap();

    backend
        .apply(&HwProfile {
            charge_end_threshold: Some(60),
            charge_start_threshold: Some(50),
            ..HwProfile::default()
        })
        .unwrap();

    assert_eq!(read(&root, END), "60");
    assert_eq!(read(&root, START), "50");
}

#[test]
fn a_start_at_or_above_the_end_is_refused_before_anything_is_written() {
    let root = writable_copy("bad-order");
    let backend = Msi::probe(&root).unwrap();

    // The end threshold in the fixture is 80.
    let result = backend.apply(&HwProfile {
        charge_start_threshold: Some(80),
        ..HwProfile::default()
    });
    assert!(
        matches!(result, Err(Error::BadValue("charge start threshold", _))),
        "got {result:?}"
    );
    assert_eq!(read(&root, START), "70", "hardware was touched anyway");
}

#[test]
fn the_half_left_alone_still_counts_towards_the_check() {
    let root = writable_copy("half-counts");
    let backend = Msi::probe(&root).unwrap();

    // Start stays at 70 in the fixture, so an end of 65 would invert the pair.
    let result = backend.apply(&HwProfile {
        charge_end_threshold: Some(65),
        ..HwProfile::default()
    });
    assert!(matches!(result, Err(Error::BadValue(..))), "got {result:?}");
    assert_eq!(read(&root, END), "80");
}

#[test]
fn a_threshold_outside_the_accepted_range_never_reaches_the_kernel() {
    let root = writable_copy("out-of-range");
    let backend = Msi::probe(&root).unwrap();

    let result = backend.apply(&HwProfile {
        charge_start_threshold: Some(5),
        ..HwProfile::default()
    });
    assert!(matches!(result, Err(Error::BadValue(..))));
    assert_eq!(read(&root, START), "70");
}
