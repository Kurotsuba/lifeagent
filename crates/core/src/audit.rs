//! The agent activity log (`docs/AUDIT_LOG_SPEC.md`): one run per agent entry point (a chat
//! turn, a plan, an approval decision) and its steps in order — model calls, tool calls,
//! approvals, guardrails, plan validation.
//!
//! Steps attach to the current run through a task-local ([`scope`], [`record`]), so code deep
//! inside the planner or the usage sink records without a run handle being passed through every
//! signature. Outside a run (event hooks, tests that don't open one) [`record`] does nothing.
//!
//! Starting a run fails closed (no record, no action); writing a step or finishing a run fails
//! open with an ERROR line, like `DbUsageSink::record` (§3.4). Every step also writes one
//! `agent step` INFO line with ids and kinds only, never arguments or text (§7).

use std::future::Future;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::CoreError;
use crate::events::{AppEvent, EventStore};

/// Runs (and their steps) older than this are deleted (§4.5).
pub const RETENTION_DAYS: i64 = 90;
/// A run still `running` after this long was cut off (the instance stopped mid-turn) and reads
/// as `interrupted`.
pub const INTERRUPTED_AFTER_MINUTES: i64 = 15;
/// The longest stored input excerpt, in characters.
pub const EXCERPT_CHARS: usize = 200;
/// Tool arguments longer than this (as JSON) are stored cut, marked `"truncated": true`.
pub const MAX_ARGUMENTS_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    User,
    Scheduler,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    ChatTurn,
    PlanScheduled,
    PlanManual,
    PlanRevision,
    ActionApproved,
    ActionRejected,
    /// The scheduler raised proactive follow-ups (`crate::follow_ups`).
    FollowUp,
    /// The user answered a follow-up.
    FollowUpAnswered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    /// Ended normally, with at least one tool call left waiting for the user's approval.
    NeedsApproval,
    Failed,
    /// Still `running` after [`INTERRUPTED_AFTER_MINUTES`]. Not stored: derived when read.
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    ModelCall,
    ToolCall,
    ApprovalRequested,
    ApprovalResolved,
    Guardrail,
    PlanValidation,
    PlanSaved,
    /// A follow-up was raised: `{follow_up_id, key, kind, reason, params}`.
    FollowUpRaised,
    /// A follow-up was answered: `{follow_up_id, key, kind, answer, result?}`.
    FollowUpAnswered,
}

/// Generates `as_str` and `parse` from the serde names, so the database strings can't drift
/// from what the API sends.
macro_rules! db_enum {
    ($ty:ident { $($variant:ident => $name:literal),* $(,)? }) => {
        impl $ty {
            pub fn as_str(self) -> &'static str {
                match self {
                    $($ty::$variant => $name),*
                }
            }

            fn parse(s: &str) -> Result<Self, CoreError> {
                match s {
                    $($name => Ok($ty::$variant),)*
                    other => Err(CoreError::Validation(format!(
                        concat!("bad ", stringify!($ty), " in db: {:?}"),
                        other
                    ))),
                }
            }
        }
    };
}

db_enum!(Actor { User => "user", Scheduler => "scheduler" });
db_enum!(RunTrigger {
    ChatTurn => "chat_turn",
    PlanScheduled => "plan_scheduled",
    PlanManual => "plan_manual",
    PlanRevision => "plan_revision",
    ActionApproved => "action_approved",
    ActionRejected => "action_rejected",
    FollowUp => "follow_up",
    FollowUpAnswered => "follow_up_answered",
});
db_enum!(RunStatus {
    Running => "running",
    Succeeded => "succeeded",
    NeedsApproval => "needs_approval",
    Failed => "failed",
    Interrupted => "interrupted",
});
db_enum!(StepKind {
    ModelCall => "model_call",
    ToolCall => "tool_call",
    ApprovalRequested => "approval_requested",
    ApprovalResolved => "approval_resolved",
    Guardrail => "guardrail",
    PlanValidation => "plan_validation",
    PlanSaved => "plan_saved",
    FollowUpRaised => "follow_up_raised",
    FollowUpAnswered => "follow_up_answered",
});

/// The plan a run saved, for the list view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedPlan {
    pub date: NaiveDate,
    pub version: u32,
}

/// Counts for the list view (§4.4), kept while the run goes and stored when it ends.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub model_calls: u32,
    pub tool_calls: u32,
    pub tokens: u64,
    pub latency_ms: u64,
    pub guardrails: u32,
    pub pending_actions: u32,
    pub plan: Option<SavedPlan>,
}

impl RunOutcome {
    fn count(&mut self, kind: StepKind, detail: &Value) {
        let u64_of = |key: &str| detail.get(key).and_then(Value::as_u64).unwrap_or(0);
        match kind {
            StepKind::ModelCall => {
                self.model_calls += 1;
                self.tokens += u64_of("prompt_tokens") + u64_of("completion_tokens");
                self.latency_ms += u64_of("latency_ms");
            }
            StepKind::ToolCall => self.tool_calls += 1,
            StepKind::Guardrail => self.guardrails += 1,
            StepKind::ApprovalRequested => self.pending_actions += 1,
            StepKind::PlanSaved => {
                if let Ok(plan) = serde_json::from_value(detail.clone()) {
                    self.plan = Some(plan);
                }
            }
            StepKind::ApprovalResolved
            | StepKind::PlanValidation
            | StepKind::FollowUpRaised
            | StepKind::FollowUpAnswered => {}
        }
    }
}

/// A run as the API returns it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Run {
    pub id: String,
    pub trigger: RunTrigger,
    pub actor: Actor,
    pub status: RunStatus,
    pub input_excerpt: Option<String>,
    pub outcome: Option<RunOutcome>,
    pub error_code: Option<String>,
    pub logical_date: NaiveDate,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub id: String,
    pub kind: StepKind,
    pub detail: Value,
    pub created_at: DateTime<Utc>,
}

/// One page of runs, newest first. `next_before` continues the listing, `None` at the end.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunPage {
    pub runs: Vec<Run>,
    pub next_before: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunFilter {
    /// Only runs started before this run id (UUID v7 ids sort by start time).
    pub before: Option<String>,
    pub actor: Option<Actor>,
    pub status: Option<RunStatus>,
}

/// Counts over a period, for the Activity page's header strip.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ActivityCounts {
    pub runs_by_user: u64,
    pub runs_by_scheduler: u64,
    pub approvals_requested: u64,
    pub approvals_approved: u64,
    pub approvals_rejected: u64,
    pub guardrails: u64,
}

/// How a run's body ended, for [`RunHandle::finish`].
pub enum RunEnd<'a> {
    Ok,
    Failed { code: &'a str },
}

#[derive(Clone)]
pub struct AuditLog {
    db: PgPool,
    clock: Arc<dyn Clock>,
    events: EventStore,
}

/// The run in progress. Cheap to clone; every clone records into the same run.
#[derive(Clone)]
pub struct RunHandle(Arc<RunInner>);

struct RunInner {
    id: String,
    trigger: RunTrigger,
    actor: Actor,
    log: AuditLog,
    outcome: Mutex<RunOutcome>,
}

tokio::task_local! {
    static CURRENT_RUN: RunHandle;
}

/// Runs `fut` with `run` as the current run, so [`record`] calls inside it reach `run`.
pub async fn scope<F: Future>(run: RunHandle, fut: F) -> F::Output {
    CURRENT_RUN.scope(run, fut).await
}

/// The current run, if the caller is inside [`scope`].
pub fn current() -> Option<RunHandle> {
    CURRENT_RUN.try_with(Clone::clone).ok()
}

/// Appends a step to the current run; does nothing outside a run.
pub async fn record(kind: StepKind, detail: Value) {
    if let Some(run) = current() {
        run.step(kind, detail).await;
    }
}

/// `detail_json` for a `tool_call` step. `outcome`: `ran`, `invalid_args`, `not_found` or
/// `failed`. Arguments larger than [`MAX_ARGUMENTS_BYTES`] are stored cut.
pub fn tool_call_detail(
    tool: &str,
    arguments: &Value,
    outcome: &str,
    summary: Option<&crate::plugin::ToolSummary>,
    error: Option<&str>,
) -> Value {
    let mut detail = json!({
        "tool": tool,
        "arguments": bounded_arguments(arguments),
        "outcome": outcome,
    });
    if let Some(summary) = summary {
        detail["summary"] = summary.0.clone();
    }
    if let Some(error) = error {
        detail["error"] = json!(error);
    }
    detail
}

fn bounded_arguments(arguments: &Value) -> Value {
    let text = arguments.to_string();
    if text.len() <= MAX_ARGUMENTS_BYTES {
        return arguments.clone();
    }
    let mut end = MAX_ARGUMENTS_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    json!({ "truncated": true, "text": &text[..end] })
}

fn excerpt(input: &str) -> String {
    input.trim().chars().take(EXCERPT_CHARS).collect()
}

impl RunHandle {
    pub fn id(&self) -> &str {
        &self.0.id
    }

    pub fn pending_actions(&self) -> u32 {
        self.outcome().pending_actions
    }

    fn outcome(&self) -> RunOutcome {
        match self.0.outcome.lock() {
            Ok(outcome) => outcome.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Appends a step. Fails open: a write error is logged, never returned.
    pub async fn step(&self, kind: StepKind, detail: Value) {
        {
            let mut outcome = match self.0.outcome.lock() {
                Ok(outcome) => outcome,
                Err(poisoned) => poisoned.into_inner(),
            };
            outcome.count(kind, &detail);
        }
        self.log_step(kind, &detail);
        let result = sqlx::query(
            "INSERT INTO agent_steps (id, run_id, kind, detail_json, created_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(&self.0.id)
        .bind(kind.as_str())
        .bind(detail.to_string())
        .bind(self.0.log.clock.now().to_rfc3339())
        .execute(&self.0.log.db)
        .await;
        if let Err(e) = result {
            tracing::error!(run_id = %self.0.id, kind = kind.as_str(), error = %e,
                "failed to record agent step");
        }
    }

    /// The Cloud Logging copy (§7): ids and kinds only. Arguments, excerpts and summaries hold
    /// personal data and stay out of INFO.
    fn log_step(&self, kind: StepKind, detail: &Value) {
        let str_of = |key: &str| detail.get(key).and_then(Value::as_str).unwrap_or_default();
        let (run_id, trigger, actor) = (
            self.0.id.as_str(),
            self.0.trigger.as_str(),
            self.0.actor.as_str(),
        );
        match kind {
            StepKind::ToolCall => tracing::info!(
                target: "lifeagent_core::audit",
                run_id, trigger, actor, kind = kind.as_str(),
                tool = str_of("tool"), outcome = str_of("outcome"),
                "agent step"
            ),
            StepKind::Guardrail => tracing::info!(
                target: "lifeagent_core::audit",
                run_id, trigger, actor, kind = kind.as_str(), rule = str_of("rule"),
                "agent step"
            ),
            StepKind::ApprovalRequested | StepKind::ApprovalResolved => tracing::info!(
                target: "lifeagent_core::audit",
                run_id, trigger, actor, kind = kind.as_str(), tool = str_of("tool"),
                "agent step"
            ),
            // Kinds and answer kinds only; params hold the user's meals and numbers.
            StepKind::FollowUpRaised | StepKind::FollowUpAnswered => tracing::info!(
                target: "lifeagent_core::audit",
                run_id, trigger, actor, kind = kind.as_str(),
                follow_up = str_of("kind"), answer = str_of("answer"),
                "agent step"
            ),
            _ => tracing::info!(
                target: "lifeagent_core::audit",
                run_id, trigger, actor, kind = kind.as_str(),
                "agent step"
            ),
        }
    }

    /// Ends the run: `needs_approval` if it queued actions, else `succeeded`, or `failed` with
    /// the error code. Fails open, then publishes `AppEvent::ActivityRecorded`.
    pub async fn finish(&self, end: RunEnd<'_>) {
        let outcome = self.outcome();
        let (status, error_code) = match end {
            RunEnd::Ok if outcome.pending_actions > 0 => (RunStatus::NeedsApproval, None),
            RunEnd::Ok => (RunStatus::Succeeded, None),
            RunEnd::Failed { code } => (RunStatus::Failed, Some(code)),
        };
        let outcome_json = serde_json::to_string(&outcome).unwrap_or_default();
        let result = sqlx::query(
            "UPDATE agent_runs SET status = $1, outcome_json = $2, error_code = $3, finished_at = $4
             WHERE id = $5",
        )
        .bind(status.as_str())
        .bind(outcome_json)
        .bind(error_code)
        .bind(self.0.log.clock.now().to_rfc3339())
        .bind(&self.0.id)
        .execute(&self.0.log.db)
        .await;
        match result {
            Ok(_) => tracing::info!(
                target: "lifeagent_core::audit",
                run_id = %self.0.id, trigger = self.0.trigger.as_str(),
                actor = self.0.actor.as_str(), status = status.as_str(),
                model_calls = outcome.model_calls, tool_calls = outcome.tool_calls,
                tokens = outcome.tokens, guardrails = outcome.guardrails,
                "agent run finished"
            ),
            Err(e) => tracing::error!(run_id = %self.0.id, error = %e,
                "failed to record the end of an agent run"),
        }
        self.0.log.events.publish(AppEvent::ActivityRecorded {
            run_id: self.0.id.clone(),
        });
    }
}

type RunRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<String>,
);

/// The status as read (also used to filter): `running` past the cutoff (`$1`) reads as
/// `interrupted`, and `needs_approval` reads as `succeeded` once none of the run's actions still
/// waits for the user (approved, rejected, failed, or expired before `$2`).
const STATUS_AS_READ: &str = "CASE
    WHEN status = 'running' AND started_at < $1 THEN 'interrupted'
    WHEN status = 'needs_approval' AND NOT EXISTS (
        SELECT 1 FROM pending_actions p
        WHERE p.run_id = agent_runs.id AND p.status = 'pending' AND p.created_at >= $2
    ) THEN 'succeeded'
    ELSE status END";

impl AuditLog {
    pub fn new(db: PgPool, clock: Arc<dyn Clock>, events: EventStore) -> Self {
        Self { db, clock, events }
    }

    /// Opens a run. Fails closed: the caller must not act if this errors.
    pub async fn start(
        &self,
        trigger: RunTrigger,
        actor: Actor,
        input: Option<&str>,
        logical_date: NaiveDate,
    ) -> Result<RunHandle, CoreError> {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO agent_runs (id, trigger, actor, status, input_excerpt, logical_date, started_at)
             VALUES ($1, $2, $3, 'running', $4, $5, $6)",
        )
        .bind(&id)
        .bind(trigger.as_str())
        .bind(actor.as_str())
        .bind(input.map(excerpt).filter(|s| !s.is_empty()))
        .bind(logical_date.to_string())
        .bind(self.clock.now().to_rfc3339())
        .execute(&self.db)
        .await?;
        tracing::info!(target: "lifeagent_core::audit", run_id = %id,
            trigger = trigger.as_str(), actor = actor.as_str(), "agent run started");
        Ok(RunHandle(Arc::new(RunInner {
            id,
            trigger,
            actor,
            log: self.clone(),
            outcome: Mutex::new(RunOutcome::default()),
        })))
    }

    fn interrupted_cutoff(&self) -> String {
        (self.clock.now() - Duration::minutes(INTERRUPTED_AFTER_MINUTES)).to_rfc3339()
    }

    /// Pending actions created before this have expired (`crate::approvals`).
    fn expiry_cutoff(&self) -> String {
        (self.clock.now() - Duration::hours(crate::approvals::EXPIRY_HOURS)).to_rfc3339()
    }

    /// Runs newest first, `limit` (1..=100) per page.
    pub async fn list(&self, filter: &RunFilter, limit: u32) -> Result<RunPage, CoreError> {
        let limit = limit.clamp(1, 100);
        let rows: Vec<RunRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT id, trigger, actor, {STATUS_AS_READ}, input_excerpt, outcome_json, error_code,
                    logical_date, started_at, finished_at
             FROM agent_runs
             WHERE ($3::text IS NULL OR id < $3)
               AND ($4::text IS NULL OR actor = $4)
               AND ($5::text IS NULL OR {STATUS_AS_READ} = $5)
             ORDER BY id DESC
             LIMIT $6"
        )))
        .bind(self.interrupted_cutoff())
        .bind(self.expiry_cutoff())
        .bind(filter.before.as_deref())
        .bind(filter.actor.map(Actor::as_str))
        .bind(filter.status.map(RunStatus::as_str))
        .bind(i64::from(limit) + 1)
        .fetch_all(&self.db)
        .await?;
        let mut runs = rows
            .into_iter()
            .map(run_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let next_before = if runs.len() > limit as usize {
            runs.truncate(limit as usize);
            runs.last().map(|r| r.id.clone())
        } else {
            None
        };
        Ok(RunPage { runs, next_before })
    }

    pub async fn get(&self, id: &str) -> Result<Option<Run>, CoreError> {
        let row: Option<RunRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT id, trigger, actor, {STATUS_AS_READ}, input_excerpt, outcome_json, error_code,
                    logical_date, started_at, finished_at
             FROM agent_runs WHERE id = $3"
        )))
        .bind(self.interrupted_cutoff())
        .bind(self.expiry_cutoff())
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        row.map(run_from_row).transpose()
    }

    /// The run's steps in the order they happened.
    pub async fn steps(&self, run_id: &str) -> Result<Vec<Step>, CoreError> {
        let rows: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT id, kind, detail_json, created_at FROM agent_steps WHERE run_id = $1
             ORDER BY seq",
        )
        .bind(run_id)
        .fetch_all(&self.db)
        .await?;
        rows.into_iter()
            .map(|(id, kind, detail, created_at)| {
                Ok(Step {
                    id,
                    kind: StepKind::parse(&kind)?,
                    detail: serde_json::from_str(&detail)?,
                    created_at: parse_instant(&created_at)?,
                })
            })
            .collect()
    }

    /// Counts for runs started since `since`.
    pub async fn counts(&self, since: DateTime<Utc>) -> Result<ActivityCounts, CoreError> {
        let since = since.to_rfc3339();
        let (runs_by_user, runs_by_scheduler): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*) FILTER (WHERE actor = 'user'),
                    COUNT(*) FILTER (WHERE actor = 'scheduler')
             FROM agent_runs WHERE started_at >= $1",
        )
        .bind(&since)
        .fetch_one(&self.db)
        .await?;
        let (requested, approved, rejected, guardrails): (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*) FILTER (WHERE s.kind = 'approval_requested'),
                    COUNT(*) FILTER (WHERE s.kind = 'approval_resolved'
                                     AND s.detail_json::jsonb->>'decision' = 'approved'),
                    COUNT(*) FILTER (WHERE s.kind = 'approval_resolved'
                                     AND s.detail_json::jsonb->>'decision' = 'rejected'),
                    COUNT(*) FILTER (WHERE s.kind = 'guardrail')
             FROM agent_steps s JOIN agent_runs r ON r.id = s.run_id
             WHERE r.started_at >= $1",
        )
        .bind(&since)
        .fetch_one(&self.db)
        .await?;
        let count = |n: i64| u64::try_from(n).unwrap_or(0);
        Ok(ActivityCounts {
            runs_by_user: count(runs_by_user),
            runs_by_scheduler: count(runs_by_scheduler),
            approvals_requested: count(requested),
            approvals_approved: count(approved),
            approvals_rejected: count(rejected),
            guardrails: count(guardrails),
        })
    }

    /// Deletes runs (and by cascade their steps) started more than [`RETENTION_DAYS`] ago.
    /// Returns how many runs went.
    pub async fn prune(&self) -> Result<u64, CoreError> {
        let cutoff = self.clock.now() - Duration::days(RETENTION_DAYS);
        let result = sqlx::query("DELETE FROM agent_runs WHERE started_at < $1")
            .bind(cutoff.to_rfc3339())
            .execute(&self.db)
            .await?;
        Ok(result.rows_affected())
    }
}

fn run_from_row(row: RunRow) -> Result<Run, CoreError> {
    let (
        id,
        trigger,
        actor,
        status,
        input_excerpt,
        outcome,
        error_code,
        logical_date,
        started_at,
        finished_at,
    ) = row;
    Ok(Run {
        id,
        trigger: RunTrigger::parse(&trigger)?,
        actor: Actor::parse(&actor)?,
        status: RunStatus::parse(&status)?,
        input_excerpt,
        outcome: outcome.as_deref().map(serde_json::from_str).transpose()?,
        error_code,
        logical_date: NaiveDate::parse_from_str(&logical_date, "%Y-%m-%d")
            .map_err(|e| CoreError::Validation(format!("bad logical_date in db: {e}")))?,
        started_at: parse_instant(&started_at)?,
        finished_at: finished_at.as_deref().map(parse_instant).transpose()?,
    })
}

fn parse_instant(s: &str) -> Result<DateTime<Utc>, CoreError> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| CoreError::Validation(format!("bad instant in db {s:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::settings::Settings;

    async fn log() -> (AuditLog, Arc<FixedClock>, crate::db::TestDb) {
        let (db, dir) = crate::db::test_pool().await;
        let clock = Arc::new(FixedClock::new(
            DateTime::parse_from_rfc3339("2026-10-07T03:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        ));
        let events = EventStore::new(db.clone(), clock.clone(), Settings::new(db.clone()));
        (AuditLog::new(db, clock.clone(), events), clock, dir)
    }

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 7).unwrap()
    }

    #[tokio::test]
    async fn a_run_records_its_steps_in_order_and_its_outcome() {
        let (log, clock, _db) = log().await;
        let approvals = crate::approvals::ApprovalStore::new(log.db.clone(), clock.clone());
        let mut rx = log.events.subscribe();
        let run = log
            .start(
                RunTrigger::ChatTurn,
                Actor::User,
                Some("  早饭吃了燕麦  "),
                date(),
            )
            .await
            .unwrap();
        scope(run.clone(), async {
            record(
                StepKind::ModelCall,
                json!({"role": "conversation", "prompt_tokens": 100, "completion_tokens": 20,
                       "latency_ms": 300, "ok": true}),
            )
            .await;
            record(
                StepKind::ToolCall,
                tool_call_detail("diet_log_meal", &json!({"text": "燕麦"}), "ran", None, None),
            )
            .await;
            let action = approvals
                .create("core_add_preference", &json!({}), &json!({}))
                .await
                .unwrap();
            record(
                StepKind::ApprovalRequested,
                json!({"tool": "core_add_preference", "action_id": action.id}),
            )
            .await;
        })
        .await;
        run.finish(RunEnd::Ok).await;

        let stored = log.get(run.id()).await.unwrap().unwrap();
        assert_eq!(stored.status, RunStatus::NeedsApproval);
        assert_eq!(stored.trigger, RunTrigger::ChatTurn);
        assert_eq!(stored.input_excerpt.as_deref(), Some("早饭吃了燕麦"));
        let outcome = stored.outcome.unwrap();
        assert_eq!(outcome.model_calls, 1);
        assert_eq!(outcome.tokens, 120);
        assert_eq!(outcome.tool_calls, 1);
        assert_eq!(outcome.pending_actions, 1);

        let kinds: Vec<StepKind> = log
            .steps(run.id())
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                StepKind::ModelCall,
                StepKind::ToolCall,
                StepKind::ApprovalRequested
            ]
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(AppEvent::ActivityRecorded { run_id }) if run_id == run.id()
        ));
    }

    #[tokio::test]
    async fn a_run_stops_needing_approval_once_its_actions_expire() {
        let (log, clock, _db) = log().await;
        let approvals = crate::approvals::ApprovalStore::new(log.db.clone(), clock.clone());
        let run = log
            .start(RunTrigger::ChatTurn, Actor::User, None, date())
            .await
            .unwrap();
        scope(run.clone(), async {
            approvals
                .create("core_add_preference", &json!({}), &json!({}))
                .await
                .unwrap();
            record(StepKind::ApprovalRequested, json!({})).await;
        })
        .await;
        run.finish(RunEnd::Ok).await;
        let status =
            |log: AuditLog, id: String| async move { log.get(&id).await.unwrap().unwrap().status };
        assert_eq!(
            status(log.clone(), run.id().to_string()).await,
            RunStatus::NeedsApproval
        );

        clock.advance(Duration::hours(crate::approvals::EXPIRY_HOURS + 1));
        assert_eq!(
            status(log.clone(), run.id().to_string()).await,
            RunStatus::Succeeded
        );
    }

    #[tokio::test]
    async fn a_failed_run_keeps_its_error_code() {
        let (log, _clock, _db) = log().await;
        let run = log
            .start(RunTrigger::PlanScheduled, Actor::Scheduler, None, date())
            .await
            .unwrap();
        run.finish(RunEnd::Failed {
            code: "budget_exceeded",
        })
        .await;
        let stored = log.get(run.id()).await.unwrap().unwrap();
        assert_eq!(stored.status, RunStatus::Failed);
        assert_eq!(stored.error_code.as_deref(), Some("budget_exceeded"));
        assert_eq!(stored.input_excerpt, None);
    }

    #[tokio::test]
    async fn steps_outside_a_run_are_not_recorded() {
        let (log, _clock, _db) = log().await;
        assert!(current().is_none());
        record(StepKind::Guardrail, json!({"rule": "unknown_tool"})).await;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_steps")
            .fetch_one(&log.db)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn concurrent_runs_keep_their_own_steps() {
        let (log, _clock, _db) = log().await;
        let a = log
            .start(RunTrigger::ChatTurn, Actor::User, None, date())
            .await
            .unwrap();
        let b = log
            .start(RunTrigger::PlanManual, Actor::User, None, date())
            .await
            .unwrap();
        let step = |n: u32| async move {
            for _ in 0..n {
                tokio::task::yield_now().await;
                record(StepKind::Guardrail, json!({"rule": "invalid_args"})).await;
            }
        };
        tokio::join!(scope(a.clone(), step(3)), scope(b.clone(), step(2)));
        assert_eq!(log.steps(a.id()).await.unwrap().len(), 3);
        assert_eq!(log.steps(b.id()).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_run_left_running_reads_as_interrupted() {
        let (log, clock, _db) = log().await;
        let run = log
            .start(RunTrigger::ChatTurn, Actor::User, None, date())
            .await
            .unwrap();
        assert_eq!(
            log.get(run.id()).await.unwrap().unwrap().status,
            RunStatus::Running
        );
        clock.advance(Duration::minutes(INTERRUPTED_AFTER_MINUTES + 1));
        assert_eq!(
            log.get(run.id()).await.unwrap().unwrap().status,
            RunStatus::Interrupted
        );
        let filter = RunFilter {
            status: Some(RunStatus::Interrupted),
            ..Default::default()
        };
        assert_eq!(log.list(&filter, 10).await.unwrap().runs.len(), 1);
    }

    #[tokio::test]
    async fn list_pages_newest_first_and_filters() {
        let (log, _clock, _db) = log().await;
        let mut ids = Vec::new();
        for i in 0..5 {
            let actor = if i % 2 == 0 {
                Actor::User
            } else {
                Actor::Scheduler
            };
            let run = log
                .start(RunTrigger::ChatTurn, actor, None, date())
                .await
                .unwrap();
            run.finish(RunEnd::Ok).await;
            ids.push(run.id().to_string());
        }
        let first = log.list(&RunFilter::default(), 2).await.unwrap();
        assert_eq!(
            first.runs.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
            vec![ids[4].clone(), ids[3].clone()]
        );
        let second = log
            .list(
                &RunFilter {
                    before: first.next_before.clone(),
                    ..Default::default()
                },
                2,
            )
            .await
            .unwrap();
        assert_eq!(second.runs[0].id, ids[2]);
        let last = log
            .list(
                &RunFilter {
                    before: second.next_before.clone(),
                    ..Default::default()
                },
                2,
            )
            .await
            .unwrap();
        assert_eq!(last.runs.len(), 1);
        assert_eq!(last.next_before, None);

        let scheduler = RunFilter {
            actor: Some(Actor::Scheduler),
            ..Default::default()
        };
        assert_eq!(log.list(&scheduler, 10).await.unwrap().runs.len(), 2);
    }

    #[tokio::test]
    async fn prune_removes_runs_past_retention_with_their_steps() {
        let (log, clock, _db) = log().await;
        let old = log
            .start(RunTrigger::ChatTurn, Actor::User, None, date())
            .await
            .unwrap();
        old.step(StepKind::Guardrail, json!({"rule": "iteration_cap"}))
            .await;
        clock.advance(Duration::days(RETENTION_DAYS) + Duration::hours(1));
        let recent = log
            .start(RunTrigger::ChatTurn, Actor::User, None, date())
            .await
            .unwrap();
        assert_eq!(log.prune().await.unwrap(), 1);
        assert!(log.get(old.id()).await.unwrap().is_none());
        assert!(log.steps(old.id()).await.unwrap().is_empty());
        assert!(log.get(recent.id()).await.unwrap().is_some());
    }

    #[test]
    fn long_arguments_are_stored_cut() {
        let big = json!({"text": "あ".repeat(MAX_ARGUMENTS_BYTES)});
        let detail = tool_call_detail("diet_log_meal", &big, "ran", None, None);
        assert_eq!(detail["arguments"]["truncated"], true);
        assert!(detail["arguments"]["text"].as_str().unwrap().len() <= MAX_ARGUMENTS_BYTES);
        let small = tool_call_detail("diet_log_meal", &json!({"a": 1}), "ran", None, None);
        assert_eq!(small["arguments"], json!({"a": 1}));
    }

    #[test]
    fn excerpts_are_cut_to_the_limit() {
        assert_eq!(excerpt(&"字".repeat(500)).chars().count(), EXCERPT_CHARS);
    }

    #[tokio::test]
    async fn counts_cover_runs_and_steps_since_a_time() {
        let (log, clock, _db) = log().await;
        let since = clock.now();
        let run = log
            .start(RunTrigger::ChatTurn, Actor::User, None, date())
            .await
            .unwrap();
        run.step(StepKind::ApprovalRequested, json!({"tool": "t"}))
            .await;
        run.step(StepKind::Guardrail, json!({"rule": "invalid_args"}))
            .await;
        let approval = log
            .start(RunTrigger::ActionApproved, Actor::User, None, date())
            .await
            .unwrap();
        approval
            .step(
                StepKind::ApprovalResolved,
                json!({"tool": "t", "decision": "approved"}),
            )
            .await;
        log.start(RunTrigger::PlanScheduled, Actor::Scheduler, None, date())
            .await
            .unwrap();
        let counts = log.counts(since).await.unwrap();
        assert_eq!(
            counts,
            ActivityCounts {
                runs_by_user: 2,
                runs_by_scheduler: 1,
                approvals_requested: 1,
                approvals_approved: 1,
                approvals_rejected: 0,
                guardrails: 1,
            }
        );
    }
}
