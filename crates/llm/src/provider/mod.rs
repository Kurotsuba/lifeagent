//! `ChatModel` provider implementations.
//!
//! Two providers land in M2 per `docs/PHASE1_SPEC.md` §6.2: an OpenAI-compatible Chat
//! Completions client (§6.2.1) and an Anthropic Messages client (§6.2.2), both behind the
//! same trait so the router (`router.rs`) never needs to know which one it is calling.

pub mod anthropic;
pub mod openai_compat;
