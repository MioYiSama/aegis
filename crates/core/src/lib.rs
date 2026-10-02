//! Shared local authentication algorithms. No network access.
pub mod qr;

#[cfg(feature = "native-face")]
pub mod face;
