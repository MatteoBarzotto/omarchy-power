//! The `org.omarchy.Power1` interface.
//!
//! Setters are deliberately one-per-attribute rather than a single "apply this
//! profile" call: it keeps the interface usable by hand with `busctl`, which is
//! how most bug reports about hardware get diagnosed.

use std::collections::HashMap;

use omarchy_power_core::types::{FanMode, HwProfile, PowerLevel};
use omarchy_power_core::{Backend, Error, wire};
use zbus::message::Header;
use zbus::{fdo, interface};

use crate::auth::{self, Authority};

pub const NAME: &str = "org.omarchy.Power1";
pub const PATH: &str = "/org/omarchy/Power1";

pub struct Power {
    backend: Box<dyn Backend>,
    authority: Authority,
}

impl Power {
    pub fn new(backend: Box<dyn Backend>, authority: Authority) -> Self {
        Self { backend, authority }
    }

    /// Apply a profile after checking that the caller is allowed to.
    async fn guarded_apply(
        &self,
        header: &Header<'_>,
        action: &str,
        profile: HwProfile,
    ) -> fdo::Result<()> {
        self.authority.check(header, action).await?;
        tracing::info!(?profile, "applying");
        self.backend.apply(&profile).map_err(to_fdo)
    }
}

#[interface(name = "org.omarchy.Power1")]
impl Power {
    /// Everything the hardware reports right now, as `a{sv}`.
    ///
    /// One call rather than a property per reading: clients refresh this once a
    /// second, and a dozen round trips per second for a dozen values is silly.
    fn snapshot(&self) -> fdo::Result<wire::Dict> {
        let state = self.backend.read_state().map_err(to_fdo)?;
        Ok(wire::state_to_dict(&state))
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

    /// Which backend claimed this machine, e.g. `msi-ec`.
    #[zbus(property)]
    fn backend_name(&self) -> String {
        self.backend.name().to_owned()
    }

    /// Firmware version, where the hardware exposes one.
    #[zbus(property)]
    fn model(&self) -> String {
        self.backend.model().unwrap_or_default()
    }

    /// What this machine supports, so clients can grey out the rest.
    #[zbus(property)]
    fn capabilities(&self) -> HashMap<String, bool> {
        wire::caps_to_dict(&self.backend.capabilities())
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
