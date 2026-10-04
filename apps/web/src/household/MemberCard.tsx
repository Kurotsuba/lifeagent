import { useState } from "react";
import {
  addMemberDislike,
  archiveMember,
  removeMemberDislike,
  setAttendanceDefault,
} from "../api/commands";
import { commandErrorMessage, type MainMealType, type MemberView } from "../api/types";
import { inputClass } from "../components/Field";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { strings } from "../i18n";

const t = strings.household;
const p = strings.onboarding.profile;
const MEALS: MainMealType[] = ["breakfast", "lunch", "dinner"];

interface MemberCardProps {
  member: MemberView;
  onEdit: () => void;
  onChanged: () => void;
}

/** One member: profile summary, usual shared meals (a weekday × meal grid saved on each
 * toggle), dislikes, and edit/remove. */
export function MemberCard({ member, onEdit, onChanged }: MemberCardProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dislike, setDislike] = useState("");
  const [confirmingRemove, setConfirmingRemove] = useState(false);

  async function run(action: () => Promise<unknown>) {
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

  const present = (weekday: number, meal: MainMealType) =>
    member.attendance.some((s) => s.weekday === weekday && s.meal_type === meal && s.present);

  return (
    <div className="space-y-3">
      <div className="flex items-start justify-between gap-2">
        <div>
          <p className="font-medium text-neutral-900 dark:text-neutral-50">{member.display_name}</p>
          <p className="text-xs text-neutral-500 dark:text-neutral-400">
            {t.memberLine(
              member.age,
              p.sexOptions[member.sex],
              p.dietaryPatternOptions[member.dietary_pattern],
            )}
          </p>
          {member.allergies.length > 0 ? (
            <p className="text-xs text-neutral-500 dark:text-neutral-400">
              {t.allergiesLine(member.allergies.join(strings.common.listSeparator))}
            </p>
          ) : null}
        </div>
        <div className="flex shrink-0 gap-2">
          <button
            type="button"
            className={`${secondaryButtonClass} !px-2 !py-1 text-xs`}
            onClick={onEdit}
          >
            {t.edit}
          </button>
          {confirmingRemove ? (
            <button
              type="button"
              className={`${primaryButtonClass} !px-2 !py-1 text-xs`}
              disabled={busy}
              onClick={() => void run(() => archiveMember(member.id))}
            >
              {t.archiveConfirm}
            </button>
          ) : (
            <button
              type="button"
              className={`${secondaryButtonClass} !px-2 !py-1 text-xs`}
              onClick={() => setConfirmingRemove(true)}
            >
              {t.archive}
            </button>
          )}
        </div>
      </div>

      <div>
        <p className="mb-1 text-xs font-medium text-neutral-700 dark:text-neutral-300">
          {t.attendance.title}
        </p>
        <table className="text-xs">
          <thead>
            <tr>
              <th />
              {t.attendance.weekdays.map((day) => (
                <th
                  key={day}
                  className="px-1 font-normal text-neutral-500 dark:text-neutral-400"
                >
                  {day}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {MEALS.map((meal) => (
              <tr key={meal}>
                <td className="pr-2 text-neutral-500 dark:text-neutral-400">
                  {strings.mealType[meal]}
                </td>
                {t.attendance.weekdays.map((day, weekday) => (
                  <td key={day} className="px-1 text-center">
                    <input
                      type="checkbox"
                      aria-label={`${day} ${strings.mealType[meal]}`}
                      checked={present(weekday, meal)}
                      disabled={busy}
                      onChange={(e) =>
                        void run(() =>
                          setAttendanceDefault(member.id, {
                            weekday,
                            meal_type: meal,
                            present: e.target.checked,
                          }),
                        )
                      }
                    />
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div>
        <p className="mb-1 text-xs font-medium text-neutral-700 dark:text-neutral-300">
          {t.dislikes.title}
        </p>
        <div className="flex flex-wrap items-center gap-1.5">
          {member.dislikes.length === 0 ? (
            <span className="text-xs text-neutral-500 dark:text-neutral-400">
              {t.dislikes.none}
            </span>
          ) : null}
          {member.dislikes.map((d) => (
            <span
              key={d.id}
              className="flex items-center gap-1 rounded-full bg-neutral-100 px-2 py-0.5 text-xs dark:bg-neutral-800"
            >
              {d.text}
              <button
                type="button"
                aria-label={strings.common.delete}
                className="text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200"
                disabled={busy}
                onClick={() => void run(() => removeMemberDislike(d.id))}
              >
                ×
              </button>
            </span>
          ))}
        </div>
        <form
          className="mt-2 flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            const text = dislike.trim();
            if (!text) return;
            void run(async () => {
              await addMemberDislike(member.id, text);
              setDislike("");
            });
          }}
        >
          <input
            type="text"
            className={`${inputClass} !mt-0`}
            maxLength={100}
            placeholder={t.dislikes.placeholder}
            value={dislike}
            onChange={(e) => setDislike(e.target.value)}
          />
          <button
            type="submit"
            className={`${secondaryButtonClass} shrink-0`}
            disabled={busy || dislike.trim() === ""}
          >
            {t.dislikes.add}
          </button>
        </form>
      </div>
      {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}
    </div>
  );
}
