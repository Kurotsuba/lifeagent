//! Languages the app talks to the user in.
//!
//! Phase 1 ships Simplified Chinese only. Keeping it an enum means adding a language later is
//! a new variant plus its strings, not a hunt for hard-coded "Simplified Chinese".

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    ZhCn,
}

impl Language {
    /// How prompts refer to the language, e.g. the planner's `{user_language}` (§9.4).
    pub fn prompt_name(&self) -> &'static str {
        match self {
            Language::ZhCn => "Simplified Chinese",
        }
    }
}

/// The only language in Phase 1. Replace with a setting when a second one is added.
pub const USER_LANGUAGE: Language = Language::ZhCn;
