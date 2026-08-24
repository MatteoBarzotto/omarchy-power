//! Authorisation for the methods that change hardware.
//!
//! Reads are open to anyone on the bus; writes go through polkit. The policy
//! shipped in `packaging/` grants an active local session everything without a
//! prompt, so the TUI never asks for a password, while an ssh session has to
//! authenticate.

use std::collections::HashMap;

use zbus::message::Header;
use zbus_polkit::policykit1::{AuthorityProxy, CheckAuthorizationFlags, Subject};

/// Changing performance level, fan mode, cooler boost or battery saver.
pub const SET_PROFILE: &str = "org.omarchy.power1.set-profile";
/// Changing the charge threshold — separate because it outlives the session and
/// affects the battery over months, not minutes.
pub const SET_CHARGE_THRESHOLD: &str = "org.omarchy.power1.set-charge-threshold";

pub struct Authority {
    proxy: AuthorityProxy<'static>,
    /// When polkit is not on the bus at all, there is nothing to ask. That
    /// happens on minimal systems and inside containers; the daemon still runs,
    /// and the socket permissions from the bus policy remain the gate.
    available: bool,
}

impl Authority {
    pub async fn new(connection: &zbus::Connection) -> zbus::Result<Self> {
        let proxy = AuthorityProxy::new(connection).await?;
        // A cheap round trip decides once whether polkit is answering, instead
        // of discovering it on every method call.
        let available = proxy.backend_name().await.is_ok();
        if !available {
            tracing::warn!("polkit is not available; falling back to bus policy only");
        }
        Ok(Self { proxy, available })
    }

    /// Fail with `AccessDenied` unless the caller is allowed to perform `action`.
    pub async fn check(&self, header: &Header<'_>, action: &str) -> zbus::fdo::Result<()> {
        if !self.available {
            return Ok(());
        }

        let subject = Subject::new_for_message_header(header)
            .map_err(|e| zbus::fdo::Error::Failed(format!("identifying the caller: {e}")))?;

        let result = self
            .proxy
            .check_authorization(
                &subject,
                action,
                &HashMap::new(),
                CheckAuthorizationFlags::AllowUserInteraction.into(),
                "",
            )
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("asking polkit: {e}")))?;

        if result.is_authorized {
            Ok(())
        } else {
            Err(zbus::fdo::Error::AccessDenied(format!(
                "not authorized for {action}"
            )))
        }
    }
}
