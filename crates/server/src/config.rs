//! Server configuration from environment variables (Cloud Run sets them per revision; see
//! `docs/HACKATHON.md` §1).

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use lifeagent_core::db::DEFAULT_DATABASE_URL;
use lifeagent_llm::router::ModelsConfig;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// `LIFEAGENT_DATABASE_URL`. The login role needs `CREATEROLE` and `CREATE` on the database
    /// (Cloud SQL's built-in users have both), to provision each user's tenant.
    pub database_url: String,
    /// `LIFEAGENT_AUTH_SCHEMA`, default `auth`: the schema holding accounts and sessions.
    pub auth_schema: String,
    /// `PORT` (set by Cloud Run), default 8080, on all interfaces. `LIFEAGENT_BIND` overrides.
    pub bind: SocketAddr,
    /// `LIFEAGENT_STATIC_DIR`: the built UI to serve at `/`. Unset: API only.
    pub static_dir: Option<PathBuf>,
    /// `LIFEAGENT_MODELS`: the `ModelsConfig` JSON every user's app uses (§6.3 shape). Users
    /// never configure models in the web build. Keys come from `LIFEAGENT_KEY_<PROVIDER_ID>`.
    pub models: Option<ModelsConfig>,
    /// `LIFEAGENT_MOCK_LLM=1`: every role answers with an echo, for local development without
    /// a model. Overrides `models`.
    pub mock_llm: bool,
    /// `LIFEAGENT_TICK_TOKEN`: the bearer token Cloud Scheduler sends to
    /// `/api/internal/tick`. Unset: the endpoint is disabled.
    pub tick_token: Option<String>,
    /// `LIFEAGENT_INPROCESS_SCHEDULER=1`: tick every user once a minute from inside the server,
    /// for local development or an always-on instance, instead of Cloud Scheduler.
    pub inprocess_scheduler: bool,
    /// `LIFEAGENT_INVITE_CODE`: when set, registration requires it.
    pub invite_code: Option<String>,
    /// `LIFEAGENT_INSECURE_COOKIES=1` drops the cookie's `Secure` flag, for plain-HTTP testing
    /// on something other than `localhost`. Never set it in production.
    pub secure_cookies: bool,
}

fn flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

fn optional(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

impl ServerConfig {
    pub fn from_env() -> Result<Self> {
        let bind = match optional("LIFEAGENT_BIND") {
            Some(addr) => addr.parse().context("LIFEAGENT_BIND is not an address")?,
            None => {
                let port: u16 = optional("PORT")
                    .map(|p| p.parse())
                    .transpose()
                    .context("PORT is not a number")?
                    .unwrap_or(8080);
                SocketAddr::from(([0, 0, 0, 0], port))
            }
        };
        let models = optional("LIFEAGENT_MODELS")
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .context("LIFEAGENT_MODELS is not a valid models config")?;
        Ok(Self {
            database_url: optional("LIFEAGENT_DATABASE_URL")
                .unwrap_or_else(|| DEFAULT_DATABASE_URL.into()),
            auth_schema: optional("LIFEAGENT_AUTH_SCHEMA").unwrap_or_else(|| "auth".into()),
            bind,
            static_dir: optional("LIFEAGENT_STATIC_DIR").map(PathBuf::from),
            models,
            mock_llm: flag("LIFEAGENT_MOCK_LLM"),
            tick_token: optional("LIFEAGENT_TICK_TOKEN"),
            inprocess_scheduler: flag("LIFEAGENT_INPROCESS_SCHEDULER"),
            invite_code: optional("LIFEAGENT_INVITE_CODE"),
            secure_cookies: !flag("LIFEAGENT_INSECURE_COOKIES"),
        })
    }
}
