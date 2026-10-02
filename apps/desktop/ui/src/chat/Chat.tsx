import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { getHistory, sendMessage } from "../api/commands";
import { onAppDataEvent } from "../api/events";
import { isWeb } from "../api/platform";
import {
  commandErrorMessage,
  isCommandError,
  type CommandErrorCode,
  type HistoryMessage,
} from "../api/types";
import { inputClass } from "../components/Field";
import { primaryButtonClass } from "../components/buttonStyles";
import { zhCN } from "../i18n/zh-CN";

const HISTORY_LIMIT = 50;

interface ChatProps {
  /** Set by Today's 调整 button ("调整明天的计划：") to prefill the input on switching tabs. */
  draftText: string | null;
  onDraftConsumed: () => void;
  onGoToSettings: () => void;
}

interface SendError {
  message: string;
  code: CommandErrorCode | null;
}

export function Chat({ draftText, onDraftConsumed, onGoToSettings }: ChatProps) {
  const [history, setHistory] = useState<HistoryMessage[]>([]);
  const [loaded, setLoaded] = useState(false);
  // Only a *live* `send_message` response carries `activities` (history replay doesn't
  // reconstruct them — see `docs/DECISIONS.md`'s M7.1 entry), so this is matched against the
  // last rendered assistant message by content rather than folded into `history` itself.
  const [lastLiveReply, setLastLiveReply] = useState<{
    content: string;
    activities: string[];
  } | null>(null);
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<SendError | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  async function refetch() {
    try {
      setHistory(await getHistory(HISTORY_LIMIT));
    } catch {
      // Background refetch (triggered by an app event, not the user) — the send path below has
      // its own error UI, so a failed refetch here just leaves the last-known history in place.
    } finally {
      setLoaded(true);
    }
  }

  useEffect(() => {
    void refetch();
    return onAppDataEvent(() => void refetch());
  }, []);

  useEffect(() => {
    if (draftText !== null) {
      setText(draftText);
      onDraftConsumed();
    }
  }, [draftText]);

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [history, sending]);

  async function handleSend() {
    const trimmed = text.trim();
    if (trimmed === "" || sending) return;
    setSending(true);
    setError(null);
    setText("");
    const optimisticUser: HistoryMessage = {
      role: "user",
      content: trimmed,
      created_at: new Date().toISOString(),
    };
    // On a retry the failed turn's user message is already stored (and shown, via the refetch
    // below) — `send_message` resumes that turn rather than storing it again, so don't show it
    // twice either.
    setHistory((h) => {
      const last = h[h.length - 1];
      return last?.role === "user" && last.content === trimmed ? h : [...h, optimisticUser];
    });
    try {
      const turn = await sendMessage(trimmed);
      setLastLiveReply({ content: turn.reply, activities: turn.activities });
      await refetch();
    } catch (e) {
      // Resync with what was actually stored instead of just dropping the optimistic bubble:
      // the user message is persisted before the model call, so after most failures it's in the
      // history, and an event-triggered refetch mid-turn may already have replaced the list.
      await refetch();
      setText(trimmed);
      setError({
        message: commandErrorMessage(e),
        code: isCommandError(e) ? e.code : null,
      });
    } finally {
      setSending(false);
    }
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void handleSend();
    }
  }

  const visible = history.filter((m) => m.role !== "tool");

  return (
    <div className="flex h-full flex-col">
      <div ref={listRef} className="flex-1 overflow-y-auto px-6 py-4">
        <div className="mx-auto flex max-w-2xl flex-col gap-3">
          {!loaded ? (
            <p className="text-sm text-neutral-500 dark:text-neutral-400">
              {zhCN.common.loading}
            </p>
          ) : null}
          {loaded && visible.length === 0 ? (
            <p className="text-sm text-neutral-500 dark:text-neutral-400">{zhCN.chat.empty}</p>
          ) : null}
          {visible.map((m, i) => {
            const isLastAssistant =
              i === visible.length - 1 &&
              m.role === "assistant" &&
              lastLiveReply?.content === m.content;
            const chips = isLastAssistant ? lastLiveReply!.activities : [];
            return (
              <div
                key={i}
                className={m.role === "user" ? "flex justify-end" : "flex justify-start"}
              >
                <div
                  className={
                    "max-w-[75%] rounded-lg px-3 py-2 text-sm " +
                    (m.role === "user"
                      ? "bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900"
                      : "bg-neutral-100 text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100")
                  }
                >
                  {m.role === "assistant" && !m.content?.trim() ? (
                    <p className="italic text-neutral-500 dark:text-neutral-400">
                      {zhCN.chat.thinking}
                    </p>
                  ) : (
                    <p className="whitespace-pre-wrap">{m.content}</p>
                  )}
                  {chips.length > 0 ? (
                    <div className="mt-2 flex flex-wrap gap-1">
                      {chips.map((chip, j) => (
                        <span
                          key={j}
                          className="rounded-full bg-emerald-100 px-2 py-0.5 text-xs text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300"
                        >
                          {chip}
                        </span>
                      ))}
                    </div>
                  ) : null}
                </div>
              </div>
            );
          })}
          {sending ? (
            <div className="flex justify-start">
              <div className="max-w-[75%] rounded-lg bg-neutral-100 px-3 py-2 text-sm text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400">
                {zhCN.chat.sending}
              </div>
            </div>
          ) : null}
        </div>
      </div>

      {error ? (
        <div className="mx-auto flex w-full max-w-2xl flex-wrap items-center gap-3 px-6 pb-2 text-sm text-red-600 dark:text-red-400">
          <span>
            {zhCN.chat.sendFailed}：{error.message}
          </span>
          <button type="button" className="underline" onClick={() => void handleSend()}>
            {zhCN.common.retry}
          </button>
          {/* The web build has no model settings to go to: the operator owns them. */}
          {!isWeb && (error.code === "auth" || error.code === "not_configured") ? (
            <button type="button" className="underline" onClick={onGoToSettings}>
              {zhCN.common.goToSettings}
            </button>
          ) : null}
        </div>
      ) : null}

      <div className="border-t border-neutral-200 px-6 py-3 dark:border-neutral-700">
        <div className="mx-auto flex max-w-2xl items-end gap-2">
          <textarea
            className={`${inputClass} mt-0 min-h-[2.5rem] resize-none`}
            rows={1}
            placeholder={zhCN.chat.inputPlaceholder}
            value={text}
            disabled={sending}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={handleKeyDown}
          />
          <button
            type="button"
            className={primaryButtonClass}
            disabled={sending || text.trim() === ""}
            onClick={() => void handleSend()}
          >
            {sending ? zhCN.chat.sending : zhCN.chat.send}
          </button>
        </div>
      </div>
    </div>
  );
}
