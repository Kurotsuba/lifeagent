//! `WeightGoal` and `GoalsService`, per `docs/PHASE1_SPEC.md` §7.4.
//!
//! Only one active weight goal at a time (§16: "Single active weight goal"). Saving a new
//! one deactivates the previous one rather than deleting it, so history survives.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::{CoreError, ValidationRule};

const DOMAIN: &str = "body";
const KIND: &str = "weight";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalDirection {
    Lose,
    Maintain,
    Gain,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightGoal {
    pub direction: GoalDirection,
    pub target_weight_kg: Option<f64>,
    /// Ignored for `Maintain`; not validated in that case either (§7.4).
    pub rate_kg_per_week: f64,
}

/// Validates a goal against the bounds in §7.4 (rate 0–1.0 kg/week, weight 30–300 kg). The
/// rate is not checked when the direction is `Maintain`, since it is unused there.
pub fn validate(goal: &WeightGoal) -> Result<(), CoreError> {
    if goal.direction != GoalDirection::Maintain && !(0.0..=1.0).contains(&goal.rate_kg_per_week) {
        return Err(CoreError::InvalidInput(
            ValidationRule::GoalRateOutOfRange {
                got: goal.rate_kg_per_week,
            },
        ));
    }

    if let Some(target) = goal.target_weight_kg {
        if !(30.0..=300.0).contains(&target) {
            return Err(CoreError::InvalidInput(
                ValidationRule::GoalTargetOutOfRange { got: target },
            ));
        }
    }

    Ok(())
}

/// Applies §7.4's default target: a Lose/Gain goal saved without `target_weight_kg` gets the
/// latest weight ∓ 1 kg, clamped to 30–300 kg. An explicit target, a Maintain goal, or no
/// logged weight leaves the goal unchanged.
pub fn with_default_target(goal: &WeightGoal, latest_weight_kg: Option<f64>) -> WeightGoal {
    let mut goal = goal.clone();
    if goal.target_weight_kg.is_none() {
        let delta = match goal.direction {
            GoalDirection::Lose => Some(-1.0),
            GoalDirection::Gain => Some(1.0),
            GoalDirection::Maintain => None,
        };
        if let (Some(delta), Some(kg)) = (delta, latest_weight_kg) {
            goal.target_weight_kg = Some((kg + delta).clamp(30.0, 300.0));
        }
    }
    goal
}

#[derive(Clone)]
pub struct GoalsService {
    db: PgPool,
    clock: Arc<dyn Clock>,
}

impl GoalsService {
    pub fn new(db: PgPool, clock: Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }

    pub async fn get_active_weight_goal(&self) -> Result<Option<WeightGoal>, CoreError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT params_json FROM goals WHERE domain = $1 AND kind = $2 AND active = 1
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(DOMAIN)
        .bind(KIND)
        .fetch_optional(&self.db)
        .await?;
        match row {
            Some((json,)) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    /// Validates, deactivates any current active weight goal, and inserts `goal` as the new
    /// active one.
    pub async fn save_weight_goal(&self, goal: &WeightGoal) -> Result<(), CoreError> {
        validate(goal)?;

        let now: DateTime<Utc> = self.clock.now();
        let now_str = now.to_rfc3339();
        let mut tx = self.db.begin().await?;

        sqlx::query(
            "UPDATE goals SET active = 0, updated_at = $1 WHERE domain = $2 AND kind = $3 AND active = 1",
        )
        .bind(&now_str)
        .bind(DOMAIN)
        .bind(KIND)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO goals (id, domain, kind, params_json, active, created_at, updated_at)
             VALUES ($1, $2, $3, $4, 1, $5, $6)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(DOMAIN)
        .bind(KIND)
        .bind(serde_json::to_string(goal)?)
        .bind(&now_str)
        .bind(&now_str)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lose(rate: f64) -> WeightGoal {
        WeightGoal {
            direction: GoalDirection::Lose,
            target_weight_kg: Some(65.0),
            rate_kg_per_week: rate,
        }
    }

    #[test]
    fn validate_accepts_reasonable_lose_goal() {
        assert!(validate(&lose(0.5)).is_ok());
    }

    #[test]
    fn validate_accepts_rate_boundaries() {
        assert!(validate(&lose(0.0)).is_ok());
        assert!(validate(&lose(1.0)).is_ok());
    }

    #[test]
    fn validate_rejects_rate_out_of_range_for_lose() {
        assert!(validate(&lose(-0.1)).is_err());
        assert!(validate(&lose(1.1)).is_err());
    }

    #[test]
    fn validate_ignores_rate_for_maintain() {
        let goal = WeightGoal {
            direction: GoalDirection::Maintain,
            target_weight_kg: None,
            rate_kg_per_week: 999.0, // would fail for Lose/Gain; irrelevant for Maintain
        };
        assert!(validate(&goal).is_ok());
    }

    #[test]
    fn validate_rejects_target_weight_out_of_range() {
        assert!(validate(&WeightGoal {
            direction: GoalDirection::Gain,
            target_weight_kg: Some(29.9),
            rate_kg_per_week: 0.25,
        })
        .is_err());
        assert!(validate(&WeightGoal {
            direction: GoalDirection::Gain,
            target_weight_kg: Some(300.1),
            rate_kg_per_week: 0.25,
        })
        .is_err());
    }

    fn untargeted(direction: GoalDirection) -> WeightGoal {
        WeightGoal {
            direction,
            target_weight_kg: None,
            rate_kg_per_week: 0.5,
        }
    }

    #[test]
    fn default_target_is_latest_weight_minus_or_plus_one_kg() {
        let lose = with_default_target(&untargeted(GoalDirection::Lose), Some(70.0));
        assert_eq!(lose.target_weight_kg, Some(69.0));
        let gain = with_default_target(&untargeted(GoalDirection::Gain), Some(70.0));
        assert_eq!(gain.target_weight_kg, Some(71.0));
    }

    #[test]
    fn default_target_is_clamped_to_30_and_300_kg() {
        let lose = with_default_target(&untargeted(GoalDirection::Lose), Some(30.5));
        assert_eq!(lose.target_weight_kg, Some(30.0));
        let gain = with_default_target(&untargeted(GoalDirection::Gain), Some(299.5));
        assert_eq!(gain.target_weight_kg, Some(300.0));
    }

    #[test]
    fn default_target_keeps_explicit_target_and_skips_maintain_or_no_weight() {
        assert_eq!(
            with_default_target(&lose(0.5), Some(70.0)).target_weight_kg,
            Some(65.0)
        );
        assert_eq!(
            with_default_target(&untargeted(GoalDirection::Maintain), Some(70.0)).target_weight_kg,
            None
        );
        assert_eq!(
            with_default_target(&untargeted(GoalDirection::Lose), None).target_weight_kg,
            None
        );
    }

    #[test]
    fn validate_accepts_missing_target_weight() {
        assert!(validate(&WeightGoal {
            direction: GoalDirection::Lose,
            target_weight_kg: None,
            rate_kg_per_week: 0.5,
        })
        .is_ok());
    }

    #[tokio::test]
    async fn get_active_is_none_before_any_goal_is_saved() {
        let (pool, _dir) = crate::db::test_pool().await;
        let service = GoalsService::new(pool, Arc::new(crate::clock::FixedClock::new(Utc::now())));
        assert_eq!(service.get_active_weight_goal().await.unwrap(), None);
    }

    #[tokio::test]
    async fn save_then_get_active_round_trips() {
        let (pool, _dir) = crate::db::test_pool().await;
        let service = GoalsService::new(pool, Arc::new(crate::clock::FixedClock::new(Utc::now())));
        let goal = lose(0.5);

        service.save_weight_goal(&goal).await.unwrap();
        assert_eq!(service.get_active_weight_goal().await.unwrap(), Some(goal));
    }

    #[tokio::test]
    async fn save_rejects_invalid_goal_and_does_not_store_it() {
        let (pool, _dir) = crate::db::test_pool().await;
        let service = GoalsService::new(pool, Arc::new(crate::clock::FixedClock::new(Utc::now())));

        assert!(service.save_weight_goal(&lose(5.0)).await.is_err());
        assert_eq!(service.get_active_weight_goal().await.unwrap(), None);
    }

    #[tokio::test]
    async fn saving_a_new_goal_deactivates_the_previous_one() {
        let (pool, _dir) = crate::db::test_pool().await;
        let service = GoalsService::new(pool, Arc::new(crate::clock::FixedClock::new(Utc::now())));

        service.save_weight_goal(&lose(0.5)).await.unwrap();
        service.save_weight_goal(&lose(0.25)).await.unwrap();

        // Only the newest goal is active; there is never more than one.
        let active: Vec<(i64,)> = sqlx::query_as("SELECT active FROM goals WHERE active = 1")
            .fetch_all(&service.db)
            .await
            .unwrap();
        assert_eq!(active.len(), 1);

        assert_eq!(
            service.get_active_weight_goal().await.unwrap(),
            Some(lose(0.25))
        );
    }
}
