//! Languages the app talks to the user in.
//!
//! The web UI's locale is stored per account as the `language` setting, so prompts built
//! without a request in hand (the scheduled plan) still use it. The serialized names match the
//! UI's locale codes (`apps/web/src/i18n`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// The UI's default locale, so an account that never synced one gets the same language
    /// the UI showed it.
    #[default]
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-CN")]
    ZhCn,
}

impl Language {
    /// How prompts refer to the language, e.g. the planner's `{user_language}` (§9.4).
    pub fn prompt_name(&self) -> &'static str {
        match self {
            Language::Ja => "Japanese",
            Language::En => "English",
            Language::ZhCn => "Simplified Chinese",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_ui_locale_codes() {
        for (lang, code) in [
            (Language::Ja, "\"ja\""),
            (Language::En, "\"en\""),
            (Language::ZhCn, "\"zh-CN\""),
        ] {
            assert_eq!(serde_json::to_string(&lang).unwrap(), code);
            assert_eq!(serde_json::from_str::<Language>(code).unwrap(), lang);
        }
        assert!(serde_json::from_str::<Language>("\"fr\"").is_err());
    }
}
