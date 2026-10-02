//! The command surface of `docs/PHASE1_SPEC.md` §10.2, independent of how it's transported.
//!
//! `ops` holds one async function per command, taking a `CoreApp` and returning
//! `Result<_, CommandError>`; `error` maps every core/plugin/LLM error to a `CommandError`
//! with a zh-CN message. The desktop shell wraps these in Tauri commands and the web server in
//! HTTP handlers, so both expose the same behavior and the same errors. Anything specific to
//! one transport (Tauri's raw IPC bodies, OS autostart, HTTP sessions) stays in that shell.

pub mod error;
pub mod events;
pub mod ops;

pub use error::CommandError;
