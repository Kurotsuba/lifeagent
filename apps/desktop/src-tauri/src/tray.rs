//! Tray icon and menu (打开 / 快速记录 / 生成明日计划 / 退出), close-to-tray behavior, and the
//! quick-entry popup window, per `docs/PHASE1_SPEC.md` §10.1/§10.3.

use lifeagent_core::app::CoreApp;
use lifeagent_core::plan::PlanTrigger;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::commands;
use crate::error::CommandError;
use crate::notify;

/// Window label the quick-entry popup is created/looked-up under; matched client-side
/// (`main.tsx`) via `getCurrentWindow().label` to decide which React root to render.
pub const QUICK_ENTRY_LABEL: &str = "quick-entry";
const QUICK_ENTRY_WIDTH: f64 = 420.0;
const QUICK_ENTRY_HEIGHT: f64 = 120.0;

/// Builds the tray icon and its menu, and wires close-to-tray on the main window. Called once
/// from `setup()` after `core_app` is built and managed. `core_app` is cloned into the menu
/// event closure (cheap — see `CoreApp`'s own `Clone`), not taken from Tauri's managed state at
/// click time, since the closure only needs to call existing `CoreApp` methods directly, not
/// round-trip through the command/IPC layer it wasn't invoked through.
pub fn build(app: &AppHandle, core_app: CoreApp) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "打开", true, None::<&str>)?;
    let quick_entry = MenuItem::with_id(app, "quick_entry", "快速记录", true, None::<&str>)?;
    let generate_plan =
        MenuItem::with_id(app, "generate_plan", "生成明日计划", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quick_entry, &generate_plan, &quit])?;

    // The app's own configured icon (`tauri.conf.json`'s `bundle.icon`) — a tray needs an icon
    // and this app ships no separate tray-specific asset, so reusing the window icon is the
    // simplest option consistent with the spec (which doesn't call for a distinct tray glyph).
    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundle.icon is configured in tauri.conf.json, so the app icon always exists");

    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .on_menu_event(move |app, event| handle_menu_event(app, event.id().as_ref(), &core_app))
        .build(app)?;

    if let Some(window) = app.get_webview_window("main") {
        let window_for_close = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(e) = window_for_close.hide() {
                    tracing::error!(error = %e, "close-to-tray: failed to hide main window");
                }
            }
        });
    }

    Ok(())
}

fn handle_menu_event(app: &AppHandle, id: &str, core_app: &CoreApp) {
    match id {
        "open" => focus_main_window(app),
        "quick_entry" => open_quick_entry(app),
        "generate_plan" => generate_tomorrow_plan(app.clone(), core_app.clone()),
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Shows and focuses the main window — the tray's own 打开 item, and (via `lib.rs`'s
/// single-instance callback) what a second launch of the app does instead of opening a new
/// window.
pub fn focus_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Err(e) = window.show() {
        tracing::error!(error = %e, "tray: failed to show main window");
    }
    if let Err(e) = window.set_focus() {
        tracing::error!(error = %e, "tray: failed to focus main window");
    }
}

/// Reuses the existing quick-entry window (just shows/focuses it) if one is already open —
/// closing it after every use would also work, but keeping it alive avoids rebuilding the
/// webview on every 快速记录 click.
fn open_quick_entry(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(QUICK_ENTRY_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let result =
        WebviewWindowBuilder::new(app, QUICK_ENTRY_LABEL, WebviewUrl::App("index.html".into()))
            .title("LifeAgent 快速记录")
            .inner_size(QUICK_ENTRY_WIDTH, QUICK_ENTRY_HEIGHT)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .center()
            .visible(true)
            .focused(true)
            .build();
    if let Err(e) = result {
        tracing::error!(error = %e, "tray: failed to open the quick-entry window");
    }
}

/// Runs `CoreApp::generate_plan` directly (not via the command/IPC layer, since this click
/// didn't come from the webview) for tomorrow's date, reusing `commands::resolve_plan_date`'s
/// same "no date given -> tomorrow" default so this matches what Today's own 立即生成 button
/// does. The outcome is reported as a toast (`notify::show_tray_plan_result`), since the main
/// window is typically hidden when the tray menu is used.
fn generate_tomorrow_plan(app: AppHandle, core_app: CoreApp) {
    tauri::async_runtime::spawn(async move {
        let result = match commands::resolve_plan_date(&core_app, None).await {
            Ok(date) => core_app
                .generate_plan(date, PlanTrigger::Manual)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "tray: 生成明日计划 failed");
                    CommandError::from(e)
                }),
            Err(e) => Err(e),
        };
        match &result {
            Ok(plan) => notify::show_tray_plan_result(&app, Ok(&plan.content.summary)),
            Err(e) => notify::show_tray_plan_result(&app, Err(&e.message)),
        }
    });
}
