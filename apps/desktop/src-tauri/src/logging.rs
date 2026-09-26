//! Tracing setup, per `docs/PHASE1_SPEC.md` §12: a daily rolling log file in `data_dir/logs`
//! (14 files kept), INFO by default, `LIFEAGENT_LOG` overriding the filter. Also logs to stdout,
//! which is what `npm run tauri dev` shows.

use std::path::Path;

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

const MAX_LOG_FILES: usize = 14;

/// Installs the global subscriber. If the log directory can't be set up, falls back to stdout
/// only (and says so) rather than refusing to start the app over a logging problem.
///
/// The file appender is used as a blocking writer rather than through
/// `tracing_appender::non_blocking`: the latter's `WorkerGuard` must be dropped to flush, and
/// Tauri's `app.exit(0)` (the tray's 退出) ends the process without unwinding `run()`, so buffered
/// lines at exit would be lost. Log volume here is low enough that blocking writes don't matter.
pub fn init(data_dir: Option<&Path>) {
    let filter =
        || EnvFilter::try_from_env("LIFEAGENT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    let file_result = data_dir.map(|dir| {
        let log_dir = dir.join("logs");
        // Created up front: the appender prunes old files (`max_log_files`) by listing this
        // directory while it's being built, and prints an error to stderr on a first run if it
        // doesn't exist yet. A failure here surfaces through `build` below anyway.
        let _ = std::fs::create_dir_all(&log_dir);
        RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix("lifeagent")
            .filename_suffix("log")
            .max_log_files(MAX_LOG_FILES)
            .build(log_dir)
    });

    let stdout_layer = fmt::layer();
    match file_result {
        Some(Ok(appender)) => {
            tracing_subscriber::registry()
                .with(filter())
                .with(stdout_layer)
                .with(fmt::layer().with_ansi(false).with_writer(appender))
                .init();
        }
        Some(Err(e)) => {
            tracing_subscriber::registry()
                .with(filter())
                .with(stdout_layer)
                .init();
            tracing::error!(error = %e, "could not open the log directory; logging to stdout only");
        }
        None => {
            tracing_subscriber::registry()
                .with(filter())
                .with(stdout_layer)
                .init();
            tracing::error!("no data directory; logging to stdout only");
        }
    }
}
