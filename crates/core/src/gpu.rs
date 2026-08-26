//! What the discrete GPU reports about itself.
//!
//! Deliberately **not** a [`crate::Backend`]. A backend is handed a sysfs root
//! and nothing else, which is what makes contributed hardware dumps testable;
//! NVIDIA exposes neither power draw nor clocks through sysfs, so honouring
//! that rule here would mean inventing a source that does not exist. Instead
//! this is a separate, optional reading that sits beside the backend and is
//! absent whenever the machine has no NVIDIA GPU — including on every AMD and
//! Intel laptop, where the keys simply never appear on the bus.
//!
//! Read-only on purpose. `nvidia-powerd` is actively moving watts between CPU
//! and GPU several times a second on a Dynamic Boost laptop, and a second
//! writer would produce bugs that reproduce only sometimes. Showing what it is
//! doing is useful; arguing with it is not.

use std::process::Command;

/// The `nvidia-smi` fields we ask for, in the order they come back.
///
/// `enforced.power.limit` rather than `power.limit`: on a Dynamic Boost laptop
/// the latter reads `[N/A]`, because there is no fixed limit to report — the
/// enforced one is the ceiling actually in effect.
const QUERY: &str = "power.draw,enforced.power.limit,clocks.gr";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gpu {
    /// Power draw right now, in whole watts.
    pub power_w: Option<u16>,
    /// The ceiling in effect, which Dynamic Boost raises and lowers.
    pub power_limit_w: Option<u16>,
    /// Graphics clock, in MHz.
    pub clock_mhz: Option<u32>,
}

impl Gpu {
    /// True when the GPU reported nothing at all, which is how a machine
    /// without one looks.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Ask the driver, or return `None` on any machine that cannot answer.
///
/// Every failure — no `nvidia-smi`, no GPU, a driver that will not talk — is
/// the same absence as far as callers are concerned, so none of them is an
/// error worth propagating. The call costs about 20 ms; the daemon caches the
/// answer rather than paying that on every client refresh.
pub fn read() -> Option<Gpu> {
    let output = Command::new("nvidia-smi")
        .args([
            &format!("--query-gpu={QUERY}"),
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let gpu = parse(&String::from_utf8_lossy(&output.stdout))?;
    (!gpu.is_empty()).then_some(gpu)
}

/// Parse one `nvidia-smi` row.
///
/// Split from the call so the shapes that matter — a missing field, the
/// `[N/A]` a Dynamic Boost laptop returns, a second GPU — are testable on a
/// machine with no NVIDIA hardware at all.
fn parse(csv: &str) -> Option<Gpu> {
    // One row per GPU; the first is the one the laptop renders on.
    let row = csv.lines().find(|line| !line.trim().is_empty())?;
    let mut fields = row.split(',').map(str::trim);
    Some(Gpu {
        // Watts arrive with decimals nobody reads on a one-line summary.
        power_w: field(fields.next())
            .and_then(|v| v.parse::<f32>().ok())
            .map(round_watts),
        power_limit_w: field(fields.next())
            .and_then(|v| v.parse::<f32>().ok())
            .map(round_watts),
        clock_mhz: field(fields.next()).and_then(|v| v.parse().ok()),
    })
}

/// `[N/A]` and friends are absences, not values.
fn field(raw: Option<&str>) -> Option<&str> {
    let value = raw?;
    (!value.is_empty() && !value.starts_with("[N/A")).then_some(value)
}

fn round_watts(value: f32) -> u16 {
    value.round().clamp(0.0, f32::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_row_parses() {
        let gpu = parse("13.12, 115.00, 1057\n").unwrap();
        assert_eq!(gpu.power_w, Some(13));
        assert_eq!(gpu.power_limit_w, Some(115));
        assert_eq!(gpu.clock_mhz, Some(1057));
    }

    #[test]
    fn a_dynamic_boost_laptop_reports_no_fixed_limit_rather_than_zero() {
        // This is exactly what `power.limit` returns on such a machine, and
        // showing it as 0 W would read as a GPU that cannot draw power.
        let gpu = parse("13.12, [N/A], 1057").unwrap();
        assert_eq!(gpu.power_w, Some(13));
        assert_eq!(gpu.power_limit_w, None);
    }

    #[test]
    fn the_first_gpu_wins_when_there_are_several() {
        let gpu = parse("40.0, 115.00, 2100\n5.0, 60.00, 300\n").unwrap();
        assert_eq!(gpu.power_w, Some(40));
    }

    #[test]
    fn nothing_at_all_is_absence_rather_than_a_row_of_zeroes() {
        assert_eq!(parse(""), None);
        assert!(parse("[N/A], [N/A], [N/A]").unwrap().is_empty());
    }

    #[test]
    fn a_truncated_row_keeps_what_it_could_read() {
        let gpu = parse("13.12").unwrap();
        assert_eq!(gpu.power_w, Some(13));
        assert_eq!(gpu.clock_mhz, None);
    }
}
