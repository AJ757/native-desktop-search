//! Application-wide error type and `Result` alias.
//!
//! Every fallible function returns `crate::error::Result<T>`. The error is
//! serialisable so Tauri commands can send readable messages to the UI.

pub type Result<T> = std::result::Result<T, String>;