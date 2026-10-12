import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { getHistory, getPendingActions, sendMessage } from "../api/commands";
import { onAppDataEvent } from "../api/events";
import {
  commandErrorMessage,
  type HistoryMessage,
  type PendingAction,
  type ToolSummary,
  type TurnNotice,
} from "../api/types";
import { inputClass } from "../components/Field";
import { primaryButtonClass } from "../components/buttonStyles";
import { describeToolSummary, strings } from "../i18n";
import { PendingActions } from "./PendingActions";

const HISTORY_LIMIT = 50;

interface ChatProps {
  /** Set by Today's 调整 button ("调整明天的计划：") to prefill the input on switching tabs. */
  draftText: string | null;
  onDraftConsumed: () => void;
}

export function Chat({ draftText, onDraftConsumed }: ChatProps) {
  const [history, setHistory] = useState<HistoryMessage[]>([]);
  const [loaded, setLoaded] = useState(false);
  // Only a *live* `send_message` response carries `activities` (history replay doesn't
  // reconstruct them — see `docs/DECISIONS.md`'s M7.1 entry), so this is matched against the
  // last rendered assistant message by content rather than folded into `history` itself.
  const [lastLiveReply, setLastLiveReply] = useState<{
    content: string;
    activities: ToolSummary[];
  } | null>(null);
  // The live turn ended without a model reply: shown after the messages, with the turn's
  // activities (there's no reply bubble to hang them on), until the next send.
  const [liveNotice, setLiveNotice] = useState<{
    notice: TurnNotice;
    activities: ToolSummary[];
  } | null>(null);
  const [pendingActions, setPendingActions] = useState<PendingAction[]>([]);
  // Approved/rejected here since the last message, kept on screen with their outcome until then.
  const [resolvedActions, setResolvedActions] = useState<PendingAction[]>([]);
  const [text, setText] = useState("");
  const [readOnly, setReadOnly] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  async function refetch() {
    try {
      const [messages, pending] = await Promise.all([
        getHistory(HISTORY_LIMIT),
        getPendingActions(),
      ]);
      setHistory(messages);
      setPendingActions(pending);
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
      setReadOnly(false);
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
    setResolvedActions([]);
    setLiveNotice(null);
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
      const turn = await sendMessage(trimmed, readOnly);
      if (turn.notice) {
        setLastLiveReply(null);
        setLiveNotice({ notice: turn.notice, activities: turn.activities });
      } else {
        setLastLiveReply({ content: turn.reply, activities: turn.activities });
      }
      setPendingActions((p) => [
        ...p,
        ...turn.pending_actions.filter((a) => !p.some((q) => q.id === a.id)),
      ]);
      await refetch();
    } catch (e) {
      // Resync with what was actually stored instead of just dropping the optimistic bubble:
      // the user message is persisted before the model call, so after most failures it's in the
      // history, and an event-triggered refetch mid-turn may already have replaced the list.
      await refetch();
      setText(trimmed);
      setError(commandErrorMessage(e));
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
  const items = displayItems(visible);
  // A turn that ended with a notice has no reply to drop its thinking bubble; the notice ends it.
  if (liveNotice && !sending && items[items.length - 1]?.kind === "thinking") items.pop();
  // A thinking bubble at the end already shows the turn is in progress.
  const showSending = sending && items[items.length - 1]?.kind !== "thinking";

  function handleResolved(action: PendingAction) {
    setPendingActions((p) => p.filter((a) => a.id !== action.id));
    setResolvedActions((r) => [...r.filter((a) => a.id !== action.id), action]);
  }

  return (
    <div className="flex h-full flex-col">
      <div ref={listRef} className="flex-1 overflow-y-auto px-4 py-4 md:px-6">
        <div className="mx-auto flex max-w-2xl flex-col gap-3">
          {!loaded ? (
            <p className="text-sm text-neutral-500 dark:text-neutral-400">
              {strings.common.loading}
            </p>
          ) : null}
          {loaded && visible.length === 0 ? (
            <p className="text-sm text-neutral-500 dark:text-neutral-400">{strings.chat.empty}</p>
          ) : null}
          {items.map((item, i) => {
            if (item.kind === "thinking") {
              return (
                <div key={i} className="flex justify-start">
                  <div className="max-w-[75%] rounded-lg bg-neutral-100 px-3 py-2 text-sm dark:bg-neutral-800">
                    <p className="italic text-neutral-500 dark:text-neutral-400">
                      {item.rounds > 1 ? strings.chat.deepThinking : strings.chat.thinking}
                    </p>
                  </div>
                </div>
              );
            }
            const m = item.message;
            const isLastAssistant =
              i === items.length - 1 &&
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
                  <p className="whitespace-pre-wrap [overflow-wrap:anywhere]">{m.content}</p>
                  <ActivityChips activities={chips} />
                </div>
              </div>
            );
          })}
          {liveNotice && !sending ? (
            <div className="flex justify-start">
              <div className="max-w-[75%] rounded-lg bg-neutral-100 px-3 py-2 text-sm text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100">
                <p className="italic text-neutral-500 dark:text-neutral-400">
                  {strings.chat.notices[liveNotice.notice]}
                </p>
                <ActivityChips activities={liveNotice.activities} />
              </div>
            </div>
          ) : null}
          {showSending ? (
            <div className="flex justify-start">
              <div className="max-w-[75%] rounded-lg bg-neutral-100 px-3 py-2 text-sm text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400">
                {strings.chat.sending}
              </div>
            </div>
          ) : null}
        </div>
      </div>

      <PendingActions
        actions={[
          ...pendingActions,
          ...resolvedActions.filter((r) => !pendingActions.some((p) => p.id === r.id)),
        ]}
        onResolved={handleResolved}
      />

      {error ? (
        <div className="mx-auto flex w-full max-w-2xl flex-wrap items-center gap-3 px-4 pb-2 md:px-6 text-sm text-red-600 dark:text-red-400">
          <span>
            {strings.common.labelled(strings.chat.sendFailed, error)}
          </span>
          <button type="button" className="underline" onClick={() => void handleSend()}>
            {strings.common.retry}
          </button>
        </div>
      ) : null}

      <div className="border-t border-neutral-200 px-4 py-3 md:px-6 dark:border-neutral-700">
        <div className="mx-auto mb-2 max-w-2xl text-xs text-neutral-600 dark:text-neutral-400">
          <label className="inline-flex cursor-pointer items-center gap-2">
            <input
              type="checkbox"
              checked={readOnly}
              disabled={sending}
              onChange={(e) => setReadOnly(e.target.checked)}
              aria-describedby={readOnly ? "chat-read-only-hint" : undefined}
            />
            {strings.chat.readOnly}
          </label>
          {readOnly ? <p id="chat-read-only-hint" className="mt-1">{strings.chat.readOnlyHint}</p> : null}
        </div>
        <div className="mx-auto flex max-w-2xl items-end gap-2">
          <textarea
            className={`${inputClass} mt-0 min-h-[2.5rem] resize-none placeholder:truncate`}
            rows={1}
            placeholder={strings.chat.inputPlaceholder}
            value={text}
            disabled={sending}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={handleKeyDown}
          />
          <button
            type="button"
            className={`${primaryButtonClass} shrink-0 whitespace-nowrap`}
            disabled={sending || text.trim() === ""}
            onClick={() => void handleSend()}
          >
            {sending ? strings.chat.sending : strings.chat.send}
          </button>
        </div>
      </div>
    </div>
  );
}

type DisplayItem =
  | { kind: "message"; message: HistoryMessage }
  | { kind: "thinking"; rounds: number };

/**
 * The messages to render. A run of text-less assistant messages (one per tool-call round)
 * becomes a single thinking bubble that counts the rounds, shown only while it ends the list:
 * once a reply or the next user message follows, that turn is over and the bubble is dropped.
 */
function displayItems(messages: HistoryMessage[]): DisplayItem[] {
  const items: DisplayItem[] = [];
  for (const message of messages) {
    const last = items[items.length - 1];
    if (message.role === "assistant" && !message.content?.trim()) {
      if (last?.kind === "thinking") last.rounds += 1;
      else items.push({ kind: "thinking", rounds: 1 });
    } else {
      if (last?.kind === "thinking") items.pop();
      items.push({ kind: "message", message });
    }
  }
  return items;
}

/** What the turn's tool calls did, one chip each. */
function ActivityChips({ activities }: { activities: ToolSummary[] }) {
  if (activities.length === 0) return null;
  return (
    <div className="mt-2 flex flex-wrap gap-1">
      {activities.map((chip, j) => (
        <span
          key={j}
          className="rounded-full bg-emerald-100 px-2 py-0.5 text-xs text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300"
        >
          {describeToolSummary(chip)}
        </span>
      ))}
    </div>
  );
}
