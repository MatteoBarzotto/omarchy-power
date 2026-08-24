//! `omarchy-powerd` — the only thing in the project that writes to sysfs.
//!
//! Runs as root under systemd, owns `org.omarchy.Power1` on the system bus and
//! gates every write behind polkit. Clients stay unprivileged.

mod auth;
mod iface;

use anyhow::{Context, Result};
use omarchy_power_core::detect;
use tokio::signal::unix::{SignalKind, signal};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        // systemd captures stderr into the journal, which already timestamps.
        .without_time()
        .init();

    let session = std::env::args().any(|arg| arg == "--session");

    let backend = detect().context(
        "no supported hardware found\n\
         on MSI laptops this usually means the msi-ec module is missing: \
         install msi-ec-dkms and `modprobe msi_ec`",
    )?;
    tracing::info!(
        backend = backend.name(),
        model = backend.model().unwrap_or_default(),
        "hardware detected"
    );

    let connection = if session {
        // Development only: lets the interface be exercised without installing
        // a bus policy or running as root.
        tracing::warn!("serving on the session bus; writes will fail without root");
        zbus::Connection::session().await
    } else {
        zbus::Connection::system().await
    }
    .context("connecting to the message bus")?;

    let authority = auth::Authority::new(&connection)
        .await
        .context("connecting to polkit")?;

    let power = iface::Power::new(backend, authority);
    connection
        .object_server()
        .at(iface::PATH, power)
        .await
        .context("publishing the object")?;
    connection
        .request_name(iface::NAME)
        .await
        .with_context(|| format!("claiming {} (is another instance running?)", iface::NAME))?;

    tracing::info!(name = iface::NAME, path = iface::PATH, "ready");
    wait_for_shutdown().await?;
    tracing::info!("shutting down");
    Ok(())
}

/// Return on SIGTERM (systemd stopping us) or SIGINT (a terminal).
async fn wait_for_shutdown() -> Result<()> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
    Ok(())
}
