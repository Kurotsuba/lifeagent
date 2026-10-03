//! The command surface of `docs/PHASE1_SPEC.md` §10.2, independent of how it's transported.
//!
//! `ops` holds one async function per command, taking a `CoreApp` and returning
//! `Result<_, CommandError>`; `error` maps every core/plugin/LLM error to a `CommandError`
//! with a structured `detail` the UI translates (and a zh-CN `message`). The web server wraps
//! these in HTTP handlers; anything specific to HTTP (sessions, request bodies) stays there.

pub mod error;
pub mod events;
pub mod ops;

pub use error::{CommandError, ErrorDetail};
