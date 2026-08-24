//! `omarchy-powerd` — the only thing in the project that writes to sysfs.
//!
//! Runs as root under systemd, owns `org.omarchy.Power1` on the system bus and
//! gates every write behind polkit. Clients stay unprivileged.

mod auth;
mod config;
mod engine;
mod iface;
mod watch;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use config::Config;
use engine::Engine;
use omarchy_power_core::detect;
use tokio::signal::unix::{SignalKind, signal};
use tracing_subscriber::EnvFilter;

/// How often the thermal guard looks at the temperature.
///
/// Fast enough to catch a machine heating up, slow enough that reading the EC
/// costs nothing worth measuring.
const THERMAL_INTERVAL: Duration = Duration::from_secs(5);

/// Points the daemon at a different config file, mostly for testing.
const CONFIG_ENV: &str = "OMARCHY_POWER_CONFIG";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        // systemd captures stderr into the journal, which already timestamps.
        .without_time()
        // Under systemd stderr is a socket, not a terminal, and colour escapes
        // would end up as literal noise in the journal.
        .with_ansi(std::io::stderr().is_terminal())
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

    let config_path = std::env::var_os(CONFIG_ENV)
        .map_or_else(|| PathBuf::from(config::DEFAULT_PATH), PathBuf::from);
    let config = Config::load(&config_path)?;

    let engine = Arc::new(Engine::new(backend, config));

    let authority = auth::Authority::new(&connection)
        .await
        .context("connecting to polkit")?;

    let power = iface::Power::new(Arc::clone(&engine), authority);
    connection
        .object_server()
        .at(iface::PATH, power)
        .await
        .context("publishing the object")?;
    connection
        .request_name(iface::NAME)
        .await
        .with_context(|| format!("claiming {} (is another instance running?)", iface::NAME))?;

    // power-profiles-daemon and upower always live on the system bus, whatever
    // bus we happen to be serving on. In `--session` mode that is a different
    // connection; otherwise it is the one we already have.
    let observed = if session {
        zbus::Connection::system()
            .await
            .context("connecting to the system bus to watch other daemons")?
    } else {
        connection.clone()
    };

    // Each watcher owns its subscription for the lifetime of the daemon; a
    // failing one must not take the others — or the interface — down with it.
    spawn_watcher(
        "power-profiles",
        watch::power_profiles(observed.clone(), Arc::clone(&engine)),
    );
    spawn_watcher(
        "power-source",
        watch::power_source(observed, Arc::clone(&engine)),
    );
    tokio::spawn(thermal_loop(Arc::clone(&engine)));

    tracing::info!(name = iface::NAME, path = iface::PATH, "ready");
    wait_for_shutdown().await?;
    tracing::info!("shutting down");
    Ok(())
}

fn spawn_watcher(name: &'static str, task: impl Future<Output = Result<()>> + Send + 'static) {
    tokio::spawn(async move {
        match task.await {
            Ok(()) => tracing::warn!(name, "watcher stopped"),
            Err(e) => tracing::error!(name, error = %e, "watcher failed"),
        }
    });
}

/// Poll the temperature so the thermal guard has something to act on.
async fn thermal_loop(engine: Arc<Engine>) {
    let mut ticker = tokio::time::interval(THERMAL_INTERVAL);
    loop {
        ticker.tick().await;
        engine.tick_thermal();
    }
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
