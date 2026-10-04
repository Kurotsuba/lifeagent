//! The event names and JSON payloads every shell pushes to its UI, per `docs/PHASE1_SPEC.md`
//! §10.1: `data-changed`, `plan-ready`, `plan-failed`, plus `plan-generating`. The web server sends them as
//! Server-Sent Events.

use lifeagent_core::events::AppEvent;
use serde_json::json;

/// The event name and JSON payload for one `AppEvent`.
pub fn payload_for(event: &AppEvent) -> (&'static str, serde_json::Value) {
    match event {
        AppEvent::DataChanged {
            event_type,
            logical_date,
        } => (
            "data-changed",
            json!({ "event_type": event_type, "logical_date": logical_date }),
        ),
        AppEvent::PlanReady {
            date,
            version,
            trigger,
        } => (
            "plan-ready",
            json!({ "date": date, "version": version, "trigger": trigger }),
        ),
        AppEvent::PlanGenerating { generating } => {
            ("plan-generating", json!({ "generating": generating }))
        }
        AppEvent::PlanFailed { date, error, code } => (
            "plan-failed",
            json!({ "date": date, "error": error, "code": code }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use lifeagent_core::plan::PlanTrigger;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn data_changed_maps_to_data_changed_event_name_and_payload() {
        let event = AppEvent::DataChanged {
            event_type: "diet.meal_logged".into(),
            logical_date: date("2026-09-25"),
        };

        let (name, payload) = payload_for(&event);

        assert_eq!(name, "data-changed");
        assert_eq!(
            payload,
            json!({ "event_type": "diet.meal_logged", "logical_date": "2026-09-25" })
        );
    }

    #[test]
    fn plan_ready_maps_to_plan_ready_event_name_and_payload() {
        let event = AppEvent::PlanReady {
            date: date("2026-09-26"),
            version: 2,
            trigger: PlanTrigger::Schedule,
        };

        let (name, payload) = payload_for(&event);

        assert_eq!(name, "plan-ready");
        assert_eq!(
            payload,
            json!({ "date": "2026-09-26", "version": 2, "trigger": "schedule" })
        );
    }

    #[test]
    fn plan_generating_maps_to_plan_generating_event_name_and_payload() {
        let (name, payload) = payload_for(&AppEvent::PlanGenerating { generating: true });

        assert_eq!(name, "plan-generating");
        assert_eq!(payload, json!({ "generating": true }));
    }

    #[test]
    fn plan_failed_maps_to_plan_failed_event_name_and_payload() {
        let event = AppEvent::PlanFailed {
            date: date("2026-09-26"),
            error: "boom".into(),
            code: "internal",
        };

        let (name, payload) = payload_for(&event);

        assert_eq!(name, "plan-failed");
        assert_eq!(
            payload,
            json!({ "date": "2026-09-26", "error": "boom", "code": "internal" })
        );
    }
}
