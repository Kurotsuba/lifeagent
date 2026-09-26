import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { sendMessage } from "../api/commands";
import { commandErrorMessage } from "../api/types";
import { inputClass } from "../components/Field";
import { zhCN } from "../i18n/zh-CN";

/** Must match `QUICK_ENTRY_LABEL` in `apps/desktop/src-tauri/src/tray.rs` — the two sides only
 * agree on this by convention, the same as the `AppEvent` names in `api/events.ts`. */
export const QUICK_ENTRY_WINDOW_LABEL = "quick-entry";

const t = zhCN.quickEntry;
const REPLY_DISPLAY_MS = 4000;

/** Small always-on-top popup opened from the tray's 快速记录 item (§10.1/§10.3): one text box,
 * Enter sends through `send_message`, the reply shows briefly and then the window hides itself
 * (not closes — the tray reuses the same window on the next 快速记录 click). */
export function QuickEntry() {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [reply, setReply] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const hideTimer = useRef<number | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  function cancelHideTimer() {
    if (hideTimer.current !== null) {
      window.clearTimeout(hideTimer.current);
      hideTimer.current = null;
    }
  }

  /** Hides (not closes) the window and resets it, so the next 快速记录 opens a clean box rather
   * than the previous reply or error. */
  function hide() {
    cancelHideTimer();
    setReply(null);
    setError(null);
    void getCurrentWindow().hide();
  }

  // The window is reused, so `autoFocus` only fires on the very first open; refocus the input
  // whenever the tray shows it again.
  useEffect(() => {
    const unlisten = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) inputRef.current?.focus();
    });
    return () => {
      cancelHideTimer();
      void unlisten.then((f) => f());
    };
  }, []);

  async function handleSubmit() {
    const trimmed = text.trim();
    if (trimmed === "" || sending) return;
    cancelHideTimer();
    setSending(true);
    setError(null);
    setReply(null);
    try {
      const turn = await sendMessage(trimmed);
      setText("");
      setReply(turn.reply);
      hideTimer.current = window.setTimeout(hide, REPLY_DISPLAY_MS);
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setSending(false);
    }
  }

  function handleKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter") {
      e.preventDefault();
      void handleSubmit();
    }
    if (e.key === "Escape") {
      hide();
    }
  }

  return (
    <div className="flex h-screen flex-col justify-center gap-1.5 bg-white p-3 dark:bg-neutral-900">
      <input
        type="text"
        ref={inputRef}
        autoFocus
        className={`${inputClass} mt-0`}
        placeholder={t.placeholder}
        value={text}
        disabled={sending}
        onChange={(e) => {
          // Typing a follow-up while the last reply is still showing keeps the window open.
          cancelHideTimer();
          setText(e.target.value);
        }}
        onKeyDown={handleKeyDown}
      />
      {sending ? (
        <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.sending}</p>
      ) : null}
      {reply ? (
        <p className="text-xs text-neutral-700 dark:text-neutral-300">{reply}</p>
      ) : null}
      {error ? <p className="text-xs text-red-600 dark:text-red-400">{error}</p> : null}
    </div>
  );
}
