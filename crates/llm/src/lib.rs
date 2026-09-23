//! `lifeagent-llm`: model abstraction, providers and role router.
//!
//! No dependency on `lifeagent-core` (see the workspace dependency rule in `CLAUDE.md`).
//! Implemented milestone by milestone per `docs/PHASE1_SPEC.md` §6; this module layout is
//! the M0 skeleton — types, providers, router, JSON helper and mock land in M2.

pub mod error;
pub mod json;
pub mod mock;
pub mod provider;
pub mod router;
pub mod types;
