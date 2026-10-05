//! Accounts and sessions (`docs/HACKATHON.md` §1, "Account management").
//!
//! - Passwords are hashed with Argon2id (the `argon2` crate's defaults: m=19 MiB, t=2, p=1),
//!   off the async runtime.
//! - A session is a random 256-bit token in an `HttpOnly`, `SameSite=Lax`, `Secure` cookie. The
//!   `sessions` table stores only its SHA-256, so a database leak doesn't hand out sessions.
//! - Login and registration answer wrong usernames and wrong passwords identically and take
//!   about the same time (a dummy hash is verified for unknown users), and are rate-limited
//!   per client IP.
//! - Registering creates the user's tenant (schema + role) through `Tenants::get`, which
//!   provisions it; a failure removes the half-made account again.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Duration as ChronoDuration};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ApiError;
use crate::AppState;
use lifeagent_api::ErrorDetail;

pub const SESSION_COOKIE: &str = "lifeagent_session";
const SESSION_DAYS: i64 = 30;

/// Login + registration attempts allowed per client IP per window.
pub const ATTEMPTS_PER_WINDOW: u32 = 20;
pub const ATTEMPT_WINDOW: Duration = Duration::from_secs(10 * 60);

// ---- Validation ------------------------------------------------------------------------------

/// Usernames are case-insensitive: stored and compared lowercase.
pub fn normalize_username(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim().to_ascii_lowercase();
    let ok = (3..=32).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-.".contains(c));
    if ok {
        Ok(name)
    } else {
        Err(ApiError::invalid(ErrorDetail::InvalidUsername))
    }
}

pub fn validate_password(password: &str) -> Result<(), ApiError> {
    if (8..=128).contains(&password.chars().count()) {
        Ok(())
    } else {
        Err(ApiError::invalid(ErrorDetail::InvalidPassword))
    }
}

// ---- Passwords and tokens ---------------------------------------------------------------------

fn random_bytes<const N: usize>() -> Result<[u8; N], ApiError> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(ApiError::internal)?;
    Ok(buf)
}

pub async fn hash_password(password: String) -> Result<String, ApiError> {
    let salt_bytes = random_bytes::<16>()?;
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::encode_b64(&salt_bytes).map_err(ApiError::internal)?;
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(ApiError::internal)
    })
    .await
    .map_err(ApiError::internal)?
}

pub async fn verify_password(password: String, hash: String) -> Result<bool, ApiError> {
    tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&hash).map_err(ApiError::internal)?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    })
    .await
    .map_err(ApiError::internal)?
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

// ---- Sessions ---------------------------------------------------------------------------------

/// The logged-in user, from the session cookie. Every `/api/*` route except register, login
/// and logout takes one, so an unauthenticated request never reaches a `CoreApp`.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: String,
    pub username: String,
    /// The user's tenant schema (and role).
    pub schema: String,
}

fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value.to_string())
        .filter(|v| !v.is_empty())
}

impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let token = session_token(&parts.headers).ok_or_else(ApiError::unauthorized)?;
        let row: Option<(String, String, String, String)> = sqlx::query_as(
            "SELECT u.id, u.username, u.schema_name, s.expires_at
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1",
        )
        .bind(token_hash(&token))
        .fetch_optional(&state.auth_db)
        .await?;
        let (id, username, schema, expires_at) = row.ok_or_else(ApiError::unauthorized)?;
        let expires_at = DateTime::parse_from_rfc3339(&expires_at).map_err(ApiError::internal)?;
        if expires_at <= state.clock.now() {
            return Err(ApiError::unauthorized());
        }
        Ok(AuthUser {
            id,
            username,
            schema,
        })
    }
}

fn session_cookie(state: &AppState, token: &str, max_age_secs: i64) -> HeaderValue {
    let secure = if state.config.secure_cookies {
        "; Secure"
    } else {
        ""
    };
    let value = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{secure}"
    );
    // The token is hex and everything else is fixed ASCII, so this can't fail.
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

async fn start_session(state: &AppState, user_id: &str) -> Result<HeaderValue, ApiError> {
    let token = hex(&random_bytes::<32>()?);
    let now = state.clock.now();
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(token_hash(&token))
    .bind(user_id)
    .bind(now.to_rfc3339())
    .bind((now + ChronoDuration::days(SESSION_DAYS)).to_rfc3339())
    .execute(&state.auth_db)
    .await?;
    Ok(session_cookie(state, &token, SESSION_DAYS * 24 * 60 * 60))
}

// ---- Rate limiting ----------------------------------------------------------------------------

/// Fixed-window attempt counter per key (client IP for logins, user id for model-backed
/// commands), in memory (per server instance; the service runs one).
pub struct RateLimiter<K> {
    limit: u32,
    window: Duration,
    windows: Mutex<HashMap<K, (Instant, u32)>>,
}

impl<K: std::hash::Hash + Eq> RateLimiter<K> {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            windows: Mutex::new(HashMap::new()),
        }
    }

    pub fn allow(&self, key: K) -> bool {
        let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        if windows.len() > 10_000 {
            windows.retain(|_, (start, _)| now.duration_since(*start) < self.window);
        }
        let entry = windows.entry(key).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= self.limit
    }
}

/// The client's IP. Behind Cloud Run's front end the TCP peer is Google's proxy, which appends
/// the real client address as the *last* `X-Forwarded-For` entry; earlier entries come from
/// the client and can be forged, so only the last one is used.
fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .and_then(|v| v.trim().parse().ok())
        .or_else(|| peer.map(|p| p.ip()))
}

/// The TCP peer, when the server was started with connect info (`main.rs` does; tests that
/// serve without it get `None`).
pub struct PeerAddr(Option<SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for PeerAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(PeerAddr(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| *addr),
        ))
    }
}

fn check_rate(state: &AppState, headers: &HeaderMap, peer: PeerAddr) -> Result<(), ApiError> {
    match client_ip(headers, peer.0) {
        Some(ip) if !state.limiter.allow(ip) => {
            Err(ApiError::new("rate_limited", ErrorDetail::TooManyAttempts))
        }
        _ => Ok(()),
    }
}

// ---- Handlers ---------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
}

#[derive(Serialize)]
pub struct Me {
    username: String,
}

fn with_cookie(cookie: HeaderValue, body: impl IntoResponse) -> Response {
    let mut response = body.into_response();
    response.headers_mut().insert(SET_COOKIE, cookie);
    response
}

fn invalid_credentials() -> ApiError {
    ApiError::new("invalid_credentials", ErrorDetail::InvalidCredentials)
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    peer: PeerAddr,
    headers: HeaderMap,
    Json(creds): Json<Credentials>,
) -> Result<Response, ApiError> {
    check_rate(&state, &headers, peer)?;
    let username = normalize_username(&creds.username)?;
    validate_password(&creds.password)?;
    let password_hash = hash_password(creds.password).await?;

    let id = Uuid::now_v7();
    // Derived from the generated id only, never from user input (`docs/HACKATHON.md` §1).
    let schema = format!("u_{}", id.simple());
    let inserted = sqlx::query(
        "INSERT INTO users (id, username, password_hash, schema_name, created_at)
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT (username) DO NOTHING",
    )
    .bind(id.to_string())
    .bind(&username)
    .bind(&password_hash)
    .bind(&schema)
    .bind(state.clock.now().to_rfc3339())
    .execute(&state.auth_db)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::new("username_taken", ErrorDetail::UsernameTaken));
    }

    // Provisions the tenant (schema, role, migrations).
    if let Err(e) = state.tenants.get(&schema).await {
        state.tenants.remove(&schema);
        let _ = lifeagent_core::db::destroy(&state.tenants.factory().tenant_config(&schema)).await;
        let _ = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id.to_string())
            .execute(&state.auth_db)
            .await;
        return Err(ApiError::internal(e));
    }
    tracing::info!(user_id = %id, "account registered");

    let cookie = start_session(&state, &id.to_string()).await?;
    Ok(with_cookie(cookie, Json(Me { username })))
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    peer: PeerAddr,
    headers: HeaderMap,
    Json(creds): Json<Credentials>,
) -> Result<Response, ApiError> {
    check_rate(&state, &headers, peer)?;
    // A malformed username can't exist, but still goes through a hash check below so the
    // response time doesn't tell it apart from a wrong password.
    let username = normalize_username(&creds.username).unwrap_or_default();
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE username = $1")
            .bind(&username)
            .fetch_optional(&state.auth_db)
            .await?;
    let (user_id, hash) = match row {
        Some((id, hash)) => (Some(id), hash),
        None => (None, state.dummy_hash.clone()),
    };
    let password_ok = verify_password(creds.password, hash).await?;
    let Some(user_id) = user_id.filter(|_| password_ok) else {
        return Err(invalid_credentials());
    };

    let cookie = start_session(&state, &user_id).await?;
    Ok(with_cookie(cookie, Json(Me { username })))
}

/// Ends the current session if there is one; always clears the cookie.
pub async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(token) = session_token(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash(&token))
            .execute(&state.auth_db)
            .await?;
    }
    Ok(with_cookie(
        session_cookie(&state, "", 0),
        Json(serde_json::Value::Null),
    ))
}

pub async fn me(user: AuthUser) -> Json<Me> {
    Json(Me {
        username: user.username,
    })
}

#[derive(Deserialize)]
pub struct DeleteAccount {
    password: String,
}

/// Deletes the account and all of its data: the tenant (schema with every table, and role) and
/// the user row, which cascades to every session. Asks for the password again.
pub async fn delete_account(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Json(body): Json<DeleteAccount>,
) -> Result<Response, ApiError> {
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(&user.id)
        .fetch_one(&state.auth_db)
        .await?;
    if !verify_password(body.password, hash).await? {
        return Err(invalid_credentials());
    }

    state.tenants.remove(&user.schema);
    lifeagent_core::db::destroy(&state.tenants.factory().tenant_config(&user.schema)).await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(&user.id)
        .execute(&state.auth_db)
        .await?;
    tracing::info!(user_id = %user.id, "account deleted");
    Ok(with_cookie(
        session_cookie(&state, "", 0),
        Json(serde_json::Value::Null),
    ))
}

/// A real Argon2id hash of a random password, verified against when a login names an unknown
/// user, so that case costs the same as a wrong password.
pub async fn dummy_hash() -> Result<String, ApiError> {
    hash_password(hex(&random_bytes::<16>()?)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_lowercased_and_restricted() {
        assert_eq!(normalize_username("  Alice_01 ").unwrap(), "alice_01");
        assert!(normalize_username("ab").is_err());
        assert!(normalize_username(&"a".repeat(33)).is_err());
        assert!(normalize_username("名前ですよ").is_err());
        assert!(normalize_username("a b c").is_err());
    }

    #[test]
    fn passwords_need_8_to_128_characters() {
        assert!(validate_password("1234567").is_err());
        assert!(validate_password("12345678").is_ok());
        assert!(validate_password(&"x".repeat(129)).is_err());
    }

    #[tokio::test]
    async fn hashes_verify_only_the_right_password() {
        let hash = hash_password("correct horse".into()).await.unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse".into(), hash.clone())
            .await
            .unwrap());
        assert!(!verify_password("wrong horse".into(), hash).await.unwrap());
    }

    #[test]
    fn the_session_cookie_is_found_among_others() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("a=1; lifeagent_session=abc123; b=2"),
        );
        assert_eq!(session_token(&headers).as_deref(), Some("abc123"));
        assert_eq!(session_token(&HeaderMap::new()), None);
    }

    #[test]
    fn client_ip_uses_the_last_forwarded_entry() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("6.6.6.6, 203.0.113.7"),
        );
        assert_eq!(
            client_ip(&headers, None),
            Some("203.0.113.7".parse().unwrap())
        );
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        assert_eq!(client_ip(&HeaderMap::new(), Some(peer)), Some(peer.ip()));
    }

    #[test]
    fn the_rate_limiter_caps_attempts_per_ip() {
        let limiter = RateLimiter::new(ATTEMPTS_PER_WINDOW, ATTEMPT_WINDOW);
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        for _ in 0..ATTEMPTS_PER_WINDOW {
            assert!(limiter.allow(ip));
        }
        assert!(!limiter.allow(ip));
        assert!(limiter.allow("203.0.113.8".parse().unwrap()));
    }
}
