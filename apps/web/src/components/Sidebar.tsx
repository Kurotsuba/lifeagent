import { useState } from "react";
import { logout, SIGNED_OUT_EVENT } from "../api/auth";
import { strings } from "../i18n";
import type { Tab } from "../App";

const TABS: { id: Tab; label: string }[] = [
  { id: "today", label: strings.nav.today },
  { id: "chat", label: strings.nav.chat },
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
    try {
      await logout();
    } catch {
      // The session is gone for this tab either way; the server forgets it on expiry.
    }
    window.dispatchEvent(new Event(SIGNED_OUT_EVENT));
  }

  return (
    <nav className="flex w-40 shrink-0 flex-col gap-1 border-r border-neutral-200 bg-neutral-50 p-3 dark:border-neutral-800 dark:bg-neutral-900">
      <p className="mb-2 px-2 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
        {strings.app.name}
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
      {username ? (
        <div className="mt-auto border-t border-neutral-200 px-2 pt-3 dark:border-neutral-800">
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
