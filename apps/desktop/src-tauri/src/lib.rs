//! Tauri 2 shell — composition root for `CoreApp` and its plugins.
//!
//! Building `CoreApp` (with `SystemClock`, `KeyringSecretStore`, the diet plugin) and
//! starting the scheduler is M7 work per `docs/PHASE1_SPEC.md` §10.1; this M0 skeleton
//! only wires up the Tauri builder, plugins, and `tracing` so the app window opens.

mod commands;
mod notify;
mod tray;

pub fn run() {
    tracing_subscriber::fmt::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {
            // A second launch focuses the existing window; wired to the main window in M7.
        }))
        .setup(|_app| {
            // CoreAppBuilder::build() + managed state + start_scheduler() land in M7.
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("error while running lifeagent-desktop");
}
