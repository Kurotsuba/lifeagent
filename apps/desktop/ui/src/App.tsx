import { useEffect, useState } from "react";
import { me, SIGNED_OUT_EVENT } from "./api/auth";
import { getAppState } from "./api/commands";
import { isWeb } from "./api/platform";
import { SESSION_EXPIRED_EVENT } from "./api/transport";
import { isCommandError } from "./api/types";
import { AuthScreen } from "./auth/AuthScreen";
import { Chat } from "./chat/Chat";
import { Sidebar } from "./components/Sidebar";
import { strings } from "./i18n";
import { Onboarding } from "./onboarding/Onboarding";
import { Settings } from "./settings/Settings";
import { Today } from "./today/Today";

/**
 * Thin view layer: all state comes from commands, no business logic here (see CLAUDE.md).
 * In the web build a login comes first; then, on the first run, onboarding; afterwards, the
 * main shell with its 今日 / 对话 / 设置 tabs.
 */
type Screen = "loading" | "auth" | "onboarding" | "main";
export type Tab = "today" | "chat" | "settings";

export default function App() {
  const [screen, setScreen] = useState<Screen>("loading");
  const [username, setUsername] = useState<string | null>(null);
  const [authNotice, setAuthNotice] = useState<string | null>(null);
  // Bumped after signing in, to rerun the startup checks for the new session.
  const [session, setSession] = useState(0);

  useEffect(() => {
    let cancelled = false;
    async function start() {
      if (isWeb) {
        try {
          const user = await me();
          if (cancelled) return;
          setUsername(user.username);
        } catch (e) {
          if (cancelled) return;
          setAuthNotice(
            isCommandError(e) && e.code === "unauthorized" ? null : strings.common.networkError,
          );
          setScreen("auth");
          return;
        }
      }
      try {
        const state = await getAppState();
        if (!cancelled) setScreen(state.onboarding_complete ? "main" : "onboarding");
      } catch {
        // No way to reach Settings from here if `get_app_state` itself fails, but onboarding
        // is the safer default screen than an indefinite loading spinner.
        if (!cancelled) setScreen("onboarding");
      }
    }
    void start();
    return () => {
      cancelled = true;
    };
  }, [session]);

  useEffect(() => {
    if (!isWeb) return;
    const toAuth = (notice: string | null) => () => {
      setUsername(null);
      setAuthNotice(notice);
      setScreen("auth");
    };
    const onExpired = toAuth(strings.auth.sessionExpired);
    const onSignedOut = toAuth(null);
    window.addEventListener(SESSION_EXPIRED_EVENT, onExpired);
    window.addEventListener(SIGNED_OUT_EVENT, onSignedOut);
    return () => {
      window.removeEventListener(SESSION_EXPIRED_EVENT, onExpired);
      window.removeEventListener(SIGNED_OUT_EVENT, onSignedOut);
    };
  }, []);

  if (screen === "auth") {
    return (
      <div className="h-screen bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
        <AuthScreen
          notice={authNotice}
          onSignedIn={() => {
            setAuthNotice(null);
            setScreen("loading");
            setSession((s) => s + 1);
          }}
        />
      </div>
    );
  }

  if (screen === "onboarding") {
    return (
      <div className="h-screen bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
        <Onboarding onComplete={() => setScreen("main")} />
      </div>
    );
  }

  if (screen === "main") {
    return <MainShell username={username} />;
  }

  return (
    <div className="flex h-screen items-center justify-center bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
      <p className="text-lg">{strings.common.loading}</p>
    </div>
  );
}

function MainShell({ username }: { username: string | null }) {
  const [tab, setTab] = useState<Tab>("today");
  const [chatDraft, setChatDraft] = useState<string | null>(null);

  function openChatWithDraft(prefillText: string) {
    setChatDraft(prefillText);
    setTab("chat");
  }

  return (
    <div className="flex h-screen bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
      <Sidebar tab={tab} onChange={setTab} username={username} />
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
