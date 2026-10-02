//! The UI language, mirrored into the shell for the text it draws itself: tray menu labels,
//! the quick-entry window title and notifications. The webview owns the choice (it is stored
//! in the browser's `localStorage`, `apps/desktop/ui/src/i18n`) and reports it with the
//! `set_locale` command on every load; until then the default (Japanese) applies.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    Ja,
    En,
    ZhCn,
}

impl Locale {
    /// The webview's locale tag (`"ja"`, `"en"`, `"zh-CN"`); unknown tags are `None`.
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "ja" => Some(Locale::Ja),
            "en" => Some(Locale::En),
            "zh-CN" => Some(Locale::ZhCn),
            _ => None,
        }
    }

    fn index(self) -> u8 {
        match self {
            Locale::Ja => 0,
            Locale::En => 1,
            Locale::ZhCn => 2,
        }
    }

    fn from_index(i: u8) -> Self {
        match i {
            1 => Locale::En,
            2 => Locale::ZhCn,
            _ => Locale::Ja,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Locale {
    Locale::from_index(CURRENT.load(Ordering::Relaxed))
}

pub fn set(locale: Locale) {
    CURRENT.store(locale.index(), Ordering::Relaxed);
}

/// The shell's own strings in one language.
pub struct Texts {
    pub tray_open: &'static str,
    pub tray_quick_entry: &'static str,
    pub tray_generate_plan: &'static str,
    pub tray_quit: &'static str,
    pub quick_entry_title: &'static str,
    pub plan_ready_title: &'static str,
    pub plan_failed_title: &'static str,
    plan_failed_not_configured: &'static str,
    plan_failed_auth: &'static str,
    plan_failed_nothing_to_plan: &'static str,
    plan_failed_llm: &'static str,
    plan_failed_other: &'static str,
}

impl Texts {
    /// The notification body for a failed plan, chosen from its error `code`
    /// (`CoreError::code()`/`CommandError::code`).
    pub fn plan_failed_body(&self, code: &str) -> &'static str {
        match code {
            "not_configured" => self.plan_failed_not_configured,
            "auth" => self.plan_failed_auth,
            "invalid_input" => self.plan_failed_nothing_to_plan,
            "llm" => self.plan_failed_llm,
            _ => self.plan_failed_other,
        }
    }
}

const JA: Texts = Texts {
    tray_open: "開く",
    tray_quick_entry: "クイック記録",
    tray_generate_plan: "明日のプランを作成",
    tray_quit: "終了",
    quick_entry_title: "LifeAgent クイック記録",
    plan_ready_title: "明日のプランができました",
    plan_failed_title: "明日のプランを作成できませんでした",
    plan_failed_not_configured: "モデルが設定されていません。設定でモデルを設定してから再試行してください。",
    plan_failed_auth: "モデルの認証に失敗しました。設定で API Key が正しいか確認してください。",
    plan_failed_nothing_to_plan: "プロフィールを入力し、体重を一度記録してからプランを作成してください。",
    plan_failed_llm: "モデルのサービスが一時的に利用できません。しばらくしてから「今日」ページで再試行してください。",
    plan_failed_other: "明日のプランを作成できませんでした。しばらくしてから「今日」ページで再試行してください。",
};

const EN: Texts = Texts {
    tray_open: "Open",
    tray_quick_entry: "Quick log",
    tray_generate_plan: "Create tomorrow's plan",
    tray_quit: "Quit",
    quick_entry_title: "LifeAgent quick log",
    plan_ready_title: "Tomorrow's plan is ready",
    plan_failed_title: "Couldn't create tomorrow's plan",
    plan_failed_not_configured: "No model is set up yet. Set one up in Settings, then try again.",
    plan_failed_auth: "The model rejected the API key. Check it in Settings.",
    plan_failed_nothing_to_plan:
        "Complete your profile and log your weight once, then create a plan.",
    plan_failed_llm:
        "The model service is unavailable right now. Try again later from the Today page.",
    plan_failed_other: "Couldn't create tomorrow's plan. Try again later from the Today page.",
};

const ZH_CN: Texts = Texts {
    tray_open: "打开",
    tray_quick_entry: "快速记录",
    tray_generate_plan: "生成明日计划",
    tray_quit: "退出",
    quick_entry_title: "LifeAgent 快速记录",
    plan_ready_title: "明日计划已生成",
    plan_failed_title: "明日计划生成失败",
    plan_failed_not_configured: "尚未完成模型配置，请先在设置中完成配置后重试。",
    plan_failed_auth: "模型鉴权失败，请在设置中检查 API Key 是否正确。",
    plan_failed_nothing_to_plan: "请先完善个人档案并记录一次体重，再生成计划。",
    plan_failed_llm: "模型服务暂时不可用，请稍后在\"今日\"页重试。",
    plan_failed_other: "生成明日计划失败，请稍后在\"今日\"页重试。",
};

pub fn texts(locale: Locale) -> &'static Texts {
    match locale {
        Locale::Ja => &JA,
        Locale::En => &EN,
        Locale::ZhCn => &ZH_CN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_round_trip_and_unknown_tags_are_rejected() {
        assert_eq!(Locale::from_tag("ja"), Some(Locale::Ja));
        assert_eq!(Locale::from_tag("en"), Some(Locale::En));
        assert_eq!(Locale::from_tag("zh-CN"), Some(Locale::ZhCn));
        assert_eq!(Locale::from_tag("fr"), None);
        for l in [Locale::Ja, Locale::En, Locale::ZhCn] {
            assert_eq!(Locale::from_index(l.index()), l);
        }
    }

    #[test]
    fn plan_failed_body_is_specific_per_code() {
        let t = texts(Locale::ZhCn);
        assert_eq!(
            t.plan_failed_body("not_configured"),
            "尚未完成模型配置，请先在设置中完成配置后重试。"
        );
        assert_eq!(
            t.plan_failed_body("auth"),
            "模型鉴权失败，请在设置中检查 API Key 是否正确。"
        );
        assert_eq!(
            t.plan_failed_body("invalid_input"),
            "请先完善个人档案并记录一次体重，再生成计划。"
        );
        assert_eq!(
            t.plan_failed_body("llm"),
            "模型服务暂时不可用，请稍后在\"今日\"页重试。"
        );
        assert_eq!(
            t.plan_failed_body("internal"),
            "生成明日计划失败，请稍后在\"今日\"页重试。"
        );
        let ja = texts(Locale::Ja);
        let bodies: std::collections::HashSet<&str> =
            ["not_configured", "auth", "invalid_input", "llm", "internal"]
                .into_iter()
                .map(|c| ja.plan_failed_body(c))
                .collect();
        assert_eq!(bodies.len(), 5);
    }
}
