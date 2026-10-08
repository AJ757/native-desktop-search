//! Tauri IPC command handlers invoked from the React frontend.
//!
//! Handlers stay thin: they unpack arguments and delegate to plain functions
//! in the other modules, so the real logic can be tested without a window.