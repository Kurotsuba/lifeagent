//! `ChatModel` provider implementations.
//!
//! Two providers land in M2 per `docs/PHASE1_SPEC.md` §6.2: an OpenAI-compatible Chat
//! Completions client (§6.2.1) and an Anthropic Messages client (§6.2.2), both behind the
//! same trait so the router (`router.rs`) never needs to know which one it is calling.

pub mod anthropic;
pub mod openai_compat;

use std::time::Duration;

use reqwest::RequestBuilder;
use tokio::time::sleep;

use crate::error::LlmError;

/// Shared HTTP policy (§6.2): "raised from 90 s because Anthropic models with adaptive
/// thinking spend thinking tokens before emitting any output."
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

const MAX_RETRIES: u32 = 2;
const RETRY_AFTER_CAP: Duration = Duration::from_secs(20);
const BODY_EXCERPT_MAX_CHARS: usize = 500;

fn backoff_for_attempt(attempt: u32) -> Duration {
    match attempt {
        0 => Duration::from_secs(1),
        _ => Duration::from_secs(4),
    }
}

/// 429, 5xx and 529 are retried; everything else (including 400, 401, 403) is not.
pub(crate) fn is_retryable_status(code: u16) -> bool {
    code == 429 || code == 529 || (500..600).contains(&code)
}

pub(crate) fn final_error_for_status(code: u16, body_excerpt: String) -> LlmError {
    match code {
        429 => LlmError::RateLimited,
        401 | 403 => LlmError::Auth,
        _ => LlmError::Status { code, body_excerpt },
    }
}

pub(crate) fn body_excerpt(body: &str) -> String {
    if body.chars().count() > BODY_EXCERPT_MAX_CHARS {
        body.chars().take(BODY_EXCERPT_MAX_CHARS).collect()
    } else {
        body.to_string()
    }
}

/// Parses a `Retry-After` header value (seconds, per HTTP semantics used by the providers we
/// target) into a capped `Duration`. Anything unparsable is ignored in favor of the default
/// backoff.
pub(crate) fn parse_retry_after(header_value: Option<&str>) -> Option<Duration> {
    header_value
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|secs| Duration::from_secs(secs).min(RETRY_AFTER_CAP))
}

/// Sends a request built fresh by `build` (so it can be retried; `reqwest::Request` is
/// consumed by `send`), applying the shared retry policy (§6.2). Returns the parsed JSON body
/// on success.
pub(crate) async fn send_with_retries(
    build: impl Fn() -> RequestBuilder,
) -> Result<serde_json::Value, LlmError> {
    let mut attempt = 0u32;
    loop {
        match build().timeout(REQUEST_TIMEOUT).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return resp
                        .json::<serde_json::Value>()
                        .await
                        .map_err(|e| LlmError::InvalidResponse(e.to_string()));
                }

                let code = status.as_u16();
                let retry_after = parse_retry_after(
                    resp.headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok()),
                );
                let body = resp.text().await.unwrap_or_default();

                if is_retryable_status(code) && attempt < MAX_RETRIES {
                    sleep(retry_after.unwrap_or_else(|| backoff_for_attempt(attempt))).await;
                    attempt += 1;
                    continue;
                }
                return Err(final_error_for_status(code, body_excerpt(&body)));
            }
            Err(e) => {
                if e.is_timeout() && attempt < MAX_RETRIES {
                    sleep(backoff_for_attempt(attempt)).await;
                    attempt += 1;
                    continue;
                }
                if e.is_timeout() {
                    return Err(LlmError::Timeout);
                }
                return Err(LlmError::Http(e.to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_statuses_are_429_5xx_and_529() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(529));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(403));
        assert!(!is_retryable_status(404));
    }

    #[test]
    fn final_error_maps_429_to_rate_limited() {
        assert!(matches!(
            final_error_for_status(429, String::new()),
            LlmError::RateLimited
        ));
    }

    #[test]
    fn final_error_maps_401_and_403_to_auth() {
        assert!(matches!(
            final_error_for_status(401, String::new()),
            LlmError::Auth
        ));
        assert!(matches!(
            final_error_for_status(403, String::new()),
            LlmError::Auth
        ));
    }

    #[test]
    fn final_error_maps_400_and_5xx_to_status() {
        assert!(matches!(
            final_error_for_status(400, "bad request".into()),
            LlmError::Status { code: 400, .. }
        ));
        assert!(matches!(
            final_error_for_status(503, "unavailable".into()),
            LlmError::Status { code: 503, .. }
        ));
    }

    #[test]
    fn body_excerpt_passes_short_bodies_through() {
        assert_eq!(body_excerpt("short"), "short");
    }

    #[test]
    fn body_excerpt_truncates_long_bodies() {
        let long = "a".repeat(BODY_EXCERPT_MAX_CHARS + 100);
        assert_eq!(body_excerpt(&long).chars().count(), BODY_EXCERPT_MAX_CHARS);
    }

    #[test]
    fn retry_after_parses_seconds_and_caps_at_20() {
        assert_eq!(parse_retry_after(Some("5")), Some(Duration::from_secs(5)));
        assert_eq!(parse_retry_after(Some("60")), Some(RETRY_AFTER_CAP));
    }

    #[test]
    fn retry_after_is_none_when_missing_or_unparsable() {
        assert_eq!(parse_retry_after(None), None);
        assert_eq!(parse_retry_after(Some("not-a-number")), None);
    }
}
