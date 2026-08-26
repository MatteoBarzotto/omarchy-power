//! Hardware abstraction behind `omarchy-power`.
//!
//! The crate does no privileged work by itself and holds no global state: give
//! it a sysfs root and it hands back a [`Backend`] for whatever laptop that tree
//! describes. The daemon uses `/sys`; tests and contributed fixtures use a
//! captured directory.

pub mod backend;
pub mod backends;
pub mod detect;
pub mod gpu;
pub mod sysfs;
pub mod types;

#[cfg(feature = "dbus")]
pub mod wire;

pub use backend::{Backend, Error, Probe, Result};
pub use detect::{detect, detect_in};
pub use gpu::Gpu;
pub use types::{Battery, Capabilities, FanMode, HwProfile, HwState, PowerLevel, Sensors};
