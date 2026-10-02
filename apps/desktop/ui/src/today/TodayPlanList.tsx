import { dietPlanSection, type MealType, type StoredPlan } from "../api/types";
import { strings } from "../i18n";

const t = strings.today.todayPlan;

interface TodayPlanListProps {
  plan: StoredPlan;
  loggedMealTypes: Set<MealType>;
}

/** Today's plan, per §10.3: a checklist with a ✓ against any meal type that already has a
 * logged meal — a simple heuristic (same meal type, not "the exact planned food was eaten"). */
export function TodayPlanList({ plan, loggedMealTypes }: TodayPlanListProps) {
  const section = dietPlanSection(plan);
  if (!section || section.meals.length === 0) return null;

  return (
    <div>
      <h3 className="mb-2 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
        {t.title}
      </h3>
      <ul className="flex flex-col gap-1 text-sm">
        {section.meals.map((meal, i) => {
          const done = loggedMealTypes.has(meal.meal_type);
          return (
            <li key={i} className="flex items-center gap-2">
              <span
                className={
                  done
                    ? "text-emerald-600 dark:text-emerald-400"
                    : "text-neutral-300 dark:text-neutral-600"
                }
                aria-hidden
              >
                {done ? "✓" : "○"}
              </span>
              <span className="text-neutral-500 dark:text-neutral-400">
                {strings.mealType[meal.meal_type]}
              </span>
              <span
                className={
                  done
                    ? "text-neutral-400 line-through dark:text-neutral-500"
                    : "text-neutral-700 dark:text-neutral-300"
                }
              >
                {meal.title}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
