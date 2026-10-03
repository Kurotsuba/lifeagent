import { useState } from "react";
import { deleteEntry } from "../api/commands";
import { commandErrorMessage, type TrainingView } from "../api/types";
import { ConfirmDeleteButton } from "../components/ConfirmDeleteButton";
import { strings } from "../i18n";
import { exerciseLabel } from "./format";

const t = strings.training.week;
const p = strings.training.plan;

interface WeekLogProps {
  view: TrainingView;
  onDeleted: () => void;
}

/** The last 7 days' logged sessions, newest day first, with totals. */
export function WeekLog({ view, onDeleted }: WeekLogProps) {
  const [error, setError] = useState<string | null>(null);

  async function handleDelete(id: string) {
    setError(null);
    try {
      await deleteEntry(id);
      onDeleted();
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  const days = [...view.days].reverse();

  return (
    <div>
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
        <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-50">{t.title}</h2>
        <p className="text-sm text-neutral-700 dark:text-neutral-300">
          {t.summary(view.week_sessions, Math.round(view.week_minutes))}
          {view.week_kcal > 0 ? (
            <span className="ml-2 text-neutral-500 dark:text-neutral-400" title={t.kcalHint}>
              {t.kcal(Math.round(view.week_kcal))}
            </span>
          ) : null}
        </p>
      </div>

      {/* Seven small bars, oldest first: minutes trained per day. */}
      <div className="mb-4 flex items-end gap-1" aria-hidden="true">
        {view.days.map((day) => {
          const max = Math.max(...view.days.map((d) => d.minutes), 1);
          return (
            <div key={day.date} className="flex flex-1 flex-col items-center gap-1">
              <div className="flex h-12 w-full items-end">
                <div
                  className={
                    "w-full rounded-sm " +
                    (day.minutes > 0
                      ? "bg-neutral-800 dark:bg-neutral-200"
                      : "bg-neutral-200 dark:bg-neutral-700")
                  }
                  style={{ height: `${Math.max(8, (day.minutes / max) * 100)}%` }}
                />
              </div>
              <span className="text-[10px] text-neutral-500 dark:text-neutral-400">
                {day.date === view.logical_date ? t.today : day.date.slice(5)}
              </span>
            </div>
          );
        })}
      </div>

      {error ? <p className="mb-2 text-sm text-red-600 dark:text-red-400">{error}</p> : null}

      {view.week_sessions === 0 ? (
        <p className="text-sm text-neutral-500 dark:text-neutral-400">{t.empty}</p>
      ) : (
        <div className="flex flex-col gap-2">
          {days.flatMap((day) =>
            day.sessions.map((s) => (
              <div
                key={s.entry_id}
                className="flex items-start justify-between gap-3 rounded-md border border-neutral-200 px-3 py-2 text-sm dark:border-neutral-700"
              >
                <div>
                  <p className="font-medium text-neutral-900 dark:text-neutral-50">
                    {day.date === view.logical_date ? t.today : day.date} {s.time} · {s.title}
                  </p>
                  <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
                    {[
                      p.minutes(Math.round(s.duration_min)),
                      p.intensity[s.intensity],
                      s.exercise_kcal != null ? p.kcal(Math.round(s.exercise_kcal)) : null,
                    ]
                      .filter((part): part is string => part !== null)
                      .join(" · ")}
                  </p>
                  <p className="mt-1 text-neutral-600 dark:text-neutral-400">
                    {s.exercises.map(exerciseLabel).join(strings.common.listSeparator)}
                  </p>
                </div>
                <ConfirmDeleteButton onConfirm={() => handleDelete(s.entry_id)} />
              </div>
            )),
          )}
        </div>
      )}
      <p className="mt-3 text-xs text-neutral-500 dark:text-neutral-400">{t.chatHint}</p>
    </div>
  );
}
