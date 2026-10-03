import { useState } from "react";
import { signOut } from "../api/auth";
import { strings } from "../i18n";
import type { Tab } from "../App";

const TABS: { id: Tab; label: string }[] = [
  { id: "today", label: strings.nav.today },
  { id: "chat", label: strings.nav.chat },
  { id: "stock", label: strings.nav.stock },
  { id: "settings", label: strings.nav.settings },
];

interface SidebarProps {
  tab: Tab;
  onChange: (tab: Tab) => void;
  /** The logged-in user, shown with a logout button. */
  username: string | null;
}

export function Sidebar({ tab, onChange, username }: SidebarProps) {
  const [loggingOut, setLoggingOut] = useState(false);

  async function handleLogout() {
    setLoggingOut(true);
    await signOut();
  }

  return (
    // Phones: a bottom tab bar (after `main` in the flex column), with logout in Settings'
    // account section. From `md` up: a left sidebar.
    <nav className="order-last flex shrink-0 gap-1 border-t border-neutral-200 bg-neutral-50 px-2 pt-1 pb-[max(0.25rem,env(safe-area-inset-bottom))] md:order-none md:w-40 md:flex-col md:border-t-0 md:border-r md:p-3 dark:border-neutral-800 dark:bg-neutral-900">
      <p className="mb-2 hidden px-2 text-sm font-semibold text-neutral-900 md:block dark:text-neutral-50">
        {strings.app.name}
      </p>
      {TABS.map((item) => (
        <button
          key={item.id}
          type="button"
          onClick={() => onChange(item.id)}
          className={
            "flex-1 rounded-md px-1 py-2 text-center text-sm font-medium transition-colors md:flex-none md:px-3 md:text-left " +
            (tab === item.id
              ? "bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900"
              : "text-neutral-700 hover:bg-neutral-200 dark:text-neutral-300 dark:hover:bg-neutral-800")
          }
        >
          {item.label}
        </button>
      ))}
      {username ? (
        <div className="mt-auto hidden border-t border-neutral-200 px-2 pt-3 md:block dark:border-neutral-800">
          <p className="truncate text-xs text-neutral-500 dark:text-neutral-400" title={username}>
            {username}
          </p>
          <button
            type="button"
            className="mt-1 text-sm text-neutral-700 underline hover:text-neutral-900 disabled:opacity-40 dark:text-neutral-300 dark:hover:text-neutral-50"
            disabled={loggingOut}
            onClick={() => void handleLogout()}
          >
            {strings.common.logout}
          </button>
        </div>
      ) : null}
    </nav>
  );
}
