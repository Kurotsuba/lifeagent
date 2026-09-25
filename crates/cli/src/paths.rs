//! Default data directory resolution, per `docs/PHASE1_SPEC.md` §3.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// `directories::ProjectDirs::from("", "", "LifeAgent").data_dir()` (on Windows,
/// `%APPDATA%\LifeAgent\data`). `--data-dir` overrides this; see `Cli::data_dir`.
pub fn default_data_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "LifeAgent").context(
        "could not determine a home directory to place LifeAgent's data in; pass --data-dir",
    )?;
    Ok(dirs.data_dir().to_path_buf())
}
