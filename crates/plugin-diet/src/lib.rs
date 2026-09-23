//! `lifeagent-plugin-diet`: meal logging, nutrition targets and the diet plan section.
//!
//! Depends on `lifeagent-core` and `lifeagent-llm`; implements the `Plugin` trait from
//! `lifeagent-core`. Registered by the composition root (`cli`, `desktop`) — `core` never
//! depends on this crate. This module layout is the M0 skeleton, filled in during M4/M5 per
//! `docs/PHASE1_SPEC.md` §8.

pub mod foods;
pub mod intake;
pub mod model;
pub mod parser;
pub mod plan;
pub mod prompts;
pub mod resolver;
pub mod targets;
pub mod tools;
pub mod validate;
