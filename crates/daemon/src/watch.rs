//! Listening to the rest of the system.
//!
//! We do not own the concept of a power profile — power-profiles-daemon does,
//! and every desktop already talks to it. So we follow it rather than compete
//! with it, and add the vendor layer it leaves as `placeholder`.

use std::sync::Arc;

use anyhow::Result;
use futures_util::StreamExt;

use crate::engine::Engine;

#[zbus::proxy(
    interface = "org.freedesktop.UPower.PowerProfiles",
    default_service = "org.freedesktop.UPower.PowerProfiles",
    default_path = "/org/freedesktop/UPower/PowerProfiles"
)]
pub trait PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
pub trait UPower {
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
}

/// Follow the active power profile for as long as the daemon runs.
pub async fn power_profiles(connection: zbus::Connection, engine: Arc<Engine>) -> Result<()> {
    let proxy = PowerProfilesProxy::new(&connection).await?;

    match proxy.active_profile().await {
        Ok(profile) => {
            tracing::info!(profile, "following power-profiles-daemon");
            engine.set_ppd_profile(Some(profile));
        }
        Err(e) => {
            // Not fatal: the hardware is still controllable by hand, and PPD
            // may well show up later.
            tracing::warn!(error = %e, "power-profiles-daemon not available");
        }
    }

    let mut changes = proxy.receive_active_profile_changed().await;
    while let Some(change) = changes.next().await {
        match change.get().await {
            Ok(profile) => {
                tracing::info!(profile, "power profile changed");
                engine.set_ppd_profile(Some(profile));
            }
            Err(e) => tracing::warn!(error = %e, "could not read the new power profile"),
        }
    }
    Ok(())
}

/// Follow whether the machine is running on battery.
pub async fn power_source(connection: zbus::Connection, engine: Arc<Engine>) -> Result<()> {
    let proxy = UPowerProxy::new(&connection).await?;

    match proxy.on_battery().await {
        Ok(on_battery) => engine.set_on_battery(on_battery),
        Err(e) => tracing::warn!(error = %e, "upower not available"),
    }

    let mut changes = proxy.receive_on_battery_changed().await;
    while let Some(change) = changes.next().await {
        match change.get().await {
            Ok(on_battery) => {
                tracing::info!(on_battery, "power source changed");
                engine.set_on_battery(on_battery);
            }
            Err(e) => tracing::warn!(error = %e, "could not read the power source"),
        }
    }
    Ok(())
}
