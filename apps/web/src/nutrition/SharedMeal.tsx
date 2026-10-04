import type { DietPlanMeal } from "../api/types";
import { strings } from "../i18n";

const t = strings.household.shared;

/** Under a planned meal shared with household members: who joins and how many portions to
 * cook, with each guest's portion kcal (computed in Rust) on expand. Nothing for other meals. */
export function SharedMeal({ meal }: { meal: DietPlanMeal }) {
  if (!meal.servings || meal.servings.length === 0) return null;
  const names = meal.servings.map((s) => s.name).join(strings.common.listSeparator);
  return (
    <details className="mt-1 text-xs text-neutral-600 dark:text-neutral-400">
      <summary className="cursor-pointer">{t.with(names, meal.portions ?? 1)}</summary>
      <ul className="mt-1 ml-4 list-disc">
        {meal.servings.map((s) => (
          <li key={s.member_id}>{t.guest(s.name, Math.round(s.total.kcal))}</li>
        ))}
      </ul>
      <p className="mt-1 text-neutral-500 dark:text-neutral-500">{t.estimateNote}</p>
    </details>
  );
}
