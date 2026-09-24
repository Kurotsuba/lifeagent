//! Scheduler (§7.13) and daily DB backup (§12), per `docs/PHASE1_SPEC.md` as amended by B3
//! and B4 in `docs/DECISIONS.md`.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use sqlx::SqlitePool;

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
/// Dated backups kept (§12).
const BACKUPS_KEPT: usize = 7;

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

/// Plan generation and backup state across ticks. Attempt counts are in memory only (§7.13).
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

    /// One tick: take the daily backup if due, then generate tomorrow's plan if due. Errors
    /// are logged, never propagated — the scheduler must keep ticking.
    pub async fn tick(&mut self) {
        let today = match self.app.logical_today().await {
            Ok(today) => today,
            Err(e) => {
                tracing::warn!(error = %e, "scheduler: could not read the logical date");
                return;
            }
        };
        if let Err(e) = backup_if_due(self.app.db(), self.app.data_dir(), today).await {
            tracing::warn!(error = %e, "daily backup failed; will retry next tick");
        }
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
        if self.app.get_plan(tomorrow).await?.is_some() {
            return Ok(());
        }
        if let Some(attempts) = self.attempts.get(&tomorrow) {
            if attempts.count >= MAX_ATTEMPTS || now < attempts.next_at {
                return Ok(());
            }
        }

        match self
            .app
            .generate_plan(tomorrow, PlanTrigger::Schedule)
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
                    error: e.to_string(),
                });
            }
        }
        Ok(())
    }
}

/// Takes today's backup unless it already exists (§12, B4). Returns whether one was taken.
pub async fn backup_if_due(
    db: &SqlitePool,
    data_dir: &Path,
    today: NaiveDate,
) -> Result<bool, CoreError> {
    let dir = data_dir.join("backups");
    let file = dir.join(format!("lifeagent-{today}.db"));
    if tokio::fs::try_exists(&file).await? {
        return Ok(false);
    }
    tokio::fs::create_dir_all(&dir).await?;

    // VACUUM INTO refuses to overwrite, so a partial file left by a crash must never sit at
    // the final name: write to .tmp, then rename.
    let tmp = dir.join(format!("lifeagent-{today}.db.tmp"));
    match tokio::fs::remove_file(&tmp).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let tmp_path = tmp
        .to_str()
        .ok_or_else(|| CoreError::Validation(format!("backup path is not UTF-8: {tmp:?}")))?;
    // Unlike a file copy, VACUUM INTO includes commits still in the -wal file.
    sqlx::query("VACUUM INTO ?")
        .bind(tmp_path)
        .execute(db)
        .await?;
    tokio::fs::rename(&tmp, &file).await?;

    prune_backups(&dir).await?;
    Ok(true)
}

/// Deletes all but the newest `BACKUPS_KEPT` dated backups.
async fn prune_backups(dir: &Path) -> Result<(), CoreError> {
    let mut dated = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let date = name
            .to_str()
            .and_then(|n| n.strip_prefix("lifeagent-"))
            .and_then(|n| n.strip_suffix(".db"))
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
        if let Some(date) = date {
            dated.push((date, entry.path()));
        }
    }
    dated.sort_by_key(|(date, _)| std::cmp::Reverse(*date));
    for (_, path) in dated.into_iter().skip(BACKUPS_KEPT) {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
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

    #[tokio::test]
    async fn backup_contains_a_row_still_in_the_wal() {
        let (db, dir) = crate::db::test_pool().await;
        sqlx::query("INSERT INTO settings (key, value_json, updated_at) VALUES ('k', '42', 'now')")
            .execute(&db)
            .await
            .unwrap();
        // Small writes stay in the -wal file until a checkpoint; a plain file copy would miss
        // this row.
        assert!(dir.path().join("lifeagent.db-wal").exists());

        assert!(backup_if_due(&db, dir.path(), date(24)).await.unwrap());

        let backup = dir.path().join("backups/lifeagent-2026-09-24.db");
        let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(&backup);
        let backup_db = SqlitePool::connect_with(opts).await.unwrap();
        let value: String = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = 'k'")
            .fetch_one(&backup_db)
            .await
            .unwrap();
        assert_eq!(value, "42");
    }

    #[tokio::test]
    async fn one_backup_per_logical_day() {
        let (db, dir) = crate::db::test_pool().await;
        assert!(backup_if_due(&db, dir.path(), date(24)).await.unwrap());
        assert!(!backup_if_due(&db, dir.path(), date(24)).await.unwrap());
        assert!(backup_if_due(&db, dir.path(), date(25)).await.unwrap());
    }

    #[tokio::test]
    async fn only_the_newest_7_backups_are_kept() {
        let (db, dir) = crate::db::test_pool().await;
        for d in 1..=9 {
            backup_if_due(&db, dir.path(), date(d)).await.unwrap();
        }
        let mut names: Vec<String> = std::fs::read_dir(dir.path().join("backups"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names.len(), 7);
        assert_eq!(names[0], "lifeagent-2026-09-03.db");
        assert_eq!(names[6], "lifeagent-2026-09-09.db");
    }

    #[tokio::test]
    async fn a_leftover_tmp_file_does_not_block_the_backup() {
        let (db, dir) = crate::db::test_pool().await;
        let backups = dir.path().join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(backups.join("lifeagent-2026-09-24.db.tmp"), b"partial").unwrap();

        assert!(backup_if_due(&db, dir.path(), date(24)).await.unwrap());
        assert!(backups.join("lifeagent-2026-09-24.db").exists());
        assert!(!backups.join("lifeagent-2026-09-24.db.tmp").exists());
    }
}
