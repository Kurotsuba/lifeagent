//! Scheduler (§7.13), per `docs/PHASE1_SPEC.md` as amended by B3 in `docs/DECISIONS.md`.
//!
//! The daily SQLite backup (§12, B4) was removed with the move to Postgres: Cloud SQL's
//! automated backups replace it (`docs/HACKATHON.md` §1).

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, Utc};

use crate::app::CoreApp;
use crate::error::CoreError;
use crate::events::AppEvent;
use crate::plan::PlanTrigger;

/// Time between ticks (§7.13). The first tick runs immediately (catch-up).
pub const TICK_INTERVAL: Duration = Duration::from_secs(60);
/// A failed scheduled generation is retried after this long (§7.13).
const RETRY_AFTER_MINUTES: i64 = 15;
/// At most this many scheduled generation attempts per target date (§7.13).
const MAX_ATTEMPTS: u32 = 3;

/// Whether `plan_time` has passed on logical day `today` (§7.13). `plan_time` within logical
/// day `today` falls on calendar date `today` because settings validation enforces
/// `plan_time ≥ day_boundary` (§7.2). Compare against `today`, never the calendar date of
/// `local_now`: between midnight and the boundary those differ (B3).
pub fn is_due(local_now: NaiveDateTime, today: NaiveDate, plan_time: NaiveTime) -> bool {
    local_now >= today.and_time(plan_time)
}

struct Attempts {
    count: u32,
    next_at: DateTime<Utc>,
}

/// Plan generation state across ticks. Attempt counts are in memory only (§7.13).
pub struct Scheduler {
    app: CoreApp,
    attempts: HashMap<NaiveDate, Attempts>,
}

impl Scheduler {
    pub fn new(app: CoreApp) -> Self {
        Self {
            app,
            attempts: HashMap::new(),
        }
    }

    /// One tick: generate tomorrow's plan if due. Errors
    /// are logged, never propagated — the scheduler must keep ticking.
    pub async fn tick(&mut self) {
        let today = match self.app.logical_today().await {
            Ok(today) => today,
            Err(e) => {
                tracing::warn!(error = %e, "scheduler: could not read the logical date");
                return;
            }
        };
        if let Err(e) = self.plan_if_due(today).await {
            tracing::warn!(error = %e, "scheduler: plan check failed");
        }
    }

    async fn plan_if_due(&mut self, today: NaiveDate) -> Result<(), CoreError> {
        let settings = self.app.settings();
        if !settings.onboarding_complete().await? || !self.app.models_configured() {
            return Ok(());
        }
        let now = self.app.clock().now();
        let plan_time = settings.plan_time().await?;
        if !is_due(now.with_timezone(&Local).naive_local(), today, plan_time) {
            return Ok(());
        }
        let Some(tomorrow) = today.succ_opt() else {
            return Ok(());
        };
        // A generation already under way (most likely tomorrow's, from the UI or chat) is
        // waited out rather than queued behind: the next tick sees its plan.
        if self.app.plan_generating() || self.app.get_plan(tomorrow).await?.is_some() {
            return Ok(());
        }
        if let Some(attempts) = self.attempts.get(&tomorrow) {
            if attempts.count >= MAX_ATTEMPTS || now < attempts.next_at {
                return Ok(());
            }
        }

        match self
            .app
            .generate_plan(tomorrow, PlanTrigger::Schedule, None)
            .await
        {
            Ok(_) => {
                self.attempts.remove(&tomorrow);
            }
            Err(e) => {
                let attempts = self.attempts.entry(tomorrow).or_insert(Attempts {
                    count: 0,
                    next_at: now,
                });
                attempts.count += 1;
                attempts.next_at = now + chrono::Duration::minutes(RETRY_AFTER_MINUTES);
                tracing::warn!(date = %tomorrow, attempt = attempts.count, error = %e,
                    "scheduled plan generation failed");
                self.app.events().publish(AppEvent::PlanFailed {
                    date: tomorrow,
                    code: e.code(),
                    error: e.to_string(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, d).unwrap()
    }

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        date(d).and_hms_opt(h, m, 0).unwrap()
    }

    fn hm(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn due_once_plan_time_passes_on_the_logical_day() {
        // plan_time 21:00, boundary 04:00; logical today the 23rd.
        assert!(!is_due(at(23, 20, 59), date(23), hm(21, 0)));
        assert!(is_due(at(23, 21, 0), date(23), hm(21, 0)));
        assert!(is_due(at(23, 23, 30), date(23), hm(21, 0)));
    }

    #[test]
    fn relaunch_after_midnight_is_still_due_for_the_logical_day() {
        // B3: 01:00 on the 24th is still logical day 23 (before the 04:00 boundary), and
        // 21:00 on the 23rd has passed. The old "today's calendar date" rule compared against
        // the 24th at 21:00 and never fired.
        assert!(is_due(at(24, 1, 0), date(23), hm(21, 0)));
    }

    #[test]
    fn plan_time_equal_to_the_boundary_is_due_from_the_boundary() {
        // plan_time 04:00 with a 04:00 boundary: due as soon as the logical day starts.
        assert!(is_due(at(24, 4, 0), date(24), hm(4, 0)));
        assert!(!is_due(at(23, 23, 0), date(24), hm(4, 0)));
    }

    #[test]
    fn early_plan_time_with_a_midnight_boundary() {
        // Boundary 00:00, plan_time 06:00: logical day == calendar day.
        assert!(!is_due(at(24, 5, 59), date(24), hm(6, 0)));
        assert!(is_due(at(24, 6, 0), date(24), hm(6, 0)));
    }
}
