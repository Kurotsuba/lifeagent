//! `CoreApp`'s proactive follow-ups (`crate::follow_ups`): the scheduler's check, the list the
//! app shows, and the user's answers.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Local, NaiveDate, NaiveDateTime};
use serde_json::json;

use super::CoreApp;
use crate::audit::{self, Actor, RunEnd, RunTrigger, StepKind};
use crate::error::CoreError;
use crate::events::AppEvent;
use crate::follow_ups::{
    FollowUp, FollowUpCandidate, FollowUpStatus, ANSWER_ADJUST_TOMORROW, ANSWER_DISMISS,
    ANSWER_REPLY, MAX_PER_DAY,
};
use crate::plan::PlanTrigger;
use crate::plugin::{Plugin, PluginError, ToolSummary};

/// What the enabled plugins' checks found. A plugin whose check failed is listed in `failed`,
/// so its open follow-ups are kept rather than cleared.
struct Checked {
    candidates: Vec<FollowUpCandidate>,
    enabled: HashSet<&'static str>,
    failed: HashSet<&'static str>,
}

impl CoreApp {
    /// The scheduler's follow-up check: raises what the enabled plugins' checks found and that
    /// wasn't raised today yet, up to [`MAX_PER_DAY`], in one activity-log run by the scheduler.
    /// Also clears open ones whose reason went away. Does nothing while the switch is off or
    /// before onboarding. Returns the follow-ups it raised.
    pub async fn check_follow_ups(&self) -> Result<Vec<FollowUp>, CoreError> {
        let settings = &self.0.settings;
        if !settings.follow_ups_enabled().await? || !settings.onboarding_complete().await? {
            return Ok(Vec::new());
        }
        let (today, now_local) = self.follow_up_now().await?;
        let raised = self.0.follow_ups.on(today).await?;
        let checked = self.check_plugins(today, now_local, &raised).await?;
        let mut changed = self.settle(&raised, &checked).await?;

        let known: HashSet<&str> = raised.iter().map(|f| f.key.as_str()).collect();
        let room = MAX_PER_DAY.saturating_sub(raised.len());
        let new: Vec<&FollowUpCandidate> = checked
            .candidates
            .iter()
            .filter(|c| !known.contains(c.key.as_str()))
            .collect();
        if new.len() > room {
            tracing::info!(
                skipped = new.len() - room,
                limit = MAX_PER_DAY,
                "follow-ups: daily limit reached"
            );
        }
        let mut out = Vec::new();
        if room > 0 && !new.is_empty() {
            let run = self
                .0
                .audit
                .start(RunTrigger::FollowUp, Actor::Scheduler, None, today)
                .await?;
            let result = audit::scope(run.clone(), async {
                for candidate in new.into_iter().take(room) {
                    let Some(follow_up) = self.0.follow_ups.raise(today, candidate).await? else {
                        continue;
                    };
                    audit::record(
                        StepKind::FollowUpRaised,
                        json!({
                            "follow_up_id": follow_up.id,
                            "key": follow_up.key,
                            "kind": follow_up.kind,
                            "reason": follow_up.reason,
                            "params": follow_up.params,
                        }),
                    )
                    .await;
                    tracing::info!(kind = %follow_up.kind, "follow-up raised");
                    out.push(follow_up);
                }
                Ok::<_, CoreError>(())
            })
            .await;
            match &result {
                Ok(()) => run.finish(RunEnd::Ok).await,
                Err(e) => run.finish(RunEnd::Failed { code: e.code() }).await,
            }
            result?;
            changed |= !out.is_empty();
        }
        if changed {
            self.0.events.publish(AppEvent::FollowUpsChanged);
        }
        Ok(out)
    }

    /// Today's open follow-ups, oldest first, after clearing those whose reason went away (a
    /// meal logged in chat meanwhile). Never raises new ones: only the scheduler does. Empty
    /// while the switch is off.
    pub async fn follow_ups(&self) -> Result<Vec<FollowUp>, CoreError> {
        if !self.0.settings.follow_ups_enabled().await? {
            return Ok(Vec::new());
        }
        let (today, now_local) = self.follow_up_now().await?;
        let raised = self.0.follow_ups.on(today).await?;
        if !raised.iter().any(|f| f.status == FollowUpStatus::Open) {
            return Ok(Vec::new());
        }
        let checked = self.check_plugins(today, now_local, &raised).await?;
        if self.settle(&raised, &checked).await? {
            self.0.events.publish(AppEvent::FollowUpsChanged);
        }
        Ok(self
            .0
            .follow_ups
            .on(today)
            .await?
            .into_iter()
            .filter(|f| f.status == FollowUpStatus::Open)
            .collect())
    }

    /// Carries out the user's answer to an open follow-up, in one activity-log run by the user:
    /// `core_dismiss` closes it, `core_adjust_tomorrow` has the planner revise tomorrow's plan
    /// (or generate it) with the follow-up's instruction, and a plugin's own answer runs in that
    /// plugin. `ActionNotPending` if it was closed meanwhile.
    pub async fn answer_follow_up(&self, id: &str, answer: &str) -> Result<FollowUp, CoreError> {
        let result = self
            .with_run(
                RunTrigger::FollowUpAnswered,
                Actor::User,
                None,
                self.answer_in_run(id, answer),
            )
            .await;
        self.0.events.publish(AppEvent::FollowUpsChanged);
        result
    }

    async fn answer_in_run(&self, id: &str, answer: &str) -> Result<FollowUp, CoreError> {
        let follow_up = self
            .0
            .follow_ups
            .get(id)
            .await?
            .ok_or_else(|| CoreError::NotFound(format!("follow-up {id}")))?;
        // Yesterday's are over: logging "as planned" now would land on the wrong day.
        if follow_up.status != FollowUpStatus::Open
            || follow_up.logical_date != self.logical_today().await?
        {
            return Err(CoreError::ActionNotPending);
        }
        if answer == ANSWER_REPLY || !follow_up.answers.iter().any(|a| a == answer) {
            return Err(CoreError::Validation(format!(
                "{answer:?} is not an answer to follow-up {id}"
            )));
        }
        let (status, result) = match answer {
            ANSWER_DISMISS => (FollowUpStatus::Dismissed, None),
            ANSWER_ADJUST_TOMORROW => {
                let summary = self.adjust_tomorrow(&follow_up).await?;
                (FollowUpStatus::Answered, Some(summary))
            }
            own => {
                let plugin = self
                    .enabled_plugins()
                    .await?
                    .into_iter()
                    .find(|p| {
                        let id = p.manifest().id;
                        id == follow_up.plugin_id() && own.starts_with(&format!("{id}_"))
                    })
                    .ok_or_else(|| {
                        CoreError::NotFound(format!("plugin for follow-up answer {own:?}"))
                    })?;
                let summary = plugin
                    .answer_follow_up(&self.plugin_context(), &follow_up, own)
                    .await
                    .map_err(|e| match e {
                        PluginError::NotFound(m) => CoreError::NotFound(m),
                        PluginError::InvalidArgs(m) => CoreError::Validation(m),
                        PluginError::Llm(e) => CoreError::Llm(e),
                        other => CoreError::Plugin(other.to_string()),
                    })?;
                (FollowUpStatus::Answered, Some(summary))
            }
        };
        let closed = self
            .0
            .follow_ups
            .close(id, status, Some(answer), result.as_ref())
            .await?;
        let mut detail = json!({
            "follow_up_id": closed.id,
            "key": closed.key,
            "kind": closed.kind,
            "answer": answer,
        });
        if let Some(result) = &closed.result {
            detail["result"] = result.0.clone();
        }
        audit::record(StepKind::FollowUpAnswered, detail).await;
        tracing::info!(kind = %closed.kind, answer, "follow-up answered");
        Ok(closed)
    }

    /// Revises tomorrow's plan with the follow-up's instruction, or generates it with that as
    /// the request when there is none yet. The planner's model calls and validation join the
    /// current run.
    async fn adjust_tomorrow(&self, follow_up: &FollowUp) -> Result<ToolSummary, CoreError> {
        let instruction = follow_up.instruction.as_deref().ok_or_else(|| {
            CoreError::Validation(format!("follow-up {} has no instruction", follow_up.id))
        })?;
        let tomorrow = follow_up
            .logical_date
            .succ_opt()
            .ok_or_else(|| CoreError::Validation("no day after the follow-up's".into()))?;
        let plan = if self.get_plan(tomorrow).await?.is_some() {
            self.revise_plan(tomorrow, instruction).await?
        } else {
            self.generate_plan(tomorrow, PlanTrigger::Manual, Some(instruction))
                .await?
        };
        Ok(ToolSummary::new("core_plan_adjusted")
            .with("date", tomorrow)
            .with("version", plan.version))
    }

    async fn follow_up_now(&self) -> Result<(NaiveDate, NaiveDateTime), CoreError> {
        let today = self.logical_today().await?;
        Ok((
            today,
            self.0.clock.now().with_timezone(&Local).naive_local(),
        ))
    }

    async fn check_plugins(
        &self,
        today: NaiveDate,
        now_local: NaiveDateTime,
        raised: &[FollowUp],
    ) -> Result<Checked, CoreError> {
        let ctx = self.plugin_context();
        let plugins: Vec<Arc<dyn Plugin>> = self.enabled_plugins().await?;
        let mut checked = Checked {
            candidates: Vec::new(),
            enabled: HashSet::new(),
            failed: HashSet::new(),
        };
        for plugin in plugins {
            let id = plugin.manifest().id;
            checked.enabled.insert(id);
            match plugin.follow_ups(&ctx, today, now_local, raised).await {
                Ok(found) => checked.candidates.extend(
                    // A plugin may only raise its own keys.
                    found
                        .into_iter()
                        .filter(|c| c.key.split('_').next() == Some(id)),
                ),
                Err(e) => {
                    tracing::warn!(plugin = id, error = %e, "follow-up check failed");
                    checked.failed.insert(id);
                }
            }
        }
        Ok(checked)
    }

    /// Clears open follow-ups whose plugin is now off or whose candidate is gone, and refreshes
    /// the params of the rest. Whether anything was cleared.
    async fn settle(&self, raised: &[FollowUp], checked: &Checked) -> Result<bool, CoreError> {
        let mut cleared = false;
        for follow_up in raised.iter().filter(|f| f.status == FollowUpStatus::Open) {
            let plugin = follow_up.plugin_id();
            if checked.failed.contains(plugin) {
                continue;
            }
            let current = checked
                .candidates
                .iter()
                .find(|c| c.key == follow_up.key)
                .filter(|_| checked.enabled.contains(plugin));
            match current {
                Some(c)
                    if c.params != follow_up.params || c.instruction != follow_up.instruction =>
                {
                    self.0.follow_ups.refresh(&follow_up.id, c).await?;
                }
                Some(_) => {}
                None => {
                    match self
                        .0
                        .follow_ups
                        .close(&follow_up.id, FollowUpStatus::Cleared, None, None)
                        .await
                    {
                        Ok(_) => cleared = true,
                        // Answered meanwhile.
                        Err(CoreError::ActionNotPending) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok(cleared)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::Value;

    use super::*;
    use crate::app::CoreAppBuilder;
    use crate::audit::RunFilter;
    use crate::clock::FixedClock;
    use crate::follow_ups::FollowUpCandidate;
    use crate::plugin::{PluginContext, PluginManifest, ToolOutput};
    use crate::secrets::MemorySecretStore;

    /// Raises `tst_thing` while `holds` is set; its own answer `tst_do` succeeds.
    #[derive(Default)]
    struct CheckPlugin {
        holds: AtomicBool,
    }

    #[async_trait]
    impl Plugin for CheckPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                id: "tst",
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
        ) -> Result<ToolOutput, PluginError> {
            unreachable!()
        }
        async fn follow_ups(
            &self,
            _ctx: &PluginContext,
            _date: NaiveDate,
            _now_local: NaiveDateTime,
            _raised: &[FollowUp],
        ) -> Result<Vec<FollowUpCandidate>, PluginError> {
            if !self.holds.load(Ordering::SeqCst) {
                return Ok(Vec::new());
            }
            let candidate = |key: &str| FollowUpCandidate {
                key: key.into(),
                kind: "tst_thing".into(),
                params: json!({}),
                answers: vec!["tst_do".into(), ANSWER_DISMISS.into()],
                reason: "it holds".into(),
                instruction: None,
            };
            // Six candidates, one not its own: more than the daily limit.
            Ok(["tst_a", "tst_b", "tst_c", "tst_d", "tst_e", "other_x"]
                .into_iter()
                .map(candidate)
                .collect())
        }
        async fn answer_follow_up(
            &self,
            _ctx: &PluginContext,
            _follow_up: &FollowUp,
            answer: &str,
        ) -> Result<ToolSummary, PluginError> {
            Ok(ToolSummary::new("tst_done").with("answer", answer))
        }
    }

    async fn app(plugin: Arc<CheckPlugin>) -> (CoreApp, crate::db::TestDb) {
        let dir = crate::db::TestDb::new().await;
        let app = CoreAppBuilder::new(dir.config.clone())
            .clock(Arc::new(FixedClock::new(Utc::now())))
            .secrets(Arc::new(MemorySecretStore::new()))
            .plugin(plugin)
            .build()
            .await
            .unwrap();
        app.settings()
            .set_onboarding_complete(true, Utc::now())
            .await
            .unwrap();
        (app, dir)
    }

    #[tokio::test]
    async fn raises_up_to_the_daily_limit_once_and_records_a_scheduler_run() {
        let plugin = Arc::new(CheckPlugin::default());
        let (app, _dir) = app(plugin.clone()).await;
        assert!(app.check_follow_ups().await.unwrap().is_empty());

        plugin.holds.store(true, Ordering::SeqCst);
        let raised = app.check_follow_ups().await.unwrap();
        let keys: Vec<&str> = raised.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["tst_a", "tst_b", "tst_c", "tst_d"]);
        // Nothing new on the next tick.
        assert!(app.check_follow_ups().await.unwrap().is_empty());
        assert_eq!(app.follow_ups().await.unwrap().len(), MAX_PER_DAY);

        let runs = app
            .audit()
            .list(&RunFilter::default(), 10)
            .await
            .unwrap()
            .runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].trigger, RunTrigger::FollowUp);
        assert_eq!(runs[0].actor, Actor::Scheduler);
        let steps = app.audit().steps(&runs[0].id).await.unwrap();
        assert_eq!(steps.len(), MAX_PER_DAY);
        assert_eq!(steps[0].kind, StepKind::FollowUpRaised);
        assert_eq!(steps[0].detail["reason"], "it holds");
    }

    #[tokio::test]
    async fn answers_run_once_and_cleared_ones_disappear() {
        let plugin = Arc::new(CheckPlugin {
            holds: AtomicBool::new(true),
        });
        let (app, _dir) = app(plugin.clone()).await;
        let raised = app.check_follow_ups().await.unwrap();

        let answered = app.answer_follow_up(&raised[0].id, "tst_do").await.unwrap();
        assert_eq!(answered.status, FollowUpStatus::Answered);
        assert_eq!(answered.result.unwrap().kind(), "tst_done");
        assert!(matches!(
            app.answer_follow_up(&raised[0].id, ANSWER_DISMISS).await,
            Err(CoreError::ActionNotPending)
        ));
        // Not offered, and the chat answer is the UI's own.
        assert!(app
            .answer_follow_up(&raised[1].id, "tst_other")
            .await
            .is_err());
        assert!(app
            .answer_follow_up(&raised[1].id, ANSWER_REPLY)
            .await
            .is_err());
        let dismissed = app
            .answer_follow_up(&raised[1].id, ANSWER_DISMISS)
            .await
            .unwrap();
        assert_eq!(dismissed.status, FollowUpStatus::Dismissed);

        let runs = app
            .audit()
            .list(&RunFilter::default(), 10)
            .await
            .unwrap()
            .runs;
        assert_eq!(runs[0].trigger, RunTrigger::FollowUpAnswered);
        assert_eq!(runs[0].actor, Actor::User);

        // The reason went away: the open ones are cleared, the answered ones stay answered.
        plugin.holds.store(false, Ordering::SeqCst);
        assert!(app.follow_ups().await.unwrap().is_empty());
        let today = app.logical_today().await.unwrap();
        let statuses: Vec<FollowUpStatus> = app
            .0
            .follow_ups
            .on(today)
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.status)
            .collect();
        assert_eq!(
            statuses,
            [
                FollowUpStatus::Answered,
                FollowUpStatus::Dismissed,
                FollowUpStatus::Cleared,
                FollowUpStatus::Cleared
            ]
        );
    }

    #[tokio::test]
    async fn the_switch_turns_follow_ups_off() {
        let plugin = Arc::new(CheckPlugin {
            holds: AtomicBool::new(true),
        });
        let (app, _dir) = app(plugin).await;
        app.settings()
            .set_follow_ups_enabled(false, Utc::now())
            .await
            .unwrap();
        assert!(app.check_follow_ups().await.unwrap().is_empty());
        assert!(app.follow_ups().await.unwrap().is_empty());
    }
}
