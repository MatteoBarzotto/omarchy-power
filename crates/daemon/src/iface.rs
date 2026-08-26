//! The `org.omarchy.Power1` interface.
//!
//! Setters are deliberately one-per-attribute rather than a single "apply this
//! profile" call: it keeps the interface usable by hand with `busctl`, which is
//! how most bug reports about hardware get diagnosed.

use std::collections::HashMap;

use std::sync::Arc;

use omarchy_power_core::types::{FanMode, HwProfile, PowerLevel};
use omarchy_power_core::{Error, wire};
use zbus::message::Header;
use zbus::{fdo, interface};

use crate::auth::{self, Authority};
use crate::engine::Engine;
use crate::gpu::GpuReader;

pub const NAME: &str = "org.omarchy.Power1";
pub const PATH: &str = "/org/omarchy/Power1";

pub struct Power {
    engine: Arc<Engine>,
    authority: Authority,
    /// The discrete GPU's readings, cached — see [`GpuReader`].
    gpu: GpuReader,
    /// Units that also write the charge threshold, as found at startup.
    ///
    /// Read once rather than on every property read: enabling a unit is a
    /// deliberate act that happens between sessions, and a client asking a
    /// dozen times a second must not turn into a dozen calls into systemd.
    charge_conflicts: Vec<String>,
}

impl Power {
    pub fn new(engine: Arc<Engine>, authority: Authority, charge_conflicts: Vec<String>) -> Self {
        Self {
            engine,
            authority,
            gpu: GpuReader::new(),
            charge_conflicts,
        }
    }

    /// Apply a profile after checking that the caller is allowed to.
    async fn guarded_apply(
        &self,
        header: &Header<'_>,
        action: &str,
        profile: HwProfile,
    ) -> fdo::Result<()> {
        self.authority.check(header, action).await?;
        self.engine.apply_manual(&profile).map_err(to_fdo)
    }
}

#[interface(name = "org.omarchy.Power1")]
impl Power {
    /// Everything the hardware reports right now, as `a{sv}`.
    ///
    /// One call rather than a property per reading: clients refresh this once a
    /// second, and a dozen round trips per second for a dozen values is silly.
    fn snapshot(&self) -> fdo::Result<wire::Dict> {
        let state = self.engine.backend().read_state().map_err(to_fdo)?;
        let mut dict = wire::state_to_dict(&state);
        // The GPU is not backend data and cannot fail the call: a machine
        // without one simply adds no keys.
        wire::gpu_into_dict(&mut dict, &self.gpu.read());
        Ok(dict)
    }

    async fn set_power_level(
        &self,
        level: &str,
        #[zbus(header)] hdr: Header<'_>,
    ) -> fdo::Result<()> {
        let level: PowerLevel = level.parse().map_err(to_invalid_args)?;
        self.guarded_apply(
            &hdr,
            auth::SET_PROFILE,
            HwProfile {
                power_level: Some(level),
                ..HwProfile::default()
            },
        )
        .await
    }

    async fn set_fan_mode(&self, mode: &str, #[zbus(header)] hdr: Header<'_>) -> fdo::Result<()> {
        let mode: FanMode = mode.parse().map_err(to_invalid_args)?;
        self.guarded_apply(
            &hdr,
            auth::SET_PROFILE,
            HwProfile {
                fan_mode: Some(mode),
                ..HwProfile::default()
            },
        )
        .await
    }

    async fn set_cooler_boost(&self, on: bool, #[zbus(header)] hdr: Header<'_>) -> fdo::Result<()> {
        self.guarded_apply(
            &hdr,
            auth::SET_PROFILE,
            HwProfile {
                cooler_boost: Some(on),
                ..HwProfile::default()
            },
        )
        .await
    }

    async fn set_battery_saver(
        &self,
        on: bool,
        #[zbus(header)] hdr: Header<'_>,
    ) -> fdo::Result<()> {
        self.guarded_apply(
            &hdr,
            auth::SET_PROFILE,
            HwProfile {
                battery_saver: Some(on),
                ..HwProfile::default()
            },
        )
        .await
    }

    async fn set_charge_end_threshold(
        &self,
        percent: u8,
        #[zbus(header)] hdr: Header<'_>,
    ) -> fdo::Result<()> {
        self.guarded_apply(
            &hdr,
            auth::SET_CHARGE_THRESHOLD,
            HwProfile {
                charge_end_threshold: Some(percent),
                ..HwProfile::default()
            },
        )
        .await
    }

    /// Where charging resumes, which is the other half of a charge limit.
    ///
    /// Same polkit action as the end threshold: both outlive the session and
    /// affect the battery over months, and splitting them would mean two
    /// prompts for what a user thinks of as one setting.
    async fn set_charge_start_threshold(
        &self,
        percent: u8,
        #[zbus(header)] hdr: Header<'_>,
    ) -> fdo::Result<()> {
        self.guarded_apply(
            &hdr,
            auth::SET_CHARGE_THRESHOLD,
            HwProfile {
                charge_start_threshold: Some(percent),
                ..HwProfile::default()
            },
        )
        .await
    }

    /// Which backend claimed this machine, e.g. `msi-ec`.
    #[zbus(property)]
    fn backend_name(&self) -> String {
        self.engine.backend().name().to_owned()
    }

    /// Firmware version, where the hardware exposes one.
    #[zbus(property)]
    fn model(&self) -> String {
        self.engine.backend().model().unwrap_or_default()
    }

    /// Units that will overwrite whatever charge threshold we set.
    ///
    /// Empty on a machine where we have the attribute to ourselves. A client
    /// showing a charge limit should say who else is writing it, because the
    /// symptom — the limit is back at 100% after a reboot — looks like this
    /// tool losing the setting.
    #[zbus(property)]
    fn charge_threshold_conflicts(&self) -> Vec<String> {
        self.charge_conflicts.clone()
    }

    /// What this machine supports, so clients can grey out the rest.
    #[zbus(property)]
    fn capabilities(&self) -> HashMap<String, bool> {
        wire::caps_to_dict(&self.engine.backend().capabilities())
    }
}

/// Map hardware failures onto the closest standard D-Bus error.
fn to_fdo(error: Error) -> fdo::Error {
    match error {
        Error::Unsupported(_) => fdo::Error::NotSupported(error.to_string()),
        Error::BadValue(..) => fdo::Error::InvalidArgs(error.to_string()),
        // A write that fails with EACCES here means the daemon itself lacks
        // access, which is a deployment problem, not the caller's fault.
        _ => fdo::Error::Failed(error.to_string()),
    }
}

fn to_invalid_args(error: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::InvalidArgs(error.to_string())
}
