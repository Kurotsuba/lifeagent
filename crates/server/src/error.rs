//! `ApiError`: a `CommandError` (`lifeagent-api`) plus the HTTP status
//! it is sent with. The body is always `{ "code": ..., "message": ..., "detail": ... }`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use lifeagent_api::{CommandError, ErrorDetail};
use lifeagent_core::error::CoreError;

#[derive(Debug)]
pub struct ApiError(pub CommandError);

impl ApiError {
    pub fn new(code: &str, detail: ErrorDetail) -> Self {
        Self(CommandError::new(code, detail))
    }

    pub fn unauthorized() -> Self {
        Self::new("unauthorized", ErrorDetail::Unauthorized)
    }

    pub fn invalid(detail: ErrorDetail) -> Self {
        Self::new("invalid_input", detail)
    }

    /// Anything the caller can't fix: the detail is logged, never sent.
    pub fn internal(detail: impl std::fmt::Display) -> Self {
        tracing::error!(error = %detail, "internal server error");
        Self::new("internal", ErrorDetail::Internal)
    }

    fn status(&self) -> StatusCode {
        match self.0.code.as_str() {
            "invalid_input" => StatusCode::BAD_REQUEST,
            "unauthorized" | "invalid_credentials" => StatusCode::UNAUTHORIZED,
            "forbidden" => StatusCode::FORBIDDEN,
            "not_found" => StatusCode::NOT_FOUND,
            "username_taken" => StatusCode::CONFLICT,
            "payload_too_large" => StatusCode::PAYLOAD_TOO_LARGE,
            "rate_limited" => StatusCode::TOO_MANY_REQUESTS,
            // The server's own model setup, not something the user can change.
            "not_configured" => StatusCode::SERVICE_UNAVAILABLE,
            // The model provider failed (`LlmError::code()`).
            "auth" | "llm" => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CommandError> for ApiError {
    fn from(e: CommandError) -> Self {
        Self(e)
    }
}

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        Self(e.into())
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status(), Json(self.0)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_follow_the_error_code() {
        let cases = [
            ("invalid_input", 400),
            ("unauthorized", 401),
            ("invalid_credentials", 401),
            ("forbidden", 403),
            ("not_found", 404),
            ("username_taken", 409),
            ("rate_limited", 429),
            ("not_configured", 503),
            ("llm", 502),
            ("auth", 502),
            ("internal", 500),
        ];
        for (code, status) in cases {
            assert_eq!(
                ApiError::new(code, ErrorDetail::Internal).status().as_u16(),
                status,
                "{code}"
            );
        }
    }
}
