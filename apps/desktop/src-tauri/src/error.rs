//! `CommandError`: the error type every Tauri command returns, per
//! `docs/PHASE1_SPEC.md` §10.2 (`CommandError { code, message }`, message already in zh-CN).
//!
//! Per-field validation detail (e.g. "height_cm must be within 120..=230") stays on the
//! `CoreError` side, in English, for logs and tests. `invalid_input` here carries a generic
//! zh-CN message rather than that detail; giving each validation failure its own zh-CN wording
//! is M8 work (`docs/PHASE1_SPEC.md` §14: "Error messages in zh-CN for every `CommandError`").

use lifeagent_core::error::CoreError;
use lifeagent_llm::error::LlmError;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

/// A `CoreError` that reaches a command boundary is either the user's own input (surfaced as
/// `invalid_input`/`not_found`) or something this layer can't fix by itself (logged here, since
/// the message shown to the user can't carry Rust's own error detail).
impl From<CoreError> for CommandError {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::NotConfigured => {
                CommandError::new("not_configured", "尚未完成模型配置，请先在设置中完成配置。")
            }
            CoreError::Validation(msg) => {
                tracing::debug!(error = %msg, "command validation error");
                CommandError::new("invalid_input", "输入内容有误，请检查后重试。")
            }
            CoreError::NotFound(msg) => {
                tracing::debug!(error = %msg, "command not-found error");
                CommandError::new("not_found", "未找到对应的数据。")
            }
            CoreError::NothingToPlan => CommandError::new(
                "invalid_input",
                "请先完善个人档案并记录一次体重，再生成计划。",
            ),
            CoreError::Llm(e) => e.into(),
            other => {
                tracing::error!(error = %other, "internal command error");
                CommandError::new("internal", "发生内部错误，请重试。")
            }
        }
    }
}

impl From<LlmError> for CommandError {
    fn from(err: LlmError) -> Self {
        match err {
            LlmError::Auth => CommandError::new("auth", "模型鉴权失败，请检查 API Key 是否正确。"),
            LlmError::Config(msg) => {
                tracing::debug!(error = %msg, "llm config error");
                CommandError::new("not_configured", "模型尚未配置，请先在设置中完成配置。")
            }
            LlmError::RateLimited => CommandError::new("llm", "请求过于频繁，请稍后再试。"),
            LlmError::Timeout => CommandError::new("llm", "请求超时，请重试。"),
            other => {
                tracing::error!(error = %other, "llm error");
                CommandError::new("llm", "模型服务出现问题，请重试。")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_configured_maps_to_not_configured_code() {
        let err: CommandError = CoreError::NotConfigured.into();
        assert_eq!(err.code, "not_configured");
    }

    #[test]
    fn validation_maps_to_invalid_input_with_a_generic_zh_cn_message() {
        let err: CommandError =
            CoreError::Validation("height_cm must be within 120..=230".into()).into();
        assert_eq!(err.code, "invalid_input");
        assert_eq!(err.message, "输入内容有误，请检查后重试。");
    }

    #[test]
    fn not_found_maps_to_not_found_code() {
        let err: CommandError = CoreError::NotFound("plan for 2026-09-25".into()).into();
        assert_eq!(err.code, "not_found");
    }

    #[test]
    fn nothing_to_plan_maps_to_invalid_input() {
        let err: CommandError = CoreError::NothingToPlan.into();
        assert_eq!(err.code, "invalid_input");
    }

    #[test]
    fn db_error_maps_to_internal() {
        let err: CommandError = CoreError::Plugin("boom".into()).into();
        assert_eq!(err.code, "internal");
    }

    #[test]
    fn llm_auth_maps_to_auth_code() {
        let err: CommandError = CoreError::Llm(LlmError::Auth).into();
        assert_eq!(err.code, "auth");
    }

    #[test]
    fn llm_config_maps_to_not_configured() {
        let err: CommandError = CoreError::Llm(LlmError::Config("no key".into())).into();
        assert_eq!(err.code, "not_configured");
    }

    #[test]
    fn llm_rate_limited_maps_to_llm_code() {
        let err: CommandError = CoreError::Llm(LlmError::RateLimited).into();
        assert_eq!(err.code, "llm");
    }

    #[test]
    fn llm_other_variants_map_to_llm_code() {
        let err: CommandError = CoreError::Llm(LlmError::Timeout).into();
        assert_eq!(err.code, "llm");
        let err: CommandError = CoreError::Llm(LlmError::Http("connection refused".into())).into();
        assert_eq!(err.code, "llm");
        let err: CommandError = CoreError::Llm(LlmError::Status {
            code: 500,
            body_excerpt: "oops".into(),
        })
        .into();
        assert_eq!(err.code, "llm");
        let err: CommandError = CoreError::Llm(LlmError::InvalidResponse("bad json".into())).into();
        assert_eq!(err.code, "llm");
    }
}
