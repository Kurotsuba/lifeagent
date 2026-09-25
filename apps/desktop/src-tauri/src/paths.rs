//! Default data directory resolution, per `docs/PHASE1_SPEC.md` §3.
//!
//! Mirrors `lifeagent-cli`'s `paths::default_data_dir` (`crates/cli/src/paths.rs`): both are
//! leaf composition roots, and the ~10 lines here aren't worth a shared crate for.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// `directories::ProjectDirs::from("", "", "LifeAgent").data_dir()` (on Windows,
/// `%APPDATA%\LifeAgent\data`).
pub fn default_data_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "LifeAgent")
        .context("could not determine a home directory to place LifeAgent's data in")?;
    Ok(dirs.data_dir().to_path_buf())
}
