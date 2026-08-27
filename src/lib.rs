//! Token-efficient Yandex Tracker client for humans and AI agents.
//!
//! The crate is split so that the CLI shell (`crate::cli`) stays thin and every
//! piece it needs is independently testable:
//!
//! * [`config`] — layered profile resolution (flag -> env -> `.tracker.toml` -> user config).
//! * [`secrets`] — OS keychain access; tokens never live in config files.
//! * [`api`] — the HTTP layer against the Tracker REST API.
//! * [`render`] — the output ladder (compact text / json / toon). This is the product:
//!   see `docs/adr/0003-output-ladder.md` before changing any default shape.
//! * [`exit`] — process exit codes shared by the shell and the integration tests.

pub mod api;
pub mod cli;
pub mod config;
pub mod exit;
pub mod render;
pub mod secrets;
