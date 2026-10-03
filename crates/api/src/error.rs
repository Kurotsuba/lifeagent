//! `CommandError`: the error type every command returns, per `docs/PHASE1_SPEC.md` §10.2
//! (`CommandError { code, message }`). The web server sends it as the JSON body of an
//! error response.
//!
//! `code` always comes from `CoreError::code()`/`LlmError::code()` (`crates/core`,
//! `crates/llm`), the same classification the scheduler attaches to a scheduled
//! `AppEvent::PlanFailed`, so the two surfaces can't drift on what
//! counts as, say, an auth failure. `detail` says precisely what went wrong, as data: the UI
//! words it in the user's language (`apps/web/src/i18n`, `errors`). `message` is the
//! same thing in zh-CN, for the CLI and for any client that doesn't know a `detail` kind.
//! `CoreError::Validation`'s free-form English detail (db/format-invariant failures, not
//! something a user's own input caused) still gets one generic message; `CoreError::InvalidInput`'s
//! named `ValidationRule`s each get their own, quoting the value and the bound it broke.

use lifeagent_core::error::{CoreError, ValidationRule};
use lifeagent_core::plugin::PluginError;
use lifeagent_core::time::format_hhmm;
use lifeagent_llm::error::LlmError;
use lifeagent_plugin_diet::import::{FoodDataError, ImportMessage};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    pub detail: ErrorDetail,
}

impl CommandError {
    pub fn new(code: &str, detail: ErrorDetail) -> Self {
        Self {
            code: code.to_string(),
            message: detail.to_string(),
            detail,
        }
    }
}

/// What went wrong, one variant per message the user can see. Serialized as
/// `{ "kind": "<snake_case variant>", ...fields }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ErrorDetail {
    NotConfigured,
    InvalidInput,
    NotFound,
    NothingToPlan,
    Internal,
    // ---- Model provider (`LlmError`) ----
    LlmAuth,
    LlmRateLimited,
    LlmTimeout,
    LlmUnavailable,
    // ---- `ValidationRule` ----
    HeightOutOfRange {
        got: f64,
    },
    AgeOutOfRange {
        got: i32,
    },
    MealsPerDayOutOfRange {
        got: u8,
    },
    GoalRateOutOfRange {
        got: f64,
    },
    GoalTargetOutOfRange {
        got: f64,
    },
    WeightOutOfRange {
        got: f64,
    },
    NoteEmpty,
    NoteTooLong {
        got: usize,
        max: usize,
    },
    /// Both as `HH:MM`.
    PlanTimeBeforeDayBoundary {
        plan_time: String,
        day_boundary: String,
    },
    InstructionEmpty,
    /// Both as `YYYY-MM-DD`.
    PreferenceDatesReversed {
        valid_from: String,
        valid_until: String,
    },
    // ---- Food data import (`FoodDataError`) ----
    FoodData {
        detail: ImportMessage,
    },
    // ---- Web server (`crates/server`): accounts and routing ----
    Unauthorized,
    InvalidUsername,
    InvalidPassword,
    TooManyAttempts,
    InvalidCredentials,
    UsernameTaken,
    BadRequest,
    UnknownEndpoint,
}

impl std::fmt::Display for ErrorDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ErrorDetail::*;
        match self {
            NotConfigured => write!(f, "尚未完成模型配置，请先在设置中完成配置。"),
            InvalidInput => write!(f, "输入内容有误，请检查后重试。"),
            NotFound => write!(f, "未找到对应的数据。"),
            NothingToPlan => write!(f, "请先完善个人档案并记录一次体重，再生成计划。"),
            Internal => write!(f, "发生内部错误，请重试。"),
            LlmAuth => write!(f, "模型鉴权失败，请检查 API Key 是否正确。"),
            LlmRateLimited => write!(f, "请求过于频繁，请稍后再试。"),
            LlmTimeout => write!(f, "请求超时，请重试。"),
            LlmUnavailable => write!(f, "模型服务出现问题，请重试。"),
            HeightOutOfRange { got } => {
                write!(f, "身高需在 120–230 厘米之间，当前填写的是 {got:.1} 厘米。")
            }
            AgeOutOfRange { got } => {
                write!(f, "年龄需在 16–100 岁之间，当前的出生日期对应 {got} 岁。")
            }
            MealsPerDayOutOfRange { got } => {
                write!(f, "每日餐数需在 3–5 之间，当前填写的是 {got}。")
            }
            GoalRateOutOfRange { got } => write!(
                f,
                "目标速度需在 0–1.0 千克/周之间，当前填写的是 {got:.2} 千克/周。"
            ),
            GoalTargetOutOfRange { got } => {
                write!(
                    f,
                    "目标体重需在 30–300 千克之间，当前填写的是 {got:.1} 千克。"
                )
            }
            WeightOutOfRange { got } => {
                write!(f, "体重需在 30–300 千克之间，当前填写的是 {got:.1} 千克。")
            }
            NoteEmpty => write!(f, "内容不能为空。"),
            NoteTooLong { got, max } => write!(f, "内容最多 {max} 字，当前是 {got} 字。"),
            PlanTimeBeforeDayBoundary {
                plan_time,
                day_boundary,
            } => write!(
                f,
                "生成时间（{plan_time}）不能早于日界时间（{day_boundary}），请调整其中一项。"
            ),
            InstructionEmpty => write!(f, "调整要求不能为空，请描述需要怎么调整。"),
            PreferenceDatesReversed {
                valid_from,
                valid_until,
            } => write!(
                f,
                "开始日期（{valid_from}）不能晚于结束日期（{valid_until}）。"
            ),
            FoodData { detail } => write!(f, "{detail}"),
            Unauthorized => write!(f, "请先登录。"),
            InvalidUsername => write!(f, "用户名需为 3–32 位英文字母、数字或 _ - . 组成。"),
            InvalidPassword => write!(f, "密码需为 8–128 位。"),
            TooManyAttempts => write!(f, "尝试次数过多，请稍后再试。"),
            InvalidCredentials => write!(f, "用户名或密码错误。"),
            UsernameTaken => write!(f, "该用户名已被使用。"),
            BadRequest => write!(f, "请求参数有误。"),
            UnknownEndpoint => write!(f, "未找到对应的接口。"),
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
            CoreError::NotConfigured => CommandError::new(code, ErrorDetail::NotConfigured),
            CoreError::Validation(msg) => {
                tracing::debug!(error = %msg, "command validation error");
                CommandError::new(code, ErrorDetail::InvalidInput)
            }
            CoreError::InvalidInput(rule) => {
                tracing::debug!(error = %rule, "command validation error");
                CommandError::new(code, invalid_input_detail(rule))
            }
            CoreError::NotFound(msg) => {
                tracing::debug!(error = %msg, "command not-found error");
                CommandError::new(code, ErrorDetail::NotFound)
            }
            CoreError::NothingToPlan => CommandError::new(code, ErrorDetail::NothingToPlan),
            CoreError::Llm(e) => e.into(),
            other => {
                tracing::error!(error = %other, "internal command error");
                CommandError::new(code, ErrorDetail::Internal)
            }
        }
    }
}

/// One detail per `ValidationRule`, carrying the value the user actually entered.
fn invalid_input_detail(rule: ValidationRule) -> ErrorDetail {
    match rule {
        ValidationRule::HeightOutOfRange { got } => ErrorDetail::HeightOutOfRange { got },
        ValidationRule::AgeOutOfRange { got, .. } => ErrorDetail::AgeOutOfRange { got },
        ValidationRule::MealsPerDayOutOfRange { got } => ErrorDetail::MealsPerDayOutOfRange { got },
        ValidationRule::GoalRateOutOfRange { got } => ErrorDetail::GoalRateOutOfRange { got },
        ValidationRule::GoalTargetOutOfRange { got } => ErrorDetail::GoalTargetOutOfRange { got },
        ValidationRule::WeightOutOfRange { got } => ErrorDetail::WeightOutOfRange { got },
        ValidationRule::NoteEmpty => ErrorDetail::NoteEmpty,
        ValidationRule::NoteTooLong { got, max } => ErrorDetail::NoteTooLong { got, max },
        ValidationRule::PlanTimeBeforeDayBoundary {
            plan_time,
            day_boundary,
        } => ErrorDetail::PlanTimeBeforeDayBoundary {
            plan_time: format_hhmm(plan_time),
            day_boundary: format_hhmm(day_boundary),
        },
        ValidationRule::InstructionEmpty => ErrorDetail::InstructionEmpty,
        ValidationRule::PreferenceDatesReversed {
            valid_from,
            valid_until,
        } => ErrorDetail::PreferenceDatesReversed {
            valid_from: valid_from.to_string(),
            valid_until: valid_until.to_string(),
        },
    }
}

/// Source-management commands (`docs/NUTRITION_SOURCES.md` §7.1, §8.2) call into
/// `lifeagent_plugin_diet::sources`, which returns `PluginError` — the same type diet tool calls
/// return to the conversation model. Its messages are meant for that audience (English, not
/// user-facing), so — like `CoreError::Validation` above — only a generic detail crosses the
/// command boundary; the specifics are logged instead.
impl From<PluginError> for CommandError {
    fn from(err: PluginError) -> Self {
        match err {
            PluginError::InvalidArgs(msg) => {
                tracing::debug!(error = %msg, "command validation error");
                CommandError::new("invalid_input", ErrorDetail::InvalidInput)
            }
            PluginError::NotFound(msg) => {
                tracing::debug!(error = %msg, "command not-found error");
                CommandError::new("not_found", ErrorDetail::NotFound)
            }
            other => {
                tracing::error!(error = %other, "internal command error");
                CommandError::new("internal", ErrorDetail::Internal)
            }
        }
    }
}

/// Batch-import commands (`docs/NUTRITION_SOURCES.md` §6, §8.2). Unlike `PluginError` above,
/// `FoodDataError::Invalid`/`NotFound` carry an `ImportMessage` built for the user (§6.2), so it
/// is passed through rather than replaced with a generic detail.
impl From<FoodDataError> for CommandError {
    fn from(err: FoodDataError) -> Self {
        match err {
            FoodDataError::Invalid(detail) => {
                CommandError::new("invalid_input", ErrorDetail::FoodData { detail })
            }
            FoodDataError::NotFound(detail) => {
                CommandError::new("not_found", ErrorDetail::FoodData { detail })
            }
            FoodDataError::Plugin(e) => {
                tracing::error!(error = %e, "internal food data error");
                CommandError::new("internal", ErrorDetail::Internal)
            }
        }
    }
}

impl From<LlmError> for CommandError {
    fn from(err: LlmError) -> Self {
        let code = err.code();
        match err {
            LlmError::Auth => CommandError::new(code, ErrorDetail::LlmAuth),
            LlmError::Config(msg) => {
                tracing::debug!(error = %msg, "llm config error");
                CommandError::new(code, ErrorDetail::NotConfigured)
            }
            LlmError::RateLimited => CommandError::new(code, ErrorDetail::LlmRateLimited),
            LlmError::Timeout => CommandError::new(code, ErrorDetail::LlmTimeout),
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
                CommandError::new(code, ErrorDetail::LlmUnavailable)
            }
            LlmError::Status {
                code: status,
                body_excerpt,
            } => {
                tracing::debug!(status, body_excerpt = %body_excerpt, "llm status error");
                tracing::error!(status, "llm request returned an error status");
                CommandError::new(code, ErrorDetail::LlmUnavailable)
            }
            LlmError::InvalidResponse(msg) => {
                tracing::debug!(error = %msg, "llm invalid response");
                tracing::error!("llm returned an unparsable response");
                CommandError::new(code, ErrorDetail::LlmUnavailable)
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
    fn validation_maps_to_invalid_input_with_a_generic_message() {
        let err: CommandError =
            CoreError::Validation("height_cm must be within 120..=230".into()).into();
        assert_eq!(err.code, "invalid_input");
        assert_eq!(err.detail, ErrorDetail::InvalidInput);
        assert_eq!(err.message, "输入内容有误，请检查后重试。");
    }

    #[test]
    fn invalid_input_maps_to_invalid_input_with_a_rule_specific_detail() {
        let err: CommandError =
            CoreError::InvalidInput(ValidationRule::HeightOutOfRange { got: 119.9 }).into();
        assert_eq!(err.code, "invalid_input");
        assert_eq!(err.detail, ErrorDetail::HeightOutOfRange { got: 119.9 });
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
            ValidationRule::PreferenceDatesReversed {
                valid_from: chrono::NaiveDate::from_ymd_opt(2026, 10, 9).unwrap(),
                valid_until: chrono::NaiveDate::from_ymd_opt(2026, 10, 3).unwrap(),
            },
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
    fn detail_serializes_with_a_kind_tag() {
        let err: CommandError =
            CoreError::InvalidInput(ValidationRule::NoteTooLong { got: 350, max: 300 }).into();
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["detail"]["kind"], "note_too_long");
        assert_eq!(json["detail"]["got"], 350);
        assert_eq!(json["detail"]["max"], 300);
    }

    #[test]
    fn food_data_errors_carry_the_import_message() {
        let err: CommandError = FoodDataError::NotFound(ImportMessage::ImportExpired).into();
        assert_eq!(err.code, "not_found");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["detail"]["kind"], "food_data");
        assert_eq!(json["detail"]["detail"]["kind"], "import_expired");
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
