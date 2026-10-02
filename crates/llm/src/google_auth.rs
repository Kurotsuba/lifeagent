//! Google Cloud access tokens for Gemini on Agent Platform (`docs/HACKATHON.md` §2).
//!
//! Agent Platform's OpenAI-compatible endpoint accepts only short-lived Google Cloud access
//! tokens (about an hour), not API keys. `GoogleTokenSource` finds credentials the way Google's
//! Application Default Credentials do, for the two cases this project needs:
//!
//! 1. A credentials file: `GOOGLE_APPLICATION_CREDENTIALS`, else the file
//!    `gcloud auth application-default login` writes. Only the `authorized_user` type (a
//!    developer's own login) is supported; its refresh token is exchanged at Google's OAuth
//!    endpoint. Service-account key files are refused: they'd need JWT signing, and Cloud Run
//!    doesn't need them.
//! 2. Otherwise the metadata server, which on Cloud Run hands out tokens for the service's own
//!    service account. No key exists anywhere.
//!
//! Tokens are cached and refreshed `REFRESH_MARGIN` before they expire; concurrent callers
//! share one fetch. Token values are never logged.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::Mutex;

use crate::error::LlmError;

const METADATA_URL: &str = "http://metadata.google.internal";
const METADATA_TOKEN_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";
const OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Refresh this long before expiry, so a request never starts with a token about to lapse.
const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq)]
enum Credentials {
    AuthorizedUser {
        client_id: String,
        client_secret: String,
        refresh_token: String,
        token_url: String,
    },
    MetadataServer {
        base_url: String,
    },
}

struct CachedToken {
    value: String,
    refresh_at: Instant,
}

pub struct GoogleTokenSource {
    client: reqwest::Client,
    /// Resolved on first use, so a process that never calls Agent Platform never looks.
    credentials: Mutex<Option<Credentials>>,
    cache: Mutex<Option<CachedToken>>,
}

/// One token source per process: every user's router shares the cached token.
pub fn shared() -> Arc<GoogleTokenSource> {
    static SHARED: OnceLock<Arc<GoogleTokenSource>> = OnceLock::new();
    SHARED
        .get_or_init(|| Arc::new(GoogleTokenSource::new(None)))
        .clone()
}

#[derive(Deserialize)]
struct CredentialsFile {
    #[serde(rename = "type")]
    kind: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

fn config_error(msg: impl Into<String>) -> LlmError {
    LlmError::Config(msg.into())
}

/// Parses an ADC file. `token_url` is a parameter so tests can point it at a local server.
fn parse_credentials_file(json: &str, token_url: &str) -> Result<Credentials, LlmError> {
    let file: CredentialsFile = serde_json::from_str(json)
        .map_err(|e| config_error(format!("unreadable Google credentials file: {e}")))?;
    match file.kind.as_str() {
        "authorized_user" => match (file.client_id, file.client_secret, file.refresh_token) {
            (Some(client_id), Some(client_secret), Some(refresh_token)) => {
                Ok(Credentials::AuthorizedUser {
                    client_id,
                    client_secret,
                    refresh_token,
                    token_url: token_url.to_string(),
                })
            }
            _ => Err(config_error(
                "Google credentials file is missing client_id, client_secret or refresh_token",
            )),
        },
        other => Err(config_error(format!(
            "Google credentials of type {other:?} are not supported; use \
             `gcloud auth application-default login` locally, or the service account of the \
             Cloud Run service"
        ))),
    }
}

/// Where gcloud writes Application Default Credentials.
fn default_credentials_path() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(|d| PathBuf::from(d).join("gcloud/application_default_credentials.json"))
    } else {
        std::env::var_os("HOME")
            .map(|d| PathBuf::from(d).join(".config/gcloud/application_default_credentials.json"))
    }
}

fn discover() -> Result<Credentials, LlmError> {
    let explicit = std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS").map(PathBuf::from);
    let path = explicit
        .clone()
        .or_else(default_credentials_path)
        .filter(|p| p.is_file());
    match path {
        Some(path) => {
            let json = std::fs::read_to_string(&path).map_err(|e| {
                config_error(format!(
                    "reading Google credentials {}: {e}",
                    path.display()
                ))
            })?;
            parse_credentials_file(&json, OAUTH_TOKEN_URL)
        }
        None if explicit.is_some() => Err(config_error(
            "GOOGLE_APPLICATION_CREDENTIALS points to a file that doesn't exist",
        )),
        None => Ok(Credentials::MetadataServer {
            base_url: METADATA_URL.to_string(),
        }),
    }
}

/// `application/x-www-form-urlencoded` value encoding: unreserved characters as is, the rest
/// as `%XX` (refresh tokens contain `/`).
fn form_encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl GoogleTokenSource {
    /// `credentials`: `None` discovers them on first use (production); tests pass them in.
    fn new(credentials: Option<Credentials>) -> Self {
        Self {
            client: reqwest::Client::new(),
            credentials: Mutex::new(credentials),
            cache: Mutex::new(None),
        }
    }

    /// A valid access token, fetching a new one if none is cached or it expires soon.
    pub async fn token(&self) -> Result<String, LlmError> {
        let mut cache = self.cache.lock().await;
        if let Some(cached) = cache.as_ref() {
            if Instant::now() < cached.refresh_at {
                return Ok(cached.value.clone());
            }
        }
        let credentials = self.credentials().await?;
        let response = self.fetch(&credentials).await?;
        let lifetime = Duration::from_secs(response.expires_in);
        *cache = Some(CachedToken {
            value: response.access_token.clone(),
            refresh_at: Instant::now() + lifetime.saturating_sub(REFRESH_MARGIN),
        });
        tracing::debug!(
            expires_in = response.expires_in,
            "fetched a Google access token"
        );
        Ok(response.access_token)
    }

    async fn credentials(&self) -> Result<Credentials, LlmError> {
        let mut credentials = self.credentials.lock().await;
        if credentials.is_none() {
            *credentials = Some(discover()?);
        }
        credentials
            .clone()
            .ok_or_else(|| config_error("no Google credentials"))
    }

    async fn fetch(&self, credentials: &Credentials) -> Result<TokenResponse, LlmError> {
        let request = match credentials {
            Credentials::MetadataServer { base_url } => self
                .client
                .get(format!("{base_url}{METADATA_TOKEN_PATH}"))
                .header("Metadata-Flavor", "Google"),
            Credentials::AuthorizedUser {
                client_id,
                client_secret,
                refresh_token,
                token_url,
            } => self
                .client
                .post(token_url)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(format!(
                    "grant_type=refresh_token&client_id={}&client_secret={}&refresh_token={}",
                    form_encode(client_id),
                    form_encode(client_secret),
                    form_encode(refresh_token)
                )),
        };
        let response = request.timeout(FETCH_TIMEOUT).send().await.map_err(|e| {
            if matches!(credentials, Credentials::MetadataServer { .. }) {
                config_error(format!(
                    "no Google credentials: no credentials file and the metadata server is \
                     unreachable ({e}); run `gcloud auth application-default login`"
                ))
            } else {
                LlmError::Http(e.to_string())
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            // The body may echo request details; keep it out of the error (CLAUDE.md logging).
            tracing::debug!(status = status.as_u16(), "Google token request failed");
            return Err(match status.as_u16() {
                400 | 401 | 403 => LlmError::Auth,
                code => LlmError::Status {
                    code,
                    body_excerpt: String::new(),
                },
            });
        }
        response
            .json::<TokenResponse>()
            .await
            .map_err(|e| LlmError::InvalidResponse(format!("Google token response: {e}")))
    }
}

/// Test helpers shared with the provider tests: a token source on given credentials, and a
/// one-route HTTP server that answers with canned JSON and records what it was sent.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    pub(crate) fn metadata_source(base_url: &str) -> GoogleTokenSource {
        GoogleTokenSource::new(Some(Credentials::MetadataServer {
            base_url: base_url.to_string(),
        }))
    }

    /// Every request the fake server received, as raw text (head and body).
    pub(crate) type Seen = Arc<Mutex<Vec<String>>>;

    /// Serves `body` with `status` to every request until the test ends. Returns the base URL.
    pub(crate) async fn fake_server(status: u16, body: &str) -> (String, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Arc::default();
        let body = body.to_string();
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Read the head, then as much body as Content-Length says.
                loop {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let length = text[..head_end]
                            .lines()
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                k.eq_ignore_ascii_case("content-length")
                                    .then(|| v.trim().parse::<usize>().ok())?
                            })
                            .unwrap_or(0);
                        if buf.len() >= head_end + 4 + length {
                            break;
                        }
                    }
                }
                log.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf).to_string());
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        (url, seen)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    const TOKEN_JSON: &str =
        r#"{"access_token":"ya29.test-token","expires_in":3599,"token_type":"Bearer"}"#;

    #[tokio::test]
    async fn metadata_tokens_are_fetched_with_the_flavor_header_and_cached() {
        let (url, seen) = fake_server(200, TOKEN_JSON).await;
        let source = metadata_source(&url);

        assert_eq!(source.token().await.unwrap(), "ya29.test-token");
        assert_eq!(source.token().await.unwrap(), "ya29.test-token");

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "second call should use the cache");
        assert!(seen[0].starts_with(&format!("GET {METADATA_TOKEN_PATH} ")));
        assert!(seen[0].to_lowercase().contains("metadata-flavor: google"));
    }

    #[tokio::test]
    async fn tokens_close_to_expiry_are_fetched_again() {
        // Expires inside the refresh margin, so it's never served from the cache.
        let (url, seen) = fake_server(200, r#"{"access_token":"short","expires_in":60}"#).await;
        let source = metadata_source(&url);
        source.token().await.unwrap();
        source.token().await.unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn authorized_user_credentials_refresh_through_the_oauth_endpoint() {
        let (url, seen) = fake_server(200, TOKEN_JSON).await;
        let file = r#"{"type":"authorized_user","client_id":"id.apps","client_secret":"s3cret",
                       "refresh_token":"1//refresh/token"}"#;
        let credentials = parse_credentials_file(file, &format!("{url}/token")).unwrap();
        let source = GoogleTokenSource::new(Some(credentials));

        assert_eq!(source.token().await.unwrap(), "ya29.test-token");
        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("POST /token "));
        assert!(seen[0].ends_with(
            "grant_type=refresh_token&client_id=id.apps&client_secret=s3cret\
             &refresh_token=1%2F%2Frefresh%2Ftoken"
        ));
    }

    #[tokio::test]
    async fn a_rejected_refresh_is_an_auth_error() {
        let (url, _) = fake_server(400, r#"{"error":"invalid_grant"}"#).await;
        let credentials = parse_credentials_file(
            r#"{"type":"authorized_user","client_id":"a","client_secret":"b","refresh_token":"c"}"#,
            &url,
        )
        .unwrap();
        let err = GoogleTokenSource::new(Some(credentials))
            .token()
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::Auth), "{err:?}");
    }

    #[tokio::test]
    async fn an_unreachable_metadata_server_explains_how_to_log_in() {
        // Nothing listens on port 9 (discard) on loopback.
        let err = metadata_source("http://127.0.0.1:9")
            .token()
            .await
            .unwrap_err();
        assert!(
            matches!(&err, LlmError::Config(m) if m.contains("gcloud auth application-default login")),
            "{err:?}"
        );
    }

    /// Talks to Google for real, so it's skipped by default (tests stay offline, CLAUDE.md).
    /// `cargo test -p lifeagent-llm -- --ignored live_` after
    /// `gcloud auth application-default login`. Never prints the token.
    #[tokio::test]
    #[ignore = "needs Google credentials and network"]
    async fn live_token_from_application_default_credentials() {
        let source = GoogleTokenSource::new(None);
        let token = source.token().await.unwrap();
        assert!(token.starts_with("ya29."), "unexpected token format");
        assert_eq!(
            source.token().await.unwrap(),
            token,
            "second call is cached"
        );
    }

    #[test]
    fn service_account_key_files_are_refused() {
        let err =
            parse_credentials_file(r#"{"type":"service_account"}"#, OAUTH_TOKEN_URL).unwrap_err();
        assert!(matches!(err, LlmError::Config(m) if m.contains("service_account")));
    }

    #[test]
    fn form_values_are_percent_encoded() {
        assert_eq!(form_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(form_encode("1//x y+z"), "1%2F%2Fx%20y%2Bz");
    }
}
