import type { DietPlanMeal } from "../api/types";
import { strings } from "../i18n";

const t = strings.members.shared;

/** Under a planned meal shared with other members: a note that the items above show grams for
 * the whole table, who joins and how many portions to cook, with each guest's portion kcal
 * (computed in Rust) on expand. Nothing for other meals. */
export function SharedMeal({ meal }: { meal: DietPlanMeal }) {
  if (!meal.servings || meal.servings.length === 0) return null;
  const names = meal.servings.map((s) => s.name).join(strings.common.listSeparator);
  return (
    <div className="mt-1 text-xs text-neutral-600 dark:text-neutral-400">
      <p>{t.gramsNote}</p>
      <details className="mt-1">
        <summary className="cursor-pointer">{t.with(names, meal.portions ?? 1)}</summary>
        <ul className="mt-1 ml-4 list-disc">
          {meal.servings.map((s) => (
            <li key={s.member_id}>{t.guest(s.name, Math.round(s.total.kcal))}</li>
          ))}
        </ul>
        <p className="mt-1 text-neutral-500 dark:text-neutral-500">{t.estimateNote}</p>
      </details>
    </div>
  );
}
