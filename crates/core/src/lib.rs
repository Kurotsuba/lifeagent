//! `lifeagent-core`: storage, plugin API, orchestrator, planner and scheduler.
//!
//! Depends on `lifeagent-llm` only — never on a plugin crate (see the dependency rule in
//! `CLAUDE.md`). Plugins are registered by the composition root (`cli`, `desktop`) via
//! `CoreAppBuilder::plugin`. Modules are filled in milestone by milestone per
//! `docs/PHASE1_SPEC.md` §7: foundations (`clock`, `time`, `db`, `error`, `settings`,
//! `secrets`, `profile`, `goals`, `events`, `memory`, `conversation`) landed in M1; the
//! plugin API, tool registry and orchestrator (`plugin`, `tools`, `core_tools`, `context`,
//! `app`) are M3; the plan service and scheduler (`plan`, `scheduler`, `usage`) are M5.

pub mod app;
pub mod clock;
pub mod context;
pub mod conversation;
pub mod core_tools;
pub mod db;
pub mod error;
pub mod events;
pub mod goals;
pub mod memory;
pub mod plan;
pub mod plugin;
pub mod profile;
pub mod scheduler;
pub mod secrets;
pub mod settings;
pub mod time;
pub mod tools;
pub mod usage;
