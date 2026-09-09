//! The daemon's listener and connection loop, as a library so its own
//! tests can start one in process (spec 018).
//!
//! The binary is `yeomnad` and lives in `main.rs`. Everything it does
//! beyond reading a config file is here.

pub mod server;
