import { useEffect, useState } from "react";
import { getAppState } from "./api/commands";
import { Chat } from "./chat/Chat";
import { Sidebar } from "./components/Sidebar";
import { zhCN } from "./i18n/zh-CN";
import { Onboarding } from "./onboarding/Onboarding";
import { Settings } from "./settings/Settings";
import { Today } from "./today/Today";

/**
 * Thin view layer: all state comes from Tauri commands, no business logic here
 * (see CLAUDE.md). First run shows onboarding; afterwards, the main shell with its 今日 / 对话 /
 * 设置 tabs.
 */
type Screen = "loading" | "onboarding" | "main";
export type Tab = "today" | "chat" | "settings";

export default function App() {
  const [screen, setScreen] = useState<Screen>("loading");

  useEffect(() => {
    let cancelled = false;
    getAppState()
      .then((state) => {
        if (!cancelled) setScreen(state.onboarding_complete ? "main" : "onboarding");
      })
      .catch(() => {
        // No way to reach Settings from here if `get_app_state` itself fails, but onboarding
        // is the safer default screen than an indefinite loading spinner.
        if (!cancelled) setScreen("onboarding");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (screen === "onboarding") {
    return (
      <div className="h-screen bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
        <Onboarding onComplete={() => setScreen("main")} />
      </div>
    );
  }

  if (screen === "main") {
    return <MainShell />;
  }

  return (
    <div className="flex h-screen items-center justify-center bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
      <p className="text-lg">{zhCN.common.loading}</p>
    </div>
  );
}

function MainShell() {
  const [tab, setTab] = useState<Tab>("today");
  const [chatDraft, setChatDraft] = useState<string | null>(null);

  function openChatWithDraft(prefillText: string) {
    setChatDraft(prefillText);
    setTab("chat");
  }

  return (
    <div className="flex h-screen bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
      <Sidebar tab={tab} onChange={setTab} />
      <main className="flex-1 overflow-hidden">
        {tab === "today" ? <Today onAdjustPlan={openChatWithDraft} /> : null}
        {tab === "chat" ? (
          <Chat
            draftText={chatDraft}
            onDraftConsumed={() => setChatDraft(null)}
            onGoToSettings={() => setTab("settings")}
          />
        ) : null}
        {tab === "settings" ? <Settings /> : null}
      </main>
    </div>
  );
}
