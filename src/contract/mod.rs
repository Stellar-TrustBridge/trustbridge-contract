//! `TrustBridgeContract` entrypoints, grouped by concern (Issue #601).
//!
//! Each submodule holds one `#[contractimpl]` block on the same contract type,
//! so the exported ABI is identical to the former single block in `lib.rs`.

mod admin;
mod challenge;
mod config;
mod governance;
mod health;
mod maintenance;
mod registry;
mod roles;
mod rotation;
mod verify;
