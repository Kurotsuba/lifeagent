//! Plan types, storage (`PlanStore`) and the plan service (`PlanService`: generate, retry,
//! revise), per `docs/PHASE1_SPEC.md` §7.12 and the planner prompt in §9.4.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use lifeagent_llm::json::complete_json;
use lifeagent_llm::types::{Message, Role};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::Mutex;

use crate::clock::Clock;
use crate::error::CoreError;
use crate::events::AppEvent;
use crate::i18n::USER_LANGUAGE;
use crate::plugin::{PlanInput, PlanSection, Plugin, PluginContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Proposed,
    Accepted,
}

impl PlanStatus {
    fn as_str(&self) -> &'static str {
        match self {
            PlanStatus::Proposed => "proposed",
            PlanStatus::Accepted => "accepted",
        }
    }

    fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "proposed" => Ok(PlanStatus::Proposed),
            "accepted" => Ok(PlanStatus::Accepted),
            other => Err(CoreError::Validation(format!(
                "unknown plan status {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanTrigger {
    Schedule,
    Manual,
    Revision,
}

impl PlanTrigger {
    fn as_str(&self) -> &'static str {
        match self {
            PlanTrigger::Schedule => "schedule",
            PlanTrigger::Manual => "manual",
            PlanTrigger::Revision => "revision",
        }
    }

    fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "schedule" => Ok(PlanTrigger::Schedule),
            "manual" => Ok(PlanTrigger::Manual),
            "revision" => Ok(PlanTrigger::Revision),
            other => Err(CoreError::Validation(format!(
                "unknown plan trigger {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanContent {
    /// Short message to the user.
    pub summary: String,
    /// Plugin id -> that plugin's finalized section data.
    pub sections: BTreeMap<String, Value>,
    pub disclaimer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredPlan {
    pub plan_date: NaiveDate,
    pub version: u32,
    pub status: PlanStatus,
    pub trigger: PlanTrigger,
    /// The user's change request, for `revision` plans.
    pub instruction: Option<String>,
    pub content: PlanContent,
    pub warnings: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct PlanStore {
    db: SqlitePool,
    clock: Arc<dyn Clock>,
}

/// (plan_date, version, status, trigger, instruction, content_json, warnings_json, created_at)
type PlanRow = (
    String,
    i64,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
);

impl PlanStore {
    pub fn new(db: SqlitePool, clock: Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }

    /// The highest version stored for `date`, if any.
    pub async fn current(&self, date: NaiveDate) -> Result<Option<StoredPlan>, CoreError> {
        let row: Option<PlanRow> = sqlx::query_as(
            "SELECT plan_date, version, status, trigger, instruction, content_json, warnings_json,
                    created_at
             FROM plans WHERE plan_date = ? ORDER BY version DESC LIMIT 1",
        )
        .bind(date.to_string())
        .fetch_optional(&self.db)
        .await?;
        row.map(Self::row_to_plan).transpose()
    }

    /// Stores a new `proposed` plan as version = previous max + 1 (1 for the first).
    pub async fn insert(
        &self,
        date: NaiveDate,
        trigger: PlanTrigger,
        instruction: Option<&str>,
        content: &PlanContent,
        warnings: &[String],
    ) -> Result<StoredPlan, CoreError> {
        let mut tx = self.db.begin().await?;
        let max: Option<i64> =
            sqlx::query_scalar("SELECT MAX(version) FROM plans WHERE plan_date = ?")
                .bind(date.to_string())
                .fetch_one(&mut *tx)
                .await?;
        let version = max.unwrap_or(0) + 1;
        let created_at = self.clock.now();
        let status = PlanStatus::Proposed;

        sqlx::query(
            "INSERT INTO plans
                (plan_date, version, status, trigger, instruction, content_json, warnings_json,
                 created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(date.to_string())
        .bind(version)
        .bind(status.as_str())
        .bind(trigger.as_str())
        .bind(instruction)
        .bind(serde_json::to_string(content)?)
        .bind(serde_json::to_string(warnings)?)
        .bind(created_at.to_rfc3339())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(StoredPlan {
            plan_date: date,
            version: version as u32,
            status,
            trigger,
            instruction: instruction.map(str::to_string),
            content: content.clone(),
            warnings: warnings.to_vec(),
            created_at,
        })
    }

    /// Marks the current version for `date` accepted. Accepting only records the user's
    /// intent (§16); it doesn't change the plan.
    pub async fn accept(&self, date: NaiveDate) -> Result<(), CoreError> {
        let result = sqlx::query(
            "UPDATE plans SET status = ?1
             WHERE plan_date = ?2
               AND version = (SELECT MAX(version) FROM plans WHERE plan_date = ?2)",
        )
        .bind(PlanStatus::Accepted.as_str())
        .bind(date.to_string())
        .execute(&self.db)
        .await?;
        if result.rows_affected() == 0 {
            return Err(CoreError::NotFound(format!("plan for {date}")));
        }
        Ok(())
    }

    fn row_to_plan(row: PlanRow) -> Result<StoredPlan, CoreError> {
        let (plan_date, version, status, trigger, instruction, content, warnings, created_at) = row;
        Ok(StoredPlan {
            plan_date: plan_date
                .parse()
                .map_err(|e| CoreError::Validation(format!("bad plan_date in db: {e}")))?,
            version: u32::try_from(version)
                .map_err(|e| CoreError::Validation(format!("bad plan version in db: {e}")))?,
            status: PlanStatus::parse(&status)?,
            trigger: PlanTrigger::parse(&trigger)?,
            instruction,
            content: serde_json::from_str(&content)?,
            warnings: serde_json::from_str(&warnings)?,
            created_at: DateTime::parse_from_rfc3339(&created_at)
                .map_err(|e| CoreError::Validation(format!("bad created_at in db: {e}")))?
                .with_timezone(&Utc),
        })
    }
}

/// §9.4, up to the per-section parts. Placeholders: `{user_language}`, `{plan_date}`,
/// `{weekday}`, `{profile_summary}`, `{memory_notes}`.
const PLANNER_HEADER: &str = r#"You create tomorrow's plan for the user. The plan has one section per domain below. Each
section must satisfy its targets and constraints and match its output schema exactly.

General rules:
- Propose realistic, easy-to-prepare meals that fit the user's habits, preferences and memory
  notes. Prefer common foods available in ordinary supermarkets and canteens.
- Use generic food names in the user's language, with grams for every item as eaten.
- Hit the numeric targets; the app recomputes nutrition with its own food table and will
  reject plans that miss them.
- Allergies are absolute. Avoid disliked foods.
- Vary dishes compared with recent days.
- "summary": 2–4 sentences to the user in {user_language}: the plan's focus for tomorrow,
  referencing recent adherence if helpful. Encouraging, never guilt-tripping.

Return JSON: {"summary": string, "sections": {<section id>: <section object>}}

Date: {plan_date} ({weekday})

## User profile
{profile_summary}

## Memory notes
{memory_notes}"#;

/// §9.4, once per section. Placeholders: `{section}`, `{targets_json}`, `{brief}`,
/// `{output_schema_json}`.
const PLANNER_SECTION: &str = r#"## Section "{section}"
Targets: {targets_json}
Brief:
{brief}
Output schema: {output_schema_json}"#;

/// §9.4, revisions only. Placeholders: `{version}`, `{previous_content_json}`, `{instruction}`.
const PLANNER_REVISION: &str = r#"## Current plan (version {version})
{previous_content_json}
## User's change request
{instruction}
Keep everything not affected by the request as close to the current plan as possible."#;

/// §7.12 step 6: prefix for violations still unmet after the retry, moved into warnings.
const UNMET_PREFIX: &str = "未完全满足：";

/// §7.12 step 7.
const MEDICAL_DISCLAIMER: &str =
    "你的档案中记录了健康状况，本计划仅为生活方式建议，请以医生或营养师意见为准。";

/// What the planner returns (§7.12 step 3).
#[derive(Debug, Serialize, Deserialize)]
struct PlannerOutput {
    summary: String,
    #[serde(default)]
    sections: BTreeMap<String, Value>,
}

/// Generates and revises plans. One generation runs at a time; a second request waits.
pub struct PlanService {
    plugins: Vec<Arc<dyn Plugin>>,
    lock: Mutex<()>,
}

impl PlanService {
    pub fn new(plugins: Vec<Arc<dyn Plugin>>) -> Self {
        Self {
            plugins,
            lock: Mutex::new(()),
        }
    }

    /// Generates a new plan version for `date` (§7.12).
    pub async fn generate(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
        trigger: PlanTrigger,
    ) -> Result<StoredPlan, CoreError> {
        self.run(ctx, date, trigger, None).await
    }

    /// Revises the current plan for `date` following the user's `instruction`. Stored as a
    /// new version with trigger `revision` and the instruction.
    pub async fn revise(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
        instruction: &str,
    ) -> Result<StoredPlan, CoreError> {
        let instruction = instruction.trim();
        if instruction.is_empty() {
            return Err(CoreError::Validation(
                "instruction must not be empty".into(),
            ));
        }
        self.run(ctx, date, PlanTrigger::Revision, Some(instruction))
            .await
    }

    async fn run(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
        trigger: PlanTrigger,
        instruction: Option<&str>,
    ) -> Result<StoredPlan, CoreError> {
        let _guard = self.lock.lock().await;

        let previous = match instruction {
            Some(_) => Some(
                ctx.plans
                    .current(date)
                    .await?
                    .ok_or_else(|| CoreError::NotFound(format!("plan for {date}")))?,
            ),
            None => None,
        };

        let mut inputs: Vec<(Arc<dyn Plugin>, PlanInput)> = Vec::new();
        for plugin in &self.plugins {
            let input = plugin
                .plan_input(ctx, date)
                .await
                .map_err(|e| CoreError::Plugin(format!("{}: {e}", plugin.manifest().id)))?;
            if let Some(input) = input {
                inputs.push((plugin.clone(), input));
            }
        }
        if inputs.is_empty() {
            return Err(CoreError::NothingToPlan);
        }

        let prompt = planner_prompt(ctx, date, &inputs, previous.as_ref().zip(instruction)).await?;
        let mut messages = vec![Message::User { content: prompt }];
        let mut output: PlannerOutput =
            complete_json(&ctx.llm, Role::Planner, messages.clone()).await?;
        let mut sections = finalize_all(ctx, date, &inputs, &output).await?;

        if sections.iter().any(|(_, s)| !s.violations.is_empty()) {
            messages.push(Message::Assistant {
                content: Some(serde_json::to_string(&output)?),
                tool_calls: vec![],
            });
            messages.push(Message::User {
                content: violations_message(&sections),
            });
            output = complete_json(&ctx.llm, Role::Planner, messages).await?;
            sections = finalize_all(ctx, date, &inputs, &output).await?;
        }

        let mut content_sections = BTreeMap::new();
        let mut warnings = Vec::new();
        for (id, section) in sections {
            warnings.extend(section.warnings);
            warnings.extend(
                section
                    .violations
                    .iter()
                    .map(|v| format!("{UNMET_PREFIX}{v}")),
            );
            content_sections.insert(id, section.data);
        }
        let has_medical_notes = ctx
            .profile
            .get()
            .await?
            .and_then(|p| p.medical_notes)
            .is_some_and(|n| !n.trim().is_empty());
        let content = PlanContent {
            summary: output.summary,
            sections: content_sections,
            disclaimer: has_medical_notes.then(|| MEDICAL_DISCLAIMER.to_string()),
        };

        let plan = ctx
            .plans
            .insert(date, trigger, instruction, &content, &warnings)
            .await?;
        ctx.events.publish(AppEvent::PlanReady {
            date,
            version: plan.version,
            trigger,
        });
        Ok(plan)
    }
}

async fn planner_prompt(
    ctx: &PluginContext,
    date: NaiveDate,
    inputs: &[(Arc<dyn Plugin>, PlanInput)],
    revision: Option<(&StoredPlan, &str)>,
) -> Result<String, CoreError> {
    let profile = ctx.profile.get().await?;
    let latest_weight = ctx.events.latest("body.weight_logged").await?;
    let profile_summary =
        crate::context::render_profile_summary(profile.as_ref(), latest_weight.as_ref(), date);
    let notes = ctx.memory.list(50).await?;
    let memory_notes = if notes.is_empty() {
        "(none)".to_string()
    } else {
        notes
            .iter()
            .map(|n| format!("- {}", n.note))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // User-supplied text is substituted last in each part, so braces in it are never
    // mistaken for a placeholder.
    let mut parts = vec![PLANNER_HEADER
        .replace("{user_language}", USER_LANGUAGE.prompt_name())
        .replace("{plan_date}", &date.to_string())
        .replace("{weekday}", &date.format("%A").to_string())
        .replace("{profile_summary}", &profile_summary)
        .replace("{memory_notes}", &memory_notes)];
    for (_, input) in inputs {
        parts.push(
            PLANNER_SECTION
                .replace("{section}", &input.section)
                .replace("{targets_json}", &input.targets.to_string())
                .replace("{output_schema_json}", &input.output_schema.to_string())
                .replace("{brief}", &input.brief),
        );
    }
    if let Some((previous, instruction)) = revision {
        parts.push(
            PLANNER_REVISION
                .replace("{version}", &previous.version.to_string())
                .replace(
                    "{previous_content_json}",
                    &serde_json::to_string(&previous.content)?,
                )
                .replace("{instruction}", instruction),
        );
    }
    Ok(parts.join("\n\n"))
}

/// §7.12 step 4: each input section through its owner's `finalize_plan_section`; a section
/// the planner left out is a violation.
async fn finalize_all(
    ctx: &PluginContext,
    date: NaiveDate,
    inputs: &[(Arc<dyn Plugin>, PlanInput)],
    output: &PlannerOutput,
) -> Result<Vec<(String, PlanSection)>, CoreError> {
    let mut sections = Vec::with_capacity(inputs.len());
    for (plugin, input) in inputs {
        let section = match output.sections.get(&input.section) {
            Some(raw) => plugin
                .finalize_plan_section(ctx, date, raw.clone())
                .await
                .map_err(|e| CoreError::Plugin(format!("{}: {e}", plugin.manifest().id)))?,
            None => PlanSection {
                data: Value::Null,
                warnings: Vec::new(),
                violations: vec![format!("计划缺少“{}”部分", input.section)],
            },
        };
        sections.push((input.section.clone(), section));
    }
    Ok(sections)
}

/// §7.12 step 5: the retry request, listing every violation per section.
fn violations_message(sections: &[(String, PlanSection)]) -> String {
    let mut text = String::from("The plan does not meet these requirements:\n");
    for (id, section) in sections {
        if section.violations.is_empty() {
            continue;
        }
        text.push_str(&format!("\n## Section \"{id}\"\n"));
        for v in &section.violations {
            text.push_str(&format!("- {v}\n"));
        }
    }
    text.push_str("\nReturn the corrected full JSON in the same format.");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(summary: &str) -> PlanContent {
        PlanContent {
            summary: summary.into(),
            sections: BTreeMap::from([("diet".to_string(), serde_json::json!({"meals": []}))]),
            disclaimer: None,
        }
    }

    async fn store() -> (PlanStore, tempfile::TempDir) {
        let (db, dir) = crate::db::test_pool().await;
        let clock = Arc::new(crate::clock::FixedClock::new(Utc::now()));
        (PlanStore::new(db, clock), dir)
    }

    const DAY: NaiveDate = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();

    /// Contributes one section, "test". A raw section `{"ok": true}` passes; anything else
    /// has a violation. Always adds the warning "注意".
    struct SectionPlugin;

    #[async_trait::async_trait]
    impl Plugin for SectionPlugin {
        fn manifest(&self) -> crate::plugin::PluginManifest {
            crate::plugin::PluginManifest {
                id: "test",
                name: "Test",
                version: "0.1.0",
                event_types: vec![],
                migrations: vec![],
            }
        }
        fn tools(&self) -> Vec<lifeagent_llm::types::ToolSpec> {
            vec![]
        }
        async fn call_tool(
            &self,
            _ctx: &PluginContext,
            _name: &str,
            _args: Value,
        ) -> Result<crate::plugin::ToolOutput, crate::plugin::PluginError> {
            unreachable!()
        }
        async fn plan_input(
            &self,
            _ctx: &PluginContext,
            _date: NaiveDate,
        ) -> Result<Option<PlanInput>, crate::plugin::PluginError> {
            Ok(Some(PlanInput {
                section: "test".into(),
                brief: "test brief".into(),
                targets: serde_json::json!({"x": 1}),
                output_schema: serde_json::json!({"type": "object"}),
            }))
        }
        async fn finalize_plan_section(
            &self,
            _ctx: &PluginContext,
            _date: NaiveDate,
            raw: Value,
        ) -> Result<PlanSection, crate::plugin::PluginError> {
            let ok = raw.get("ok") == Some(&Value::Bool(true));
            Ok(PlanSection {
                data: raw,
                warnings: vec!["注意".into()],
                violations: if ok { vec![] } else { vec!["not ok".into()] },
            })
        }
    }

    fn planner_reply(summary: &str, sections: Value) -> lifeagent_llm::types::ChatResponse {
        lifeagent_llm::types::ChatResponse {
            content: Some(
                serde_json::json!({"summary": summary, "sections": sections}).to_string(),
            ),
            tool_calls: vec![],
            usage: Default::default(),
            finish_reason: "stop".into(),
        }
    }

    fn service() -> PlanService {
        PlanService::new(vec![Arc::new(SectionPlugin)])
    }

    fn last_user_message(request: &lifeagent_llm::types::ChatRequest) -> String {
        match request.messages.last() {
            Some(Message::User { content }) => content.clone(),
            other => panic!("expected a user message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn generate_retries_once_on_violations_then_stores_version_1() {
        let (ctx, mock, _dir) = crate::test_support::context().await;
        let mut rx = ctx.events.subscribe();
        mock.push_response(planner_reply(
            "first",
            serde_json::json!({"test": {"ok": false}}),
        ));
        mock.push_response(planner_reply(
            "second",
            serde_json::json!({"test": {"ok": true}}),
        ));

        let plan = service()
            .generate(&ctx, DAY, PlanTrigger::Manual)
            .await
            .unwrap();

        assert_eq!(plan.version, 1);
        assert_eq!(plan.content.summary, "second");
        assert_eq!(plan.warnings, vec!["注意"]);
        let requests = mock.received_requests();
        assert_eq!(requests.len(), 2);
        let retry = last_user_message(&requests[1]);
        assert!(retry.contains("## Section \"test\"\n- not ok"));
        // The retry also carries the model's previous output.
        assert!(matches!(
            &requests[1].messages[1],
            Message::Assistant { content: Some(c), .. } if c.contains("first")
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            AppEvent::PlanReady {
                date: DAY,
                version: 1,
                trigger: PlanTrigger::Manual
            }
        ));
    }

    #[tokio::test]
    async fn violations_left_after_the_retry_become_prefixed_warnings() {
        let (ctx, mock, _dir) = crate::test_support::context().await;
        for _ in 0..2 {
            mock.push_response(planner_reply(
                "s",
                serde_json::json!({"test": {"ok": false}}),
            ));
        }

        let plan = service()
            .generate(&ctx, DAY, PlanTrigger::Schedule)
            .await
            .unwrap();

        assert_eq!(plan.warnings, vec!["注意", "未完全满足：not ok"]);
        assert_eq!(mock.received_requests().len(), 2);
    }

    #[tokio::test]
    async fn a_missing_section_is_a_violation() {
        let (ctx, mock, _dir) = crate::test_support::context().await;
        mock.push_response(planner_reply("s", serde_json::json!({})));
        mock.push_response(planner_reply(
            "s",
            serde_json::json!({"test": {"ok": true}}),
        ));

        service()
            .generate(&ctx, DAY, PlanTrigger::Manual)
            .await
            .unwrap();

        let retry = last_user_message(&mock.received_requests()[1]);
        assert!(retry.contains("计划缺少“test”部分"));
    }

    #[tokio::test]
    async fn no_plan_inputs_is_nothing_to_plan() {
        let (ctx, mock, _dir) = crate::test_support::context().await;
        let err = PlanService::new(vec![])
            .generate(&ctx, DAY, PlanTrigger::Manual)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NothingToPlan));
        assert!(mock.received_requests().is_empty());
    }

    #[tokio::test]
    async fn prompt_contains_language_date_and_section_brief() {
        let (ctx, mock, _dir) = crate::test_support::context().await;
        mock.push_response(planner_reply(
            "s",
            serde_json::json!({"test": {"ok": true}}),
        ));

        service()
            .generate(&ctx, DAY, PlanTrigger::Manual)
            .await
            .unwrap();

        let prompt = last_user_message(&mock.received_requests()[0]);
        assert!(prompt.contains("to the user in Simplified Chinese"));
        assert!(prompt.contains("Date: 2026-09-25 (Friday)"));
        assert!(prompt.contains("## Section \"test\"\nTargets: {\"x\":1}\nBrief:\ntest brief"));
        assert!(!prompt.contains("## Current plan"));
    }

    #[tokio::test]
    async fn revise_stores_version_2_with_instruction_and_shows_the_current_plan() {
        let (ctx, mock, _dir) = crate::test_support::context().await;
        mock.push_response(planner_reply(
            "v1",
            serde_json::json!({"test": {"ok": true}}),
        ));
        mock.push_response(planner_reply(
            "v2",
            serde_json::json!({"test": {"ok": true}}),
        ));
        let service = service();
        service
            .generate(&ctx, DAY, PlanTrigger::Manual)
            .await
            .unwrap();

        let revised = service.revise(&ctx, DAY, "明天中午公司聚餐").await.unwrap();

        assert_eq!(revised.version, 2);
        assert_eq!(revised.trigger, PlanTrigger::Revision);
        assert_eq!(revised.instruction.as_deref(), Some("明天中午公司聚餐"));
        let prompt = last_user_message(&mock.received_requests()[1]);
        assert!(prompt.contains("## Current plan (version 1)"));
        assert!(prompt.contains("## User's change request\n明天中午公司聚餐"));
    }

    #[tokio::test]
    async fn revise_without_a_plan_is_not_found() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            service().revise(&ctx, DAY, "少一点").await,
            Err(CoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn medical_notes_add_the_disclaimer() {
        let (ctx, mock, _dir) = crate::test_support::context().await;
        ctx.profile
            .set(&crate::profile::Profile {
                display_name: None,
                sex: crate::profile::Sex::Female,
                birth_date: NaiveDate::from_ymd_opt(1990, 1, 1).unwrap(),
                height_cm: 160.0,
                activity_level: crate::profile::ActivityLevel::Light,
                dietary_pattern: crate::profile::DietaryPattern::Omnivore,
                allergies: vec![],
                dislikes: vec![],
                medical_notes: Some("2 型糖尿病".into()),
                meals_per_day: 3,
            })
            .await
            .unwrap();
        mock.push_response(planner_reply(
            "s",
            serde_json::json!({"test": {"ok": true}}),
        ));

        let plan = service()
            .generate(&ctx, DAY, PlanTrigger::Manual)
            .await
            .unwrap();

        assert_eq!(plan.content.disclaimer.as_deref(), Some(MEDICAL_DISCLAIMER));
    }

    #[tokio::test]
    async fn accept_marks_the_current_version() {
        let (store, _dir) = store().await;
        assert!(matches!(
            store.accept(DAY).await,
            Err(CoreError::NotFound(_))
        ));
        store
            .insert(DAY, PlanTrigger::Manual, None, &content("v1"), &[])
            .await
            .unwrap();
        store
            .insert(DAY, PlanTrigger::Manual, None, &content("v2"), &[])
            .await
            .unwrap();

        store.accept(DAY).await.unwrap();

        let current = store.current(DAY).await.unwrap().unwrap();
        assert_eq!(current.version, 2);
        assert_eq!(current.status, PlanStatus::Accepted);
    }

    #[tokio::test]
    async fn current_is_none_when_no_plan_exists() {
        let (store, _dir) = store().await;
        assert_eq!(store.current(DAY).await.unwrap(), None);
    }

    #[tokio::test]
    async fn insert_then_current_round_trips() {
        let (store, _dir) = store().await;
        let inserted = store
            .insert(
                DAY,
                PlanTrigger::Manual,
                None,
                &content("v1"),
                &["w".to_string()],
            )
            .await
            .unwrap();

        assert_eq!(inserted.version, 1);
        assert_eq!(inserted.status, PlanStatus::Proposed);
        assert_eq!(store.current(DAY).await.unwrap(), Some(inserted));
    }

    #[tokio::test]
    async fn versions_increment_per_date_and_current_is_the_latest() {
        let (store, _dir) = store().await;
        store
            .insert(DAY, PlanTrigger::Schedule, None, &content("v1"), &[])
            .await
            .unwrap();
        let v2 = store
            .insert(
                DAY,
                PlanTrigger::Revision,
                Some("午饭换成沙拉"),
                &content("v2"),
                &[],
            )
            .await
            .unwrap();
        let other_day = store
            .insert(
                DAY.succ_opt().unwrap(),
                PlanTrigger::Manual,
                None,
                &content("x"),
                &[],
            )
            .await
            .unwrap();

        assert_eq!(v2.version, 2);
        assert_eq!(other_day.version, 1);
        let current = store.current(DAY).await.unwrap().unwrap();
        assert_eq!(current, v2);
        assert_eq!(current.instruction.as_deref(), Some("午饭换成沙拉"));
    }
}
