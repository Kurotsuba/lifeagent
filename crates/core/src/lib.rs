//! `lifeagent-core`: storage, plugin API, orchestrator, planner and scheduler.
//!
//! Depends on `lifeagent-llm` only — never on a plugin crate (see the dependency rule in
//! `CLAUDE.md`). Plugins are registered by the composition root (`cli`, `server`) via
//! `CoreAppBuilder::plugin`. Modules are filled in milestone by milestone per
//! `docs/PHASE1_SPEC.md` §7: foundations (`clock`, `time`, `db`, `error`, `settings`,
//! `secrets`, `profile`, `goals`, `events`, `memory`, `conversation`) landed in M1 (`memory` later became
//! `preferences`); the
//! plugin API, tool registry and orchestrator (`plugin`, `tools`, `core_tools`, `context`,
//! `app`) landed in M3, along with `usage` (needed as soon as an `LlmRouter` can be built —
//! see `usage.rs`'s doc comment). M4 added the plan types and `PlanStore`, the event
//! dispatcher and `today_view`; M5 added plan generation (`plan::PlanService`), the
//! `scheduler` with the daily backup, and `i18n`.

pub mod app;
pub mod approvals;
pub mod audit;
pub mod clock;
pub mod context;
pub mod conversation;
pub mod core_tools;
pub mod db;
pub mod error;
pub mod events;
pub mod follow_ups;
pub mod goals;
pub mod i18n;
pub mod plan;
pub mod plugin;
pub mod preferences;
pub mod profile;
pub mod scheduler;
pub mod secrets;
pub mod settings;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod time;
pub mod tools;
pub mod untrusted;
pub mod usage;
