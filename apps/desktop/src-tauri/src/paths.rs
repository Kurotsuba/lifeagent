//! Default data directory resolution, per `docs/PHASE1_SPEC.md` §3. Since the Postgres port
//! (`docs/DECISIONS.md`, 2026-10-02) it only holds the desktop app's `logs/`.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// `directories::ProjectDirs::from("", "", "LifeAgent").data_dir()` (on Windows,
/// `%APPDATA%\LifeAgent\data`).
pub fn default_data_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "LifeAgent")
        .context("could not determine a home directory to place LifeAgent's data in")?;
    Ok(dirs.data_dir().to_path_buf())
}
