//! Where the TUI gets its data and where its changes go.
//!
//! Two implementations: the daemon over D-Bus, and — when the daemon is not
//! running — a direct read-only view of sysfs. The fallback exists so that
//! `OMARCHY_POWER_SYSFS=fixtures/... omarchy-power` still shows something, which
//! is how contributed hardware dumps get eyeballed.

use anyhow::{Context, Result, bail};
use omarchy_power_core::gpu::Gpu;
use omarchy_power_core::types::{Capabilities, FanMode, HwState, PowerLevel};
use omarchy_power_core::{Backend, detect, wire};

#[zbus::proxy(
    interface = "org.omarchy.Power1",
    default_service = "org.omarchy.Power1",
    default_path = "/org/omarchy/Power1"
)]
pub(crate) trait Power {
    fn snapshot(&self) -> zbus::Result<wire::Dict>;
    fn set_power_level(&self, level: &str) -> zbus::Result<()>;
    fn set_fan_mode(&self, mode: &str) -> zbus::Result<()>;
    fn set_cooler_boost(&self, on: bool) -> zbus::Result<()>;
    fn set_battery_saver(&self, on: bool) -> zbus::Result<()>;
    fn set_charge_end_threshold(&self, percent: u8) -> zbus::Result<()>;

    #[zbus(property)]
    fn backend_name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn model(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn capabilities(&self) -> zbus::Result<std::collections::HashMap<String, bool>>;
    #[zbus(property)]
    fn charge_threshold_conflicts(&self) -> zbus::Result<Vec<String>>;
}

pub enum Source {
    /// The daemon is running: reads and writes both go over the bus.
    Daemon {
        proxy: PowerProxyBlocking<'static>,
        backend: String,
        model: Option<String>,
        capabilities: Capabilities,
        /// Units that overwrite the charge threshold behind our back.
        charge_conflicts: Vec<String>,
    },
    /// No daemon: read straight from sysfs and refuse to write.
    Local {
        backend: Box<dyn Backend>,
        capabilities: Capabilities,
    },
}

impl Source {
    /// Prefer the daemon; fall back to reading sysfs directly.
    pub fn connect() -> Result<Self> {
        match Self::daemon() {
            Ok(source) => Ok(source),
            Err(e) => {
                // Not an error worth failing on — the read-only view is useful
                // on its own — but worth saying out loud, because "my keys do
                // nothing" otherwise looks like a bug in the TUI.
                eprintln!("omarchy-powerd unavailable ({e}); starting read-only");
                Self::local()
            }
        }
    }

    fn daemon() -> Result<Self> {
        let connection = zbus::blocking::Connection::system()?;
        let proxy = PowerProxyBlocking::new(&connection)?;
        // The first property read doubles as the liveness check: if the daemon
        // is not on the bus, this is what fails.
        let backend = proxy.backend_name()?;
        let model = proxy.model().ok().filter(|m| !m.is_empty());
        let capabilities = wire::caps_from_dict(&proxy.capabilities()?);
        // An older daemon does not have the property; that is not a reason to
        // refuse to start, only a reason not to warn about anything.
        let charge_conflicts = proxy.charge_threshold_conflicts().unwrap_or_default();
        Ok(Self::Daemon {
            proxy,
            backend,
            model,
            capabilities,
            charge_conflicts,
        })
    }

    fn local() -> Result<Self> {
        let backend = detect().context(
            "no supported hardware found\n\
             on MSI laptops this usually means the msi-ec module is missing: \
             install msi-ec-dkms and `modprobe msi_ec`",
        )?;
        let capabilities = backend.capabilities();
        Ok(Self::Local {
            backend,
            capabilities,
        })
    }

    pub fn is_read_only(&self) -> bool {
        matches!(self, Self::Local { .. })
    }

    pub fn backend_name(&self) -> String {
        match self {
            Self::Daemon { backend, .. } => backend.clone(),
            Self::Local { backend, .. } => backend.name().to_owned(),
        }
    }

    pub fn model(&self) -> Option<String> {
        match self {
            Self::Daemon { model, .. } => model.clone(),
            Self::Local { backend, .. } => backend.model(),
        }
    }

    pub fn capabilities(&self) -> Capabilities {
        match self {
            Self::Daemon { capabilities, .. } | Self::Local { capabilities, .. } => *capabilities,
        }
    }

    /// Who else writes the charge threshold. Empty without a daemon: the
    /// read-only view has no bus to ask systemd over.
    pub fn charge_conflicts(&self) -> &[String] {
        match self {
            Self::Daemon {
                charge_conflicts, ..
            } => charge_conflicts,
            Self::Local { .. } => &[],
        }
    }

    /// One call returns both, because they arrive in one dictionary and a
    /// second round trip for the GPU would double the traffic for nothing.
    pub fn snapshot(&self) -> Result<(HwState, Gpu)> {
        match self {
            Self::Daemon { proxy, .. } => {
                let dict = proxy.snapshot()?;
                Ok((wire::state_from_dict(&dict), wire::gpu_from_dict(&dict)))
            }
            // The read-only fallback has no daemon to ask, and reading the GPU
            // here would mean the TUI running a process of its own.
            Self::Local { backend, .. } => Ok((backend.read_state()?, Gpu::default())),
        }
    }

    pub fn set_power_level(&self, level: PowerLevel) -> Result<()> {
        self.proxy()?.set_power_level(level.as_str())?;
        Ok(())
    }

    pub fn set_fan_mode(&self, mode: FanMode) -> Result<()> {
        self.proxy()?.set_fan_mode(mode.as_str())?;
        Ok(())
    }

    pub fn set_cooler_boost(&self, on: bool) -> Result<()> {
        self.proxy()?.set_cooler_boost(on)?;
        Ok(())
    }

    pub fn set_battery_saver(&self, on: bool) -> Result<()> {
        self.proxy()?.set_battery_saver(on)?;
        Ok(())
    }

    pub fn set_charge_end_threshold(&self, percent: u8) -> Result<()> {
        self.proxy()?.set_charge_end_threshold(percent)?;
        Ok(())
    }

    /// Writes only exist through the daemon; the TUI never touches sysfs itself.
    fn proxy(&self) -> Result<&PowerProxyBlocking<'static>> {
        match self {
            Self::Daemon { proxy, .. } => Ok(proxy),
            Self::Local { .. } => bail!(
                "omarchy-powerd is not running: start it with `systemctl start omarchy-powerd`"
            ),
        }
    }
}
