//! Configuration and filesystem locations.
//!
//! Resolves the app data directory and its subfolders (database, search
//! index, logs, model cache) and exposes them through a single `AppPaths`
//! struct so no other module builds paths by hand.