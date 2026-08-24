//! The vendor boundary.
//!
//! A backend is handed a sysfs root and never builds an absolute path of its
//! own. That single rule is what makes hardware support contributable: a
//! captured directory tree from someone else's laptop is indistinguishable from
//! real `/sys` as far as a backend is concerned, so their machine can be tested
//! in CI by people who do not own it.

use std::path::Path;

use crate::types::{Capabilities, HwProfile, HwState};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no supported hardware backend found for this machine")]
    NoBackend,
    #[error("this machine does not support {0}")]
    Unsupported(&'static str),
    #[error("{0} does not accept the value {1}")]
    BadValue(&'static str, String),
    #[error("reading {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("writing {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Implemented once per hardware vendor.
///
/// Object-safe on purpose: detection returns `Box<dyn Backend>` so the rest of
/// the program never names a concrete vendor type.
pub trait Backend: Send + Sync {
    /// Stable identifier, used in logs, on the D-Bus interface and in bug reports.
    fn name(&self) -> &'static str;

    /// Human-readable model, when the firmware exposes one.
    fn model(&self) -> Option<String> {
        None
    }

    fn capabilities(&self) -> Capabilities;

    fn read_state(&self) -> Result<HwState>;

    /// Apply every field the profile sets, leaving the rest untouched.
    ///
    /// Implementations must validate against [`Backend::capabilities`] first and
    /// return [`Error::Unsupported`] rather than writing hopefully.
    fn apply(&self, profile: &HwProfile) -> Result<()>;
}

/// A backend that can recognise its own hardware.
///
/// Kept separate from [`Backend`] because `probe` is not object-safe; the
/// registry in [`crate::detect`] bridges the two.
pub trait Probe: Backend + Sized + 'static {
    /// Return a backend if this sysfs tree belongs to hardware we handle.
    fn probe(sysfs_root: &Path) -> Option<Self>;
}
