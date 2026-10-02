//! Forwards `CoreApp`'s `AppEvent` bus to the webview as Tauri events, per
//! `docs/PHASE1_SPEC.md` §10.1: `data-changed`, `plan-ready`, `plan-failed`.
//!
//! Notifications on `PlanReady`/`PlanFailed` are a separate step (`notify.rs`).

use lifeagent_api::events::payload_for;
use lifeagent_core::app::CoreApp;
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

fn emit(app: &AppHandle, event: &str, payload: serde_json::Value) {
    if let Err(e) = app.emit(event, payload) {
        tracing::error!(error = %e, event, "failed to emit event to the webview");
    }
}
