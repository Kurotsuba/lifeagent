//! The plugin's two switches, stored in core's key-value `settings` table under
//! [`SETTINGS_KEY`]. Both default to on; with both off the plugin is a plain pantry list.

use chrono::{DateTime, Utc};
use lifeagent_core::plugin::{PluginContext, PluginError};
use serde::{Deserialize, Serialize};

pub const SETTINGS_KEY: &str = "stock.settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StockSettings {
    /// Logged meals take what they used out of stock.
    pub auto_consume: bool,
    /// Plans are built from what's in stock and come with a shopping list.
    pub plan_from_stock: bool,
}

impl Default for StockSettings {
    fn default() -> Self {
        Self {
            auto_consume: true,
            plan_from_stock: true,
        }
    }
}

pub async fn get(ctx: &PluginContext) -> Result<StockSettings, PluginError> {
    Ok(ctx
        .settings
        .get_json::<StockSettings>(SETTINGS_KEY)
        .await?
        .unwrap_or_default())
}

pub async fn set(
    ctx: &PluginContext,
    settings: &StockSettings,
    now: DateTime<Utc>,
) -> Result<(), PluginError> {
    Ok(ctx.settings.set_json(SETTINGS_KEY, settings, now).await?)
}
