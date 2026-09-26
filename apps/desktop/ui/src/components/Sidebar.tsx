import { zhCN } from "../i18n/zh-CN";
import type { Tab } from "../App";

const TABS: { id: Tab; label: string }[] = [
  { id: "today", label: zhCN.nav.today },
  { id: "chat", label: zhCN.nav.chat },
  { id: "settings", label: zhCN.nav.settings },
];

interface SidebarProps {
  tab: Tab;
  onChange: (tab: Tab) => void;
}

export function Sidebar({ tab, onChange }: SidebarProps) {
  return (
    <nav className="flex w-40 shrink-0 flex-col gap-1 border-r border-neutral-200 bg-neutral-50 p-3 dark:border-neutral-800 dark:bg-neutral-900">
      <p className="mb-2 px-2 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
        {zhCN.app.name}
      </p>
      {TABS.map((item) => (
        <button
          key={item.id}
          type="button"
          onClick={() => onChange(item.id)}
          className={
            "rounded-md px-3 py-2 text-left text-sm font-medium transition-colors " +
            (tab === item.id
              ? "bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900"
              : "text-neutral-700 hover:bg-neutral-200 dark:text-neutral-300 dark:hover:bg-neutral-800")
          }
        >
          {item.label}
        </button>
      ))}
    </nav>
  );
}
