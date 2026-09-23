import { zhCN } from "./i18n/zh-CN";

/**
 * Thin view layer: all state comes from Tauri commands, no business logic here
 * (see CLAUDE.md). Onboarding / Today / Chat / Settings pages land in M7.
 */
export default function App() {
  return (
    <div className="flex h-screen items-center justify-center bg-neutral-50 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-50">
      <p className="text-lg">{zhCN.app.name} — 开发中</p>
    </div>
  );
}
