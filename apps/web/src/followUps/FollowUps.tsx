import { useEffect, useState } from "react";
import { answerFollowUp, getFollowUps } from "../api/commands";
import { onFollowUpsChanged } from "../api/events";
import { commandErrorMessage, type FollowUp } from "../api/types";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { describeFollowUp, followUpAnswerLabel, strings } from "../i18n";

const t = strings.followUps;

interface FollowUpsProps {
  /** For `core_reply`: opens the chat with `draft` in the input. */
  onReply: (draft: string) => void;
}

/**
 * Check-ins the scheduler raised on its own (`crates/core/src/follow_ups.rs`), shown on every
 * page with one-tap answers. Whether to ask is decided on the server; this only renders the
 * open ones and sends the chosen answer. `core_reply` never reaches the server: it opens the
 * chat, and the follow-up clears itself once the meal is logged there.
 */
export function FollowUps({ onReply }: FollowUpsProps) {
  const [followUps, setFollowUps] = useState<FollowUp[]>([]);
  const [minimized, setMinimized] = useState(false);
  const [busy, setBusy] = useState<{ id: string; answer: string } | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function refetch() {
      try {
        const open = await getFollowUps();
        if (!cancelled) setFollowUps(open);
      } catch {
        // Best effort: the next event refetches.
      }
    }
    void refetch();
    const unsubscribe = onFollowUpsChanged(() => void refetch());
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, []);

  // A new check-in reopens the panel.
  const newestId = followUps[followUps.length - 1]?.id;
  useEffect(() => {
    if (newestId) setMinimized(false);
  }, [newestId]);

  if (followUps.length === 0) return null;

  async function answer(f: FollowUp, kind: string) {
    if (kind === "core_reply") {
      if (f.kind === "diet_meal_missed") {
        onReply(t.replyDraft(strings.mealType[f.params.meal_type]));
      } else {
        onReply("");
      }
      return;
    }
    setBusy({ id: f.id, answer: kind });
    setError(null);
    try {
      await answerFollowUp(f.id, kind);
      setFollowUps((list) => list.filter((x) => x.id !== f.id));
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setBusy(null);
    }
  }

  if (minimized) {
    return (
      <button
        type="button"
        onClick={() => setMinimized(false)}
        className="fixed top-3 right-3 z-30 rounded-full bg-violet-600 px-3 py-1.5 text-xs font-medium text-white shadow-lg hover:bg-violet-500 md:top-4 md:right-6"
      >
        {t.show(followUps.length)}
      </button>
    );
  }

  return (
    <section
      role="status"
      aria-label={t.title}
      className="fixed inset-x-3 top-3 z-30 mx-auto max-h-[70dvh] max-w-sm overflow-y-auto rounded-lg border border-violet-300 bg-white p-3 text-sm shadow-lg dark:border-violet-800 dark:bg-neutral-800 md:inset-x-auto md:top-4 md:right-6"
    >
      <div className="flex items-start justify-between gap-2">
        <div>
          <p className="font-medium text-violet-900 dark:text-violet-200">{t.title}</p>
          <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">{t.hint}</p>
        </div>
        <button
          type="button"
          onClick={() => setMinimized(true)}
          className="shrink-0 rounded px-2 py-0.5 text-xs text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-700"
        >
          {t.minimize}
        </button>
      </div>
      <ul className="mt-2 flex flex-col gap-2">
        {followUps.map((f) => {
          const working = busy?.id === f.id;
          return (
            <li key={f.id} className="rounded-md bg-violet-50 px-3 py-2 dark:bg-violet-950/40">
              {describeFollowUp(f).map((line, i) => (
                <p
                  key={i}
                  className="[overflow-wrap:anywhere] text-neutral-900 dark:text-neutral-100"
                >
                  {line}
                </p>
              ))}
              {working ? (
                <p className="mt-2 text-xs text-violet-800 dark:text-violet-300">
                  {busy.answer === "core_adjust_tomorrow" ? t.adjusting : t.working}
                </p>
              ) : (
                <div className="mt-2 flex flex-wrap gap-2">
                  {f.answers.map((kind, i) => (
                    <button
                      key={kind}
                      type="button"
                      disabled={busy !== null}
                      onClick={() => void answer(f, kind)}
                      className={`${i === 0 ? primaryButtonClass : secondaryButtonClass} px-3 py-1`}
                    >
                      {followUpAnswerLabel(kind, f)}
                    </button>
                  ))}
                </div>
              )}
            </li>
          );
        })}
      </ul>
      {error ? (
        <p className="mt-2 text-red-600 dark:text-red-400">
          {strings.common.labelled(t.failed, error)}
        </p>
      ) : null}
    </section>
  );
}
