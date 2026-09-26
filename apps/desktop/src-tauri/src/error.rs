//! `CommandError`: the error type every Tauri command returns, per
//! `docs/PHASE1_SPEC.md` §10.2 (`CommandError { code, message }`, message already in zh-CN).
//!
//! `code` always comes from `CoreError::code()`/`LlmError::code()` (`crates/core`,
//! `crates/llm`), the same classification the scheduler attaches to a scheduled
//! `AppEvent::PlanFailed` (`notify.rs`), so the two surfaces can't drift on what counts as, say,
//! an auth failure. `CoreError::Validation`'s free-form English detail (db/format-invariant
//! failures, not something a user's own input caused) still gets one generic zh-CN message;
//! `CoreError::InvalidInput`'s named `ValidationRule`s each get their own, precise wording
//! (`invalid_input_message`), per `docs/PHASE1_SPEC.md` §14 M8: "Error messages in zh-CN for
//! every `CommandError`".

use lifeagent_core::error::{CoreError, ValidationRule};
use lifeagent_core::plugin::PluginError;
use lifeagent_core::time::format_hhmm;
use lifeagent_llm::error::LlmError;
use lifeagent_plugin_diet::import::FoodDataError;
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
        let code = err.code();
        match err {
            CoreError::NotConfigured => {
                CommandError::new(code, "尚未完成模型配置，请先在设置中完成配置。")
            }
            CoreError::Validation(msg) => {
                tracing::debug!(error = %msg, "command validation error");
                CommandError::new(code, "输入内容有误，请检查后重试。")
            }
            CoreError::InvalidInput(rule) => {
                tracing::debug!(error = %rule, "command validation error");
                CommandError::new(code, invalid_input_message(rule))
            }
            CoreError::NotFound(msg) => {
                tracing::debug!(error = %msg, "command not-found error");
                CommandError::new(code, "未找到对应的数据。")
            }
            CoreError::NothingToPlan => {
                CommandError::new(code, "请先完善个人档案并记录一次体重，再生成计划。")
            }
            CoreError::Llm(e) => e.into(),
            other => {
                tracing::error!(error = %other, "internal command error");
                CommandError::new(code, "发生内部错误，请重试。")
            }
        }
    }
}

/// One zh-CN sentence per `ValidationRule`, quoting the value the user actually entered and the
/// bound it broke — the "which field, which bound" detail `docs/DECISIONS.md`'s M7.1-step-1
/// entry deferred to M8, once the UI forms existed to make it actionable.
fn invalid_input_message(rule: ValidationRule) -> String {
    match rule {
        ValidationRule::HeightOutOfRange { got } => {
            format!("身高需在 120–230 厘米之间，当前填写的是 {got:.1} 厘米。")
        }
        ValidationRule::AgeOutOfRange { got, .. } => {
            format!("年龄需在 16–100 岁之间，当前的出生日期对应 {got} 岁。")
        }
        ValidationRule::MealsPerDayOutOfRange { got } => {
            format!("每日餐数需在 3–5 之间，当前填写的是 {got}。")
        }
        ValidationRule::GoalRateOutOfRange { got } => {
            format!("目标速度需在 0–1.0 千克/周之间，当前填写的是 {got:.2} 千克/周。")
        }
        ValidationRule::GoalTargetOutOfRange { got } => {
            format!("目标体重需在 30–300 千克之间，当前填写的是 {got:.1} 千克。")
        }
        ValidationRule::WeightOutOfRange { got } => {
            format!("体重需在 30–300 千克之间，当前填写的是 {got:.1} 千克。")
        }
        ValidationRule::NoteEmpty => "备忘内容不能为空。".to_string(),
        ValidationRule::NoteTooLong { got, max } => {
            format!("备忘内容最多 {max} 字，当前是 {got} 字。")
        }
        ValidationRule::PlanTimeBeforeDayBoundary {
            plan_time,
            day_boundary,
        } => format!(
            "生成时间（{}）不能早于日界时间（{}），请调整其中一项。",
            format_hhmm(plan_time),
            format_hhmm(day_boundary)
        ),
        ValidationRule::InstructionEmpty => "调整要求不能为空，请描述需要怎么调整。".to_string(),
    }
}

/// Source-management commands (`docs/NUTRITION_SOURCES.md` §7.1, §8.2) call into
/// `lifeagent_plugin_diet::sources`, which returns `PluginError` — the same type diet tool calls
/// return to the conversation model. Its messages are meant for that audience (English, not
/// user-facing), so — like `CoreError::Validation` above — only the generic zh-CN text crosses
/// the command boundary; the detail is logged instead.
impl From<PluginError> for CommandError {
    fn from(err: PluginError) -> Self {
        match err {
            PluginError::InvalidArgs(msg) => {
                tracing::debug!(error = %msg, "command validation error");
                CommandError::new("invalid_input", "输入内容有误，请检查后重试。")
            }
            PluginError::NotFound(msg) => {
                tracing::debug!(error = %msg, "command not-found error");
                CommandError::new("not_found", "未找到对应的数据。")
            }
            other => {
                tracing::error!(error = %other, "internal command error");
                CommandError::new("internal", "发生内部错误，请重试。")
            }
        }
    }
}

/// Batch-import commands (`docs/NUTRITION_SOURCES.md` §6, §8.2). Unlike `PluginError` above,
/// `FoodDataError::Invalid`/`NotFound` messages are already zh-CN, built for exactly this
/// audience (§6.2), so they pass through unchanged rather than being replaced with fixed text.
impl From<FoodDataError> for CommandError {
    fn from(err: FoodDataError) -> Self {
        match err {
            FoodDataError::Invalid(msg) => CommandError::new("invalid_input", msg),
            FoodDataError::NotFound(msg) => CommandError::new("not_found", msg),
            FoodDataError::Plugin(e) => {
                tracing::error!(error = %e, "internal food data error");
                CommandError::new("internal", "发生内部错误，请重试。")
            }
        }
    }
}

impl From<LlmError> for CommandError {
    fn from(err: LlmError) -> Self {
        let code = err.code();
        match err {
            LlmError::Auth => CommandError::new(code, "模型鉴权失败，请检查 API Key 是否正确。"),
            LlmError::Config(msg) => {
                tracing::debug!(error = %msg, "llm config error");
                CommandError::new(code, "模型尚未配置，请先在设置中完成配置。")
            }
            LlmError::RateLimited => CommandError::new(code, "请求过于频繁，请稍后再试。"),
            LlmError::Timeout => CommandError::new(code, "请求超时，请重试。"),
            // These three can carry a provider's raw response text (`Status.body_excerpt`) or a
            // `reqwest`/`serde_json` error message, which may itself quote a fragment of the
            // request that triggered it (e.g. a provider echoing back an offending value from a
            // prompt). That detail stays at DEBUG only, matching `CoreError::Validation`/
            // `NotFound` above; the ERROR line an operator sees carries just the status code —
            // enough to diagnose a provider outage or a schema mismatch without risking personal
            // data at INFO or above (`CLAUDE.md` §"Logging").
            LlmError::Http(msg) => {
                tracing::debug!(error = %msg, "llm http error");
                tracing::error!("llm request failed to reach the provider");
                CommandError::new(code, "模型服务出现问题，请重试。")
            }
            LlmError::Status {
                code: status,
                body_excerpt,
            } => {
                tracing::debug!(status, body_excerpt = %body_excerpt, "llm status error");
                tracing::error!(status, "llm request returned an error status");
                CommandError::new(code, "模型服务出现问题，请重试。")
            }
            LlmError::InvalidResponse(msg) => {
                tracing::debug!(error = %msg, "llm invalid response");
                tracing::error!("llm returned an unparsable response");
                CommandError::new(code, "模型服务出现问题，请重试。")
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
    fn invalid_input_maps_to_invalid_input_with_a_rule_specific_zh_cn_message() {
        let err: CommandError =
            CoreError::InvalidInput(ValidationRule::HeightOutOfRange { got: 119.9 }).into();
        assert_eq!(err.code, "invalid_input");
        assert_eq!(
            err.message,
            "身高需在 120–230 厘米之间，当前填写的是 119.9 厘米。"
        );
    }

    #[test]
    fn invalid_input_messages_are_distinct_per_rule() {
        use chrono::NaiveTime;

        let cases = [
            ValidationRule::AgeOutOfRange {
                got: 15,
                birth_date: chrono::NaiveDate::from_ymd_opt(2011, 1, 1).unwrap(),
            },
            ValidationRule::MealsPerDayOutOfRange { got: 2 },
            ValidationRule::GoalRateOutOfRange { got: 1.5 },
            ValidationRule::GoalTargetOutOfRange { got: 20.0 },
            ValidationRule::WeightOutOfRange { got: 20.0 },
            ValidationRule::NoteEmpty,
            ValidationRule::NoteTooLong { got: 350, max: 300 },
            ValidationRule::PlanTimeBeforeDayBoundary {
                plan_time: NaiveTime::from_hms_opt(3, 0, 0).unwrap(),
                day_boundary: NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            },
            ValidationRule::InstructionEmpty,
        ];

        let messages: Vec<String> = cases
            .into_iter()
            .map(|rule| {
                let err: CommandError = CoreError::InvalidInput(rule).into();
                assert_eq!(err.code, "invalid_input");
                err.message
            })
            .collect();

        let unique: std::collections::HashSet<&String> = messages.iter().collect();
        assert_eq!(unique.len(), messages.len(), "messages: {messages:?}");
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
