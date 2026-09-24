//! Conversation system-prompt builder, per `docs/PHASE1_SPEC.md` §7.9/§9.1.
//!
//! The "Plans" section is a fixed placeholder until the plan service lands in M5 — there is
//! nothing to report yet (see `docs/DECISIONS.md`).

use std::sync::Arc;

use chrono::{Local, NaiveDate};
use serde_json::Value;

use crate::error::CoreError;
use crate::events::Event;
use crate::goals::{GoalDirection, WeightGoal};
use crate::memory::MemoryNote;
use crate::plugin::{Plugin, PluginContext};
use crate::profile::Profile;
use crate::time::{age_years, logical_date};

const SYSTEM_PROMPT_TEMPLATE: &str = "\
You are LifeAgent, a personal assistant running on the user's computer. You help the user
track daily life and follow plans that move them toward their goals.

Rules:
- Record things the user reports (meals, weight, ...) by calling the matching tool. Do not
  just acknowledge without recording.
- Never invent or compute nutrition numbers yourself. Numbers come from tool results; quote
  them as given.
- If a report is ambiguous in a way that changes the numbers a lot (e.g. \"ate some rice\"),
  log it with a reasonable typical portion and tell the user which portion you assumed.
- When the user corrects a logged entry, delete it with core_delete_entry and log it again.
- Save durable preferences with core_remember; do not save one-off events.
- To change a plan, call core_revise_plan with the user's request as the instruction.
- Be brief and warm. Reply in the language the user writes in (default Simplified Chinese).
- You give lifestyle guidance, not medical advice. If the user describes symptoms or a medical
  condition, suggest consulting a professional.

Current time: {now_local} ({weekday}). Logical date: {logical_date}.

## User profile
{profile_summary}

## Goals
{goals}

## Today's status
{plugin_snippets}

## Plans
{plans}

## Memory notes
{memory_notes}";

/// Target size for the built prompt (§7.9); memory notes are truncated first if exceeded.
const TARGET_TOKENS: usize = 2500;

/// Plan service lands in M5; until then there is nothing to report here.
const PLANS_PLACEHOLDER: &str = "No plan for today or tomorrow yet.";

/// Rough token estimate for CJK-heavy text (§7.9): chars / 3.
fn estimate_tokens(text: &str) -> usize {
    text.chars().count() / 3
}

pub async fn build_system_prompt(
    ctx: &PluginContext,
    plugins: &[Arc<dyn Plugin>],
) -> Result<String, CoreError> {
    let now = ctx.clock.now();
    let now_local = now.with_timezone(&Local);
    let boundary = ctx.settings.day_boundary().await?;
    let today = logical_date(now, boundary);

    let profile = ctx.profile.get().await?;
    let latest_weight = ctx.events.latest("body.weight_logged").await?;
    let profile_summary = render_profile_summary(profile.as_ref(), latest_weight.as_ref(), today);

    let goal = ctx.goals.get_active_weight_goal().await?;
    let goals_text = render_goals(goal.as_ref());

    let mut snippets = Vec::new();
    for plugin in plugins {
        let manifest = plugin.manifest();
        if let Some(snippet) = plugin
            .context_snippet(ctx, today)
            .await
            .map_err(|e| CoreError::Plugin(format!("{}: {e}", manifest.id)))?
        {
            snippets.push(format!("### {}\n{}", manifest.name, snippet));
        }
    }
    let plugin_snippets = if snippets.is_empty() {
        "(no data yet)".to_string()
    } else {
        snippets.join("\n\n")
    };

    let mut notes = ctx.memory.list(50).await?;
    let render = |notes: &[MemoryNote]| -> String {
        render_template(
            now_local,
            today,
            &profile_summary,
            &goals_text,
            &plugin_snippets,
            notes,
        )
    };

    let mut prompt = render(&notes);
    while estimate_tokens(&prompt) > TARGET_TOKENS && !notes.is_empty() {
        // Notes are newest-first; drop the oldest (last) one first.
        notes.pop();
        prompt = render(&notes);
    }

    Ok(prompt)
}

fn render_template(
    now_local: chrono::DateTime<Local>,
    today: NaiveDate,
    profile_summary: &str,
    goals_text: &str,
    plugin_snippets: &str,
    notes: &[MemoryNote],
) -> String {
    let memory_notes = if notes.is_empty() {
        "(none)".to_string()
    } else {
        notes
            .iter()
            .map(|n| format!("- [{}] {}", n.id, n.note))
            .collect::<Vec<_>>()
            .join("\n")
    };

    SYSTEM_PROMPT_TEMPLATE
        .replace(
            "{now_local}",
            &now_local.format("%Y-%m-%d %H:%M").to_string(),
        )
        .replace("{weekday}", &now_local.format("%A").to_string())
        .replace("{logical_date}", &today.to_string())
        .replace("{profile_summary}", profile_summary)
        .replace("{goals}", goals_text)
        .replace("{plugin_snippets}", plugin_snippets)
        .replace("{plans}", PLANS_PLACEHOLDER)
        .replace("{memory_notes}", &memory_notes)
}

fn render_profile_summary(
    profile: Option<&Profile>,
    latest_weight: Option<&Event>,
    today: NaiveDate,
) -> String {
    let Some(profile) = profile else {
        return "Profile not set yet.".to_string();
    };

    let age = age_years(profile.birth_date, today);
    let sex = match profile.sex {
        crate::profile::Sex::Male => "male",
        crate::profile::Sex::Female => "female",
    };
    let activity = format!("{:?}", profile.activity_level).to_lowercase();
    let dietary = format!("{:?}", profile.dietary_pattern).to_lowercase();
    let weight_part = match latest_weight {
        Some(event) => {
            let kg = event
                .payload
                .get("kg")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            format!("latest weight {kg} kg (recorded {})", event.logical_date)
        }
        None => "no weight recorded yet".to_string(),
    };
    let allergies = if profile.allergies.is_empty() {
        "none".to_string()
    } else {
        profile.allergies.join(", ")
    };
    let dislikes = if profile.dislikes.is_empty() {
        "none".to_string()
    } else {
        profile.dislikes.join(", ")
    };

    let mut summary = format!(
        "Age {age}, {sex}, height {:.0} cm; {weight_part}; activity level: {activity}; \
         dietary pattern: {dietary}; allergies: {allergies}; dislikes: {dislikes}.",
        profile.height_cm
    );
    if let Some(name) = &profile.display_name {
        summary = format!("Name: {name}. {summary}");
    }
    if let Some(notes) = &profile.medical_notes {
        if !notes.trim().is_empty() {
            summary.push_str(&format!(" Medical notes: {notes}"));
        }
    }
    summary
}

fn render_goals(goal: Option<&WeightGoal>) -> String {
    let Some(goal) = goal else {
        return "No active goals.".to_string();
    };
    match goal.direction {
        GoalDirection::Maintain => "Weight goal: maintain current weight.".to_string(),
        GoalDirection::Lose | GoalDirection::Gain => {
            let direction = if goal.direction == GoalDirection::Lose {
                "lose"
            } else {
                "gain"
            };
            let target = goal
                .target_weight_kg
                .map(|t| format!(", target {t} kg"))
                .unwrap_or_default();
            format!(
                "Weight goal: {direction} weight at {} kg/week{target}.",
                goal.rate_kg_per_week
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventSource;
    use crate::plugin::{PluginError, PluginManifest, ToolOutput};
    use crate::profile::{ActivityLevel, DietaryPattern, Sex};
    use async_trait::async_trait;
    use lifeagent_llm::types::ToolSpec;
    use serde_json::json;

    struct SnippetPlugin;

    #[async_trait]
    impl Plugin for SnippetPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                id: "snippet",
                name: "Snippet Plugin",
                version: "0.1.0",
                event_types: vec![],
                migrations: vec![],
            }
        }
        fn tools(&self) -> Vec<ToolSpec> {
            vec![]
        }
        async fn call_tool(
            &self,
            _ctx: &PluginContext,
            _name: &str,
            _args: Value,
        ) -> Result<ToolOutput, PluginError> {
            unreachable!()
        }
        async fn context_snippet(
            &self,
            _ctx: &PluginContext,
            _date: NaiveDate,
        ) -> Result<Option<String>, PluginError> {
            Ok(Some("today's snippet text".to_string()))
        }
    }

    fn sample_profile() -> Profile {
        Profile {
            display_name: Some("测试".into()),
            sex: Sex::Female,
            birth_date: NaiveDate::from_ymd_opt(1996, 5, 1).unwrap(),
            height_cm: 165.0,
            activity_level: ActivityLevel::Moderate,
            dietary_pattern: DietaryPattern::Omnivore,
            allergies: vec!["虾".into()],
            dislikes: vec!["香菜".into()],
            medical_notes: None,
            meals_per_day: 3,
        }
    }

    #[tokio::test]
    async fn includes_fixed_rules_and_defaults_when_nothing_is_set() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        let prompt = build_system_prompt(&ctx, &[]).await.unwrap();

        assert!(prompt.contains("You are LifeAgent"));
        assert!(prompt.contains("Profile not set yet."));
        assert!(prompt.contains("No active goals."));
        assert!(prompt.contains("(no data yet)"));
        assert!(prompt.contains(PLANS_PLACEHOLDER));
        assert!(prompt.contains("(none)"));
    }

    #[tokio::test]
    async fn includes_profile_weight_and_goal_when_set() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        ctx.profile.set(&sample_profile()).await.unwrap();
        ctx.events
            .append(
                "body.weight_logged",
                ctx.clock.now(),
                json!({"kg": 62.5}),
                EventSource::Chat,
            )
            .await
            .unwrap();
        ctx.goals
            .save_weight_goal(&WeightGoal {
                direction: GoalDirection::Lose,
                target_weight_kg: Some(58.0),
                rate_kg_per_week: 0.5,
            })
            .await
            .unwrap();

        let prompt = build_system_prompt(&ctx, &[]).await.unwrap();

        assert!(prompt.contains("62.5 kg"));
        assert!(prompt.contains("female"));
        assert!(prompt.contains("allergies: 虾"));
        assert!(prompt.contains("Weight goal: lose weight at 0.5 kg/week, target 58 kg."));
    }

    #[tokio::test]
    async fn includes_each_plugins_context_snippet_under_its_name() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(SnippetPlugin)];

        let prompt = build_system_prompt(&ctx, &plugins).await.unwrap();

        assert!(prompt.contains("### Snippet Plugin"));
        assert!(prompt.contains("today's snippet text"));
    }

    #[tokio::test]
    async fn includes_memory_notes_with_their_ids() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        let note = ctx.memory.add("不吃香菜").await.unwrap();

        let prompt = build_system_prompt(&ctx, &[]).await.unwrap();
        assert!(prompt.contains(&format!("[{}] 不吃香菜", note.id)));
    }

    #[tokio::test]
    async fn truncates_oldest_memory_notes_first_when_over_budget() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        // 40 notes near the 300-char cap: comfortably over the 2,500-token budget on their
        // own, forcing truncation.
        let mut ids = Vec::new();
        for i in 0..40 {
            let note = ctx.memory.add(&"记".repeat(290)).await.unwrap();
            ids.push((i, note.id));
        }

        let prompt = build_system_prompt(&ctx, &[]).await.unwrap();

        assert!(estimate_tokens(&prompt) <= TARGET_TOKENS);
        // The newest note (added last) must survive; the oldest must be the first dropped.
        let (_, newest_id) = ids.last().unwrap();
        let (_, oldest_id) = ids.first().unwrap();
        assert!(prompt.contains(newest_id.as_str()));
        assert!(!prompt.contains(oldest_id.as_str()));
    }
}
