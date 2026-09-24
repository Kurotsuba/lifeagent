//! `lifeagent-llm`: model abstraction, providers and role router.
//!
//! No dependency on `lifeagent-core` (see the workspace dependency rule in `CLAUDE.md`).
//! Implemented milestone by milestone per `docs/PHASE1_SPEC.md` §6: types, both providers,
//! the router and its `KeyProvider`/`UsageSink` traits, the `complete_json` helper and
//! `MockChatModel` landed in M2.

pub mod error;
pub mod json;
pub mod mock;
pub mod provider;
pub mod router;
pub mod types;
