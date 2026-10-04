import { useState } from "react";
import { clearAttendanceOverride, setAttendanceOverride } from "../api/commands";
import { commandErrorMessage, type DaySlot, type HouseholdView } from "../api/types";
import { strings } from "../i18n";

const t = strings.household;

/** "10-05 (Sun)": the date without the year and its weekday, for labels only. */
function dayLabel(date: string): string {
  const weekday = (new Date(`${date}T00:00:00`).getDay() + 6) % 7;
  return `${date.slice(5)} (${t.attendance.weekdays[weekday]})`;
}

interface WeekAttendanceProps {
  view: HouseholdView;
  onChanged: () => void;
}

/** Who eats each main meal over the next 7 days, as the backend resolved it. Tapping a meal
 * makes a one-off change for that day; ↺ removes it. */
export function WeekAttendance({ view, onChanged }: WeekAttendanceProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      await action();
      onChanged();
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  function chip(date: string, slot: DaySlot) {
    return (
      <span key={slot.meal_type} className="inline-flex items-center">
        <button
          type="button"
          title={slot.note ?? undefined}
          disabled={busy}
          onClick={() =>
            void run(() =>
              setAttendanceOverride(slot.member_id, date, slot.meal_type, !slot.present),
            )
          }
          className={
            "rounded px-1.5 py-0.5 text-xs " +
            (slot.present
              ? "bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900"
              : "border border-neutral-300 text-neutral-400 line-through dark:border-neutral-700")
          }
        >
          {strings.mealType[slot.meal_type]}
          {slot.overridden ? "*" : ""}
        </button>
        {slot.overridden ? (
          <button
            type="button"
            aria-label={t.week.reset}
            title={t.week.reset}
            disabled={busy}
            className="px-0.5 text-xs text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200"
            onClick={() =>
              void run(() => clearAttendanceOverride(slot.member_id, date, slot.meal_type))
            }
          >
            ↺
          </button>
        ) : null}
      </span>
    );
  }

  return (
    <div className="space-y-3">
      <div>
        <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-50">
          {t.week.title}
        </h2>
        <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.week.hint}</p>
      </div>
      <ul className="flex flex-col gap-2 text-sm">
        {view.week.map((day) => (
          <li
            key={day.date}
            className="flex flex-col gap-1 border-b border-neutral-100 pb-2 last:border-0 dark:border-neutral-800"
          >
            <span className="text-xs font-medium text-neutral-700 dark:text-neutral-300">
              {day.date === view.today ? `${t.week.today} · ` : ""}
              {dayLabel(day.date)}
            </span>
            {view.members.map((m) => (
              <div key={m.id} className="flex flex-wrap items-center gap-1.5">
                <span className="w-20 truncate text-xs text-neutral-600 dark:text-neutral-400">
                  {m.display_name}
                </span>
                {day.slots.filter((s) => s.member_id === m.id).map((s) => chip(day.date, s))}
                {day.slots
                  .filter((s) => s.member_id === m.id && s.note)
                  .map((s) => (
                    <span
                      key={`note-${s.meal_type}`}
                      className="text-xs italic text-neutral-500 dark:text-neutral-400"
                    >
                      {s.note}
                    </span>
                  ))}
              </div>
            ))}
          </li>
        ))}
      </ul>
      {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}
    </div>
  );
}
