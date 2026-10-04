import { useEffect, useState } from "react";
import { me, SIGNED_OUT_EVENT } from "./api/auth";
import { getAppState, setLanguage } from "./api/commands";
import { SESSION_EXPIRED_EVENT } from "./api/transport";
import { isCommandError } from "./api/types";
import { AuthScreen } from "./auth/AuthScreen";
import { Chat } from "./chat/Chat";
import { Guide, guideSeen } from "./components/Guide";
import { Household } from "./household/Household";
import { Sidebar } from "./components/Sidebar";
import { locale, strings } from "./i18n";
import { Nutrition } from "./nutrition/Nutrition";
import { Onboarding } from "./onboarding/Onboarding";
import { Settings } from "./settings/Settings";
import { Stock } from "./stock/Stock";
import { Today } from "./today/Today";
import { Training } from "./training/Training";

/**
 * Thin view layer: all state comes from commands, no business logic here (see CLAUDE.md).
 * A login comes first; then, on the first run, onboarding; afterwards, the
 * main shell. Its tabs, in order: Today (a read-only summary of the day), Chat, then one tab per
 * plugin (Nutrition, plus Training, Household and Stock while those optional plugins are on), and
 * Settings.
 */
type Screen = "loading" | "auth" | "onboarding" | "main";
export type Tab =
  | "today"
  | "chat"
  | "nutrition"
  | "training"
  | "household"
  | "stock"
  | "settings";

/** Tabs that belong to an optional plugin, by plugin id. */
const PLUGIN_TABS: Partial<Record<Tab, string>> = {
  training: "training",
  household: "multimember",
  stock: "stock",
};

export default function App() {
  const [screen, setScreen] = useState<Screen>("loading");
  const [username, setUsername] = useState<string | null>(null);
  const [authNotice, setAuthNotice] = useState<string | null>(null);
  // Bumped after signing in, to rerun the startup checks for the new session.
  const [session, setSession] = useState(0);

  useEffect(() => {
    let cancelled = false;
    async function start() {
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
      // The assistant replies in the UI's language, including in plans made overnight, so the
      // server keeps a copy. Switching language reloads the page, which lands back here.
      // Best effort: on failure the server keeps the previous language.
      void setLanguage(locale).catch(() => {});
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
      <div className="h-dvh bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
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
      <div className="h-dvh bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
        <Onboarding onComplete={() => setScreen("main")} />
      </div>
    );
  }

  if (screen === "main") {
    return <MainShell username={username} />;
  }

  return (
    <div className="flex h-dvh items-center justify-center bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
      <p className="text-lg">{strings.common.loading}</p>
    </div>
  );
}

function MainShell({ username }: { username: string | null }) {
  const [tab, setTab] = useState<Tab>("today");
  const [chatDraft, setChatDraft] = useState<string | null>(null);
  // The tour opens by itself the first time this account reaches the shell on this browser.
  const [showGuide, setShowGuide] = useState(() => !guideSeen(username));
  // Null until loaded, so the Today tab doesn't fetch twice (without, then with, stock).
  const [enabledPlugins, setEnabledPlugins] = useState<string[] | null>(null);

  async function refreshPlugins() {
    try {
      setEnabledPlugins((await getAppState()).enabled_plugins);
    } catch {
      // Keep the current tabs; Settings shows its own error for a failed switch. On the first
      // load, fall back to no optional plugins rather than an empty Today tab forever.
      setEnabledPlugins((current) => current ?? []);
    }
  }

  useEffect(() => {
    void refreshPlugins();
  }, []);

  // A plugin switched off in Settings takes its tab with it.
  const visibleTab: Tab = (() => {
    const plugin = PLUGIN_TABS[tab];
    return plugin && enabledPlugins && !enabledPlugins.includes(plugin) ? "today" : tab;
  })();

  function openChatWithDraft(prefillText: string) {
    setChatDraft(prefillText);
    setTab("chat");
  }

  return (
    <div className="flex h-dvh flex-col bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50 md:flex-row">
      <Sidebar
        tab={visibleTab}
        onChange={setTab}
        username={username}
        onShowGuide={() => setShowGuide(true)}
        isTabShown={(id) => {
          const plugin = PLUGIN_TABS[id];
          return !plugin || (enabledPlugins?.includes(plugin) ?? false);
        }}
      />
      <main className="min-h-0 flex-1 overflow-hidden">
        {visibleTab === "today" && enabledPlugins ? (
          <Today enabledPlugins={enabledPlugins} />
        ) : null}
        {visibleTab === "chat" ? (
          <Chat draftText={chatDraft} onDraftConsumed={() => setChatDraft(null)} />
        ) : null}
        {visibleTab === "nutrition" ? <Nutrition onAdjustPlan={openChatWithDraft} /> : null}
        {visibleTab === "training" ? <Training /> : null}
        {visibleTab === "household" ? <Household /> : null}
        {visibleTab === "stock" ? <Stock /> : null}
        {visibleTab === "settings" ? (
          <Settings onPluginsChanged={() => void refreshPlugins()} />
        ) : null}
      </main>
      {showGuide ? <Guide username={username} onClose={() => setShowGuide(false)} /> : null}
    </div>
  );
}
