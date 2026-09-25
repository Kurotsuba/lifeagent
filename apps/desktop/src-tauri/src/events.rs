//! Forwards `CoreApp`'s `AppEvent` bus to the webview as Tauri events, per
//! `docs/PHASE1_SPEC.md` §10.1: `data-changed`, `plan-ready`, `plan-failed`.
//!
//! Notifications on `PlanReady`/`PlanFailed` are a separate step (`notify.rs`).

use lifeagent_core::app::CoreApp;
use lifeagent_core::events::AppEvent;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast::error::RecvError;

/// Spawns the forwarding task. `core_app` is only borrowed to open a receiver; the task itself
/// owns nothing but that receiver and the `AppHandle`, so this doesn't keep `CoreApp` from being
/// moved into Tauri's managed state right after this call.
pub fn spawn(app: AppHandle, core_app: &CoreApp) {
    let mut rx = core_app.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let (name, payload) = payload_for(&event);
                    emit(&app, name, payload);
                }
                // A slow-consuming frontend just misses the oldest events; the next `get_today`/
                // `get_history` refetch (also triggered by whichever event *does* arrive) still
                // catches it up, so this is a warning, not a reason to stop forwarding.
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        skipped,
                        "event forwarder lagged; some AppEvents were skipped"
                    );
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// The Tauri event name and JSON payload for one `AppEvent`. Kept separate from `spawn` so the
/// mapping itself is testable without a live `AppHandle`/webview.
fn payload_for(event: &AppEvent) -> (&'static str, serde_json::Value) {
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
        AppEvent::PlanFailed { date, error } => {
            ("plan-failed", json!({ "date": date, "error": error }))
        }
    }
}

fn emit(app: &AppHandle, event: &str, payload: serde_json::Value) {
    if let Err(e) = app.emit(event, payload) {
        tracing::error!(error = %e, event, "failed to emit event to the webview");
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
    fn plan_failed_maps_to_plan_failed_event_name_and_payload() {
        let event = AppEvent::PlanFailed {
            date: date("2026-09-26"),
            error: "boom".into(),
        };

        let (name, payload) = payload_for(&event);

        assert_eq!(name, "plan-failed");
        assert_eq!(payload, json!({ "date": "2026-09-26", "error": "boom" }));
    }
}
