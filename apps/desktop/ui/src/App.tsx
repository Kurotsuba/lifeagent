import { useEffect, useState } from "react";
import { getAppState } from "./api/commands";
import { zhCN } from "./i18n/zh-CN";
import { Onboarding } from "./onboarding/Onboarding";

/**
 * Thin view layer: all state comes from Tauri commands, no business logic here
 * (see CLAUDE.md). Onboarding lands in M7.2; Today / Chat / Settings land in M7.3/M7.4.
 */
type Screen = "loading" | "onboarding" | "main";

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

  return (
    <div className="flex h-screen items-center justify-center bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
      <p className="text-lg">
        {screen === "loading" ? zhCN.common.loading : `${zhCN.app.name} — 开发中`}
      </p>
    </div>
  );
}
