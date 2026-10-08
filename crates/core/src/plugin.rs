//! `PluginManifest`, `PluginContext`, the `Plugin` trait and `PluginError`, per
//! `docs/PHASE1_SPEC.md` §7.6, plus plugin-migration application (§5 "Plugin migrations").

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use lifeagent_llm::error::LlmError;
use lifeagent_llm::router::LlmRouter;
use lifeagent_llm::types::ToolSpec;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use thiserror::Error;

use crate::clock::Clock;
use crate::error::CoreError;
use crate::events::{Event, EventStore};
use crate::goals::GoalsService;
use crate::plan::PlanStore;
use crate::preferences::PreferenceService;
use crate::profile::ProfileService;
use crate::settings::Settings;

/// One versioned schema change a plugin supplies. `sql` must be a single statement; core
/// applies missing versions in order, each inside its own transaction, and records them in
/// `plugin_migrations`. Plugin tables must be prefixed with the plugin id (e.g. `diet_`).
pub struct PluginMigration {
    pub version: u32,
    pub sql: &'static str,
}

pub struct PluginManifest {
    /// Also the tool-name and table prefix (e.g. `"diet"` -> tools `diet_*`, tables `diet_*`).
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub event_types: Vec<&'static str>,
    pub migrations: Vec<PluginMigration>,
}

/// Everything a plugin needs to do its work. Assembled fresh by `CoreApp` for each call —
/// cheap, since every field is a cloneable handle (`PgPool`, `Arc<...>`, or a service
/// wrapping one).
#[derive(Clone)]
pub struct PluginContext {
    pub db: PgPool,
    pub events: EventStore,
    pub llm: Arc<LlmRouter>,
    pub clock: Arc<dyn Clock>,
    pub settings: Settings,
    /// Profile access. Active goals and the latest weight (§7.6's "profile, active goals,
    /// latest weight") live on `goals` and `events` respectively rather than being folded into
    /// this one field.
    pub profile: ProfileService,
    pub goals: GoalsService,
    pub preferences: PreferenceService,
    /// Read access to stored plans (plan-vs-actual, context snippets). Not in §7.6's field
    /// list; see M4 in `docs/DECISIONS.md`.
    pub plans: PlanStore,
}

impl PluginContext {
    /// The current logical date (§4), using the `day_boundary` setting.
    pub async fn today(&self) -> Result<NaiveDate, CoreError> {
        let boundary = self.settings.day_boundary().await?;
        Ok(crate::time::logical_date(self.clock.now(), boundary))
    }
}

/// What a tool call returns: JSON handed back to the LLM as the tool result, plus a summary of
/// what it did for the UI's activity chips and the activity log.
pub struct ToolOutput {
    pub content: Value,
    pub summary: ToolSummary,
}

/// What a tool call did, as data each UI words in its own language (`toolSummaries` in
/// `apps/web/src/i18n/`): `{"kind": ..., ...params}`, like approval previews. Kinds are
/// `snake_case` and start with the owning plugin's id (`core_` for core tools), so plugins
/// can't collide. Numbers in params are the ones Rust computed. See
/// `docs/AUDIT_LOG_SPEC.md` §3.6.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolSummary(pub Value);

impl ToolSummary {
    pub fn new(kind: &str) -> Self {
        Self(serde_json::json!({ "kind": kind }))
    }

    /// Adds one param. A value that can't be serialized (never the case for the plain data
    /// tools pass) becomes `null` rather than failing the tool call.
    pub fn with(mut self, key: &str, value: impl Serialize) -> Self {
        if let Value::Object(map) = &mut self.0 {
            map.insert(
                key.to_string(),
                serde_json::to_value(value).unwrap_or(Value::Null),
            );
        }
        self
    }

    pub fn kind(&self) -> &str {
        self.0.get("kind").and_then(Value::as_str).unwrap_or("")
    }
}

/// `kind key=value ...`, for the CLI and test messages; the UI words summaries itself.
impl std::fmt::Display for ToolSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.kind())?;
        if let Value::Object(map) = &self.0 {
            for (key, value) in map.iter().filter(|(k, _)| k.as_str() != "kind") {
                match value {
                    Value::String(s) => write!(f, " {key}={s}")?,
                    other => write!(f, " {key}={other}")?,
                }
            }
        }
        Ok(())
    }
}

/// A plugin's contribution to the next plan (M5).
pub struct PlanInput {
    pub section: String,
    pub brief: String,
    pub targets: Value,
    pub output_schema: Value,
}

/// Finalized sections (section id -> `PlanSection::data`) from earlier planning stages, handed
/// to a plugin that declared them in [`Plugin::plan_after`]. Empty in the first stage, and for
/// plugins that depend on nothing. A section a plugin depends on but that isn't being planned
/// (its plugin disabled, or it had no plan input) is simply absent.
pub type PlanDraft = BTreeMap<String, Value>;

/// The result of validating and enriching a plugin's raw planner output for one section (M5).
pub struct PlanSection {
    pub data: Value,
    pub warnings: Vec<PlanNote>,
    pub violations: Vec<PlanNote>,
}

/// A plan warning or violation, as data each UI words in its own language (`planNotes` in
/// `apps/web/src/i18n/`): `{"kind": ..., "text": ..., ...params}`, like [`ToolSummary`]. Kinds
/// are `snake_case` and start with the owning plugin's id (`core_` for core). `text` is the
/// English wording: the planner's retry message uses it, and a UI shows it for a kind it doesn't
/// know. Numbers in params are the ones Rust computed. Plans stored before notes were data hold
/// plain zh-CN strings, which deserialize as a bare JSON string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlanNote(pub Value);

impl PlanNote {
    pub fn new(kind: &str, text: impl Into<String>) -> Self {
        Self(serde_json::json!({ "kind": kind, "text": text.into() }))
    }

    /// Adds one param, like [`ToolSummary::with`].
    pub fn with(mut self, key: &str, value: impl Serialize) -> Self {
        if let Value::Object(map) = &mut self.0 {
            map.insert(
                key.to_string(),
                serde_json::to_value(value).unwrap_or(Value::Null),
            );
        }
        self
    }

    /// `""` for a legacy plain-string note.
    pub fn kind(&self) -> &str {
        self.0.get("kind").and_then(Value::as_str).unwrap_or("")
    }

    pub fn text(&self) -> &str {
        match &self.0 {
            Value::String(s) => s,
            other => other.get("text").and_then(Value::as_str).unwrap_or(""),
        }
    }
}

impl std::fmt::Display for PlanNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.text())
    }
}

#[derive(Debug, Error)]
pub enum PluginError {
    /// Returned to the LLM as a tool error so it can fix the call.
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("unsupported")]
    Unsupported,
    #[error("{0}")]
    Other(String),
}

impl From<CoreError> for PluginError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::NotFound(m) => PluginError::NotFound(m),
            CoreError::Validation(m) => PluginError::InvalidArgs(m),
            // Same bucket as `Validation`: a named rule is still just an argument the model can
            // fix (a note it should trim, a weight it should re-ask about), so it gets the rule's
            // own English detail rather than falling into `Other`'s generic message below.
            CoreError::InvalidInput(rule) => PluginError::InvalidArgs(rule.to_string()),
            // Passed through (as NotFound) so the model can tell the user what's missing.
            e @ CoreError::NothingToPlan => PluginError::NotFound(e.to_string()),
            other => PluginError::Other(other.to_string()),
        }
    }
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> PluginManifest;
    fn tools(&self) -> Vec<ToolSpec>;
    async fn call_tool(
        &self,
        ctx: &PluginContext,
        name: &str,
        args: Value,
    ) -> Result<ToolOutput, PluginError>;

    /// For a call of one of this plugin's tools from chat: `Some(preview)` if it must wait for
    /// the user's approval in the app instead of running now (`crate::approvals`). The preview is
    /// `{"kind": "<snake_case>", ...}`, data the UI words in the user's language. Called with
    /// arguments that already passed the tool's schema. By default, nothing needs approval.
    async fn approval_preview(
        &self,
        _ctx: &PluginContext,
        _name: &str,
        _args: &Value,
    ) -> Result<Option<Value>, PluginError> {
        Ok(None)
    }

    async fn on_event(&self, _ctx: &PluginContext, _e: &Event) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called after an event is soft-deleted (`core_delete_entry`, the UI's delete button), so
    /// a plugin can undo whatever its `on_event` did for it (e.g. stock a meal took out).
    async fn on_event_deleted(&self, _ctx: &PluginContext, _e: &Event) -> Result<(), PluginError> {
        Ok(())
    }

    async fn context_snippet(
        &self,
        _ctx: &PluginContext,
        _date: NaiveDate,
    ) -> Result<Option<String>, PluginError> {
        Ok(None)
    }

    async fn today_card(
        &self,
        _ctx: &PluginContext,
        _date: NaiveDate,
    ) -> Result<Option<Value>, PluginError> {
        Ok(None)
    }

    /// Optional plugins can be switched off by the user and start switched off (unless
    /// `enabled_by_default`); the rest are always on. A disabled plugin keeps its tables and
    /// events, but contributes no tools, context, today card or plan section, and its
    /// `on_event` isn't called, until it is enabled again. `on_event_deleted` still is, so it
    /// can undo what it did while it was on.
    fn optional(&self) -> bool {
        false
    }

    /// For an optional plugin: whether it is on before the user has touched its switch.
    fn enabled_by_default(&self) -> bool {
        false
    }

    /// Plan sections this plugin's own section builds on (e.g. diet after `"training"`). They are
    /// planned and finalized in an earlier planner call, and passed to `plan_input` and
    /// `finalize_plan_section` as the [`PlanDraft`].
    fn plan_after(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether this plugin's plan section is built from stored data, without the planner LLM
    /// (`docs/MULTIMEMBER_SPEC.md` §3.1). Such a plugin implements
    /// [`Plugin::computed_plan_section`] instead of `plan_input`/`finalize_plan_section`. Its
    /// section is handed to later stages in the [`PlanDraft`] and stored with the plan, but never
    /// shown to the planner LLM (not in "already planned", not in a revision's previous plan):
    /// a plugin that reads it puts what the model needs into its own brief.
    fn computes_plan_section(&self) -> bool {
        false
    }

    /// The section for `date`, for a plugin whose [`Plugin::computes_plan_section`] is true.
    /// `None`: no section that day. Called in the plugin's planning stage before any LLM call.
    /// There is nothing to retry, so any `violations` are kept as warnings.
    async fn computed_plan_section(
        &self,
        _ctx: &PluginContext,
        _date: NaiveDate,
        _prior: &PlanDraft,
    ) -> Result<Option<PlanSection>, PluginError> {
        Ok(None)
    }

    async fn plan_input(
        &self,
        _ctx: &PluginContext,
        _date: NaiveDate,
        _prior: &PlanDraft,
    ) -> Result<Option<PlanInput>, PluginError> {
        Ok(None)
    }

    async fn finalize_plan_section(
        &self,
        _ctx: &PluginContext,
        _date: NaiveDate,
        _raw: Value,
        _prior: &PlanDraft,
    ) -> Result<PlanSection, PluginError> {
        Err(PluginError::Unsupported)
    }
}

/// The plugins currently switched on, in registration order: every non-optional plugin, plus
/// the optional ones switched on (`Settings::plugins_enabled`, else `enabled_by_default`).
pub async fn enabled_plugins(
    settings: &Settings,
    plugins: &[Arc<dyn Plugin>],
) -> Result<Vec<Arc<dyn Plugin>>, CoreError> {
    let switches = settings.plugins_enabled().await?;
    Ok(plugins
        .iter()
        .filter(|p| {
            !p.optional()
                || switches
                    .get(p.manifest().id)
                    .copied()
                    .unwrap_or_else(|| p.enabled_by_default())
        })
        .cloned()
        .collect())
}

/// Applies any of `manifest`'s migrations not yet recorded in `plugin_migrations`, in version
/// order, each inside its own transaction (§5 "Plugin migrations").
pub async fn apply_migrations(
    db: &PgPool,
    clock: &dyn Clock,
    manifest: &PluginManifest,
) -> Result<(), CoreError> {
    let applied_rows: Vec<(i64,)> =
        sqlx::query_as("SELECT version FROM plugin_migrations WHERE plugin_id = $1")
            .bind(manifest.id)
            .fetch_all(db)
            .await?;
    let applied: HashSet<i64> = applied_rows.into_iter().map(|(v,)| v).collect();

    let mut migrations: Vec<&PluginMigration> = manifest.migrations.iter().collect();
    migrations.sort_by_key(|m| m.version);

    for migration in migrations {
        if applied.contains(&(migration.version as i64)) {
            continue;
        }
        let mut tx = db.begin().await?;
        // Two app instances opening the same schema at once (e.g. two Cloud Run instances)
        // must not both apply a migration: serialize on a per-schema lock held until commit,
        // then re-check under it.
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtext('plugin_migrations.' || current_schema()))",
        )
        .execute(&mut *tx)
        .await?;
        let already: Option<(i64,)> = sqlx::query_as(
            "SELECT version FROM plugin_migrations WHERE plugin_id = $1 AND version = $2",
        )
        .bind(manifest.id)
        .bind(migration.version as i64)
        .fetch_optional(&mut *tx)
        .await?;
        if already.is_some() {
            continue;
        }
        // `raw_sql`: a migration may hold several statements, which a prepared statement can't.
        sqlx::raw_sql(migration.sql).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO plugin_migrations (plugin_id, version, applied_at) VALUES ($1, $2, $3)",
        )
        .bind(manifest.id)
        .bind(migration.version as i64)
        .bind(clock.now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn plan_notes_read_new_and_legacy_shapes() {
        let note = PlanNote::new("diet_x", "English text").with("grams", 12.5);
        assert_eq!(note.kind(), "diet_x");
        assert_eq!(note.to_string(), "English text");
        assert_eq!(note.0["grams"], 12.5);

        // A warning stored before notes were data.
        let legacy: Vec<PlanNote> = serde_json::from_str(r#"["脂肪偏高"]"#).unwrap();
        assert_eq!(legacy[0].kind(), "");
        assert_eq!(legacy[0].text(), "脂肪偏高");
    }

    fn manifest(migrations: Vec<PluginMigration>) -> PluginManifest {
        PluginManifest {
            id: "test",
            name: "Test",
            version: "0.1.0",
            event_types: vec![],
            migrations,
        }
    }

    #[tokio::test]
    async fn applies_missing_migrations_in_order_and_records_them() {
        let (db, _dir) = crate::db::test_pool().await;
        let clock = crate::clock::FixedClock::new(Utc::now());

        let manifest = manifest(vec![
            PluginMigration {
                version: 1,
                sql: "CREATE TABLE test_things (id TEXT PRIMARY KEY)",
            },
            PluginMigration {
                version: 2,
                sql: "ALTER TABLE test_things ADD COLUMN name TEXT",
            },
        ]);

        apply_migrations(&db, &clock, &manifest).await.unwrap();

        // Both migrations landed: the table has the column the second one added.
        sqlx::query("INSERT INTO test_things (id, name) VALUES ('a', 'b')")
            .execute(&db)
            .await
            .unwrap();

        let applied: Vec<(i64,)> = sqlx::query_as(
            "SELECT version FROM plugin_migrations WHERE plugin_id = 'test' ORDER BY version",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(applied, vec![(1,), (2,)]);
    }

    #[tokio::test]
    async fn does_not_reapply_already_applied_migrations() {
        let (db, _dir) = crate::db::test_pool().await;
        let clock = crate::clock::FixedClock::new(Utc::now());
        let manifest = manifest(vec![PluginMigration {
            version: 1,
            sql: "CREATE TABLE test_things (id TEXT PRIMARY KEY)",
        }]);

        apply_migrations(&db, &clock, &manifest).await.unwrap();
        // A second run must not try to CREATE TABLE again (which would error).
        apply_migrations(&db, &clock, &manifest).await.unwrap();
    }

    #[tokio::test]
    async fn only_applies_migrations_not_yet_recorded() {
        let (db, _dir) = crate::db::test_pool().await;
        let clock = crate::clock::FixedClock::new(Utc::now());

        apply_migrations(
            &db,
            &clock,
            &manifest(vec![PluginMigration {
                version: 1,
                sql: "CREATE TABLE test_things (id TEXT PRIMARY KEY)",
            }]),
        )
        .await
        .unwrap();

        // A later run with an additional migration applies only the new one.
        apply_migrations(
            &db,
            &clock,
            &manifest(vec![
                PluginMigration {
                    version: 1,
                    sql: "CREATE TABLE test_things (id TEXT PRIMARY KEY)",
                },
                PluginMigration {
                    version: 2,
                    sql: "ALTER TABLE test_things ADD COLUMN name TEXT",
                },
            ]),
        )
        .await
        .unwrap();

        let applied: Vec<(i64,)> = sqlx::query_as(
            "SELECT version FROM plugin_migrations WHERE plugin_id = 'test' ORDER BY version",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(applied, vec![(1,), (2,)]);
    }

    #[test]
    fn plugin_error_from_core_error_maps_variants() {
        assert!(matches!(
            PluginError::from(CoreError::NotFound("x".into())),
            PluginError::NotFound(_)
        ));
        assert!(matches!(
            PluginError::from(CoreError::Validation("x".into())),
            PluginError::InvalidArgs(_)
        ));
        // A named rule (e.g. `core_add_preference`/`core_revise_plan`'s empty-string checks) must stay
        // in the same `InvalidArgs` bucket as `Validation`, not fall into `Other`'s generic
        // "internal error" — the model needs the detail to retry or explain itself to the user.
        assert!(matches!(
            PluginError::from(CoreError::InvalidInput(
                crate::error::ValidationRule::NoteEmpty
            )),
            PluginError::InvalidArgs(_)
        ));
        assert!(matches!(
            PluginError::from(CoreError::Secret("x".into())),
            PluginError::Other(_)
        ));
    }
}
