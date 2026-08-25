//! One module per hardware vendor, plus `platform_profile` for machines the
//! kernel already covers on its own. See `msi` for a worked vendor example.

pub mod msi;
pub mod platform_profile;
