//! Application setup and shared state.
//!
//! Builds the `AppState` (database handle, data paths, logging guard) that
//! Tauri manages, and runs startup steps such as creating directories,
//! initialising logging and migrating the database.