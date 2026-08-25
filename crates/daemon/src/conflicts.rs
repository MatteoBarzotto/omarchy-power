//! Who else on this machine writes the charge threshold.
//!
//! A charge limit is the one setting here that silently disappears: another
//! unit rewrites the sysfs attribute at boot, the user's 80% is back at 100%,
//! and nothing anywhere says why. Rather than fight over the attribute, the
//! daemon asks systemd once who else is enabled and reports it, so the TUI can
//! say out loud that something else has the last word.

use anyhow::{Context, Result};

/// Units known to write a charge threshold of their own.
///
/// A list rather than a heuristic: guessing from unit names would flag every
/// service with "battery" in it, and being wrong here sends people hunting for
/// a conflict that does not exist.
const KNOWN_WRITERS: &[&str] = &[
    // Omarchy's own, which hard-codes a value and applies it at every boot.
    "battery-charge-threshold.service",
    // TLP applies START/STOP_CHARGE_THRESH_BAT from its own config.
    "tlp.service",
];

/// Unit file states that mean the unit actually runs. `static` and `disabled`
/// units are installed but never pulled in by anything, so they are not a
/// conflict — merely a package the user happens to have.
const ACTIVE_STATES: &[&str] = &["enabled", "enabled-runtime", "linked", "linked-runtime"];

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
pub trait Systemd {
    /// Returns `(unit file path, state)` for every unit matching a pattern.
    fn list_unit_files_by_patterns(
        &self,
        states: &[&str],
        patterns: &[&str],
    ) -> zbus::Result<Vec<(String, String)>>;
}

/// Ask systemd which of the known writers are enabled on this machine.
pub async fn charge_threshold_writers(connection: &zbus::Connection) -> Result<Vec<String>> {
    let proxy = SystemdProxy::new(connection)
        .await
        .context("connecting to systemd")?;
    let files = proxy
        .list_unit_files_by_patterns(&[], KNOWN_WRITERS)
        .await
        .context("listing unit files")?;
    Ok(enabled_writers(&files))
}

/// Reduce systemd's answer to the unit names that will run.
///
/// Split out from the call so the filtering is testable without a bus: the
/// interesting part is which states count, not the round trip.
fn enabled_writers(files: &[(String, String)]) -> Vec<String> {
    let mut names: Vec<String> = files
        .iter()
        .filter(|(_, state)| ACTIVE_STATES.contains(&state.as_str()))
        .filter_map(|(path, _)| unit_name(path))
        .collect();
    // A stable order keeps the warning line from reshuffling between reads.
    names.sort();
    names.dedup();
    names
}

/// systemd answers with full paths; the name is what a user types.
fn unit_name(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    KNOWN_WRITERS.contains(&name).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, state: &str) -> (String, String) {
        (path.to_owned(), state.to_owned())
    }

    #[test]
    fn an_enabled_writer_is_reported_by_name() {
        let files = [file(
            "/usr/lib/systemd/system/battery-charge-threshold.service",
            "enabled",
        )];
        assert_eq!(
            enabled_writers(&files),
            ["battery-charge-threshold.service"]
        );
    }

    #[test]
    fn an_installed_but_disabled_unit_is_not_a_conflict() {
        let files = [
            file("/usr/lib/systemd/system/tlp.service", "disabled"),
            file(
                "/usr/lib/systemd/system/battery-charge-threshold.service",
                "static",
            ),
        ];
        assert!(enabled_writers(&files).is_empty());
    }

    #[test]
    fn several_writers_come_back_in_a_stable_order() {
        let files = [
            file("/etc/systemd/system/tlp.service", "enabled"),
            file(
                "/usr/lib/systemd/system/battery-charge-threshold.service",
                "enabled-runtime",
            ),
        ];
        assert_eq!(
            enabled_writers(&files),
            ["battery-charge-threshold.service", "tlp.service"]
        );
    }

    #[test]
    fn a_unit_we_do_not_know_about_is_ignored_even_when_it_matches_a_pattern() {
        // systemd matches patterns its own way; the list stays authoritative.
        let files = [file("/etc/systemd/system/battery-life.service", "enabled")];
        assert!(enabled_writers(&files).is_empty());
    }
}
