//! Windows notifications on `PlanReady`/`PlanFailed`, per `docs/PHASE1_SPEC.md` §10.1.
//!
//! **Not implemented here:** clicking a notification to focus the main window on Today (also
//! specified in §10.1). `tauri-plugin-notification`'s desktop backend has no click/activation
//! callback in its public API (it wraps `notify_rust`, which exposes only
//! title/body/icon/sound) — reaching that needs a lower-level, Windows-toast-specific API this
//! step doesn't take on. See `docs/DECISIONS.md`'s step-4 entry.

use chrono::NaiveDate;
use lifeagent_core::app::CoreApp;
use lifeagent_core::events::AppEvent;
use lifeagent_core::plan::PlanTrigger;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::locale;
use tokio::sync::broadcast::error::RecvError;

/// Which notification an `AppEvent` raises. Its title and body are worded in the UI's language
/// (`locale.rs`) when it is shown. A `PlanFailed` body is chosen from the event's `code`
/// (`CoreError::code()`) rather than the underlying `CoreError`'s own (English, developer-facing)
/// `Display` text — not something to put in a user's toast. That detail is already logged by the
/// scheduler (`scheduler.rs`'s `tracing::warn!`), so nothing is lost, just not duplicated here in
/// a form a user would see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notification {
    PlanReady,
    PlanFailed,
}

/// Spawns the notification task. `core_app` is a clone the task owns for the rest of the
/// process's life (it needs `settings()`/`get_plan()` on every event, unlike `events::spawn`'s
/// forwarder, which only borrows `CoreApp` transiently to open a receiver).
pub fn spawn(app: AppHandle, core_app: CoreApp) {
    let mut rx = core_app.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => handle(&app, &core_app, event).await,
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        skipped,
                        "notification listener lagged; some AppEvents were skipped"
                    );
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

async fn handle(app: &AppHandle, core_app: &CoreApp, event: AppEvent) {
    let Some((kind, date)) = notification_target(&event) else {
        return;
    };
    let enabled = core_app
        .settings()
        .notifications_enabled()
        .await
        .unwrap_or(true);
    if !enabled {
        return;
    }

    let body = match &event {
        AppEvent::PlanReady { .. } => match core_app.get_plan(date).await {
            Ok(Some(plan)) => first_sentence(&plan.content.summary),
            Ok(None) => {
                tracing::warn!(%date, "PlanReady fired but no plan is stored for that date");
                return;
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to load the plan for its notification body");
                return;
            }
        },
        AppEvent::PlanFailed { code, .. } => texts().plan_failed_body(code).to_string(),
        AppEvent::DataChanged { .. } => return,
    };

    let title = match kind {
        Notification::PlanReady => texts().plan_ready_title,
        Notification::PlanFailed => texts().plan_failed_title,
    };
    show(app, title, &body);
}

/// Which `AppEvent`s notify, and their title — pure, so it's testable without a live `CoreApp`
/// or `AppHandle`. `PlanReady` only notifies for a *scheduled* plan (§10.1): a manual or
/// revision generation is something the user just did themselves in the UI and already sees the
/// result of, without needing a toast to tell them. `PlanFailed` needs no such filter: only the
/// scheduler publishes it (a manual generation's failure is returned to its caller instead), so
/// every `PlanFailed` is already a scheduled one.
fn notification_target(event: &AppEvent) -> Option<(Notification, NaiveDate)> {
    match event {
        AppEvent::PlanReady {
            date,
            trigger: PlanTrigger::Schedule,
            ..
        } => Some((Notification::PlanReady, *date)),
        AppEvent::PlanFailed { date, .. } => Some((Notification::PlanFailed, *date)),
        _ => None,
    }
}

/// The first sentence of `text` (through the first "。", "！", "？", ".", "!" or "?"), or all of
/// `text` if none of those appear.
fn first_sentence(text: &str) -> String {
    let end = text
        .char_indices()
        .find(|(_, c)| matches!(c, '。' | '！' | '？' | '.' | '!' | '?'))
        .map(|(i, c)| i + c.len_utf8());
    match end {
        Some(end) => text[..end].trim().to_string(),
        None => text.trim().to_string(),
    }
}

/// Result toast for the tray's 生成明日计划 (`tray.rs`). That's a `Manual` generation, so the
/// event-driven path above never notifies on success, and a manual failure never publishes
/// `PlanFailed` at all (only the scheduler does) — but the main window is usually hidden in the
/// tray when this is clicked, so without this the click would have no visible outcome.
/// Shown regardless of `notifications_enabled`: it's the direct response to the user's own click,
/// not an unsolicited notification. `result` is the plan's summary, or the failure's
/// `CommandError::code`, worded the same way as a scheduled failure.
pub fn show_tray_plan_result(app: &AppHandle, result: Result<&str, &str>) {
    let t = texts();
    match result {
        Ok(summary) => show(app, t.plan_ready_title, &first_sentence(summary)),
        Err(code) => show(app, t.plan_failed_title, t.plan_failed_body(code)),
    }
}

fn texts() -> &'static locale::Texts {
    locale::texts(locale::current())
}

fn show(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::error!(error = %e, "failed to show notification");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn scheduled_plan_ready_notifies() {
        let event = AppEvent::PlanReady {
            date: date("2026-09-26"),
            version: 1,
            trigger: PlanTrigger::Schedule,
        };

        assert_eq!(
            notification_target(&event),
            Some((Notification::PlanReady, date("2026-09-26")))
        );
    }

    #[test]
    fn manual_and_revision_plan_ready_do_not_notify() {
        for trigger in [PlanTrigger::Manual, PlanTrigger::Revision] {
            let event = AppEvent::PlanReady {
                date: date("2026-09-26"),
                version: 1,
                trigger,
            };
            assert_eq!(notification_target(&event), None);
        }
    }

    #[test]
    fn plan_failed_always_notifies() {
        let event = AppEvent::PlanFailed {
            date: date("2026-09-26"),
            error: "boom".into(),
            code: "internal",
        };

        assert_eq!(
            notification_target(&event),
            Some((Notification::PlanFailed, date("2026-09-26")))
        );
    }

    #[test]
    fn data_changed_does_not_notify() {
        let event = AppEvent::DataChanged {
            event_type: "diet.meal_logged".into(),
            logical_date: date("2026-09-25"),
        };

        assert_eq!(notification_target(&event), None);
    }

    #[test]
    fn first_sentence_stops_at_a_full_width_period() {
        assert_eq!(
            first_sentence("明天多吃蔬菜。控制精制碳水，晚餐提早。"),
            "明天多吃蔬菜。"
        );
    }

    #[test]
    fn first_sentence_stops_at_an_ascii_terminator() {
        assert_eq!(first_sentence("Plan ready. Details inside."), "Plan ready.");
    }

    #[test]
    fn first_sentence_returns_the_whole_text_when_there_is_no_terminator() {
        assert_eq!(first_sentence("明天多吃蔬菜"), "明天多吃蔬菜");
    }
}
