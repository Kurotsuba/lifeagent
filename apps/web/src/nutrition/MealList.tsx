import { useState } from "react";
import { deleteEntry } from "../api/commands";
import { commandErrorMessage, type DietLoggedMeal } from "../api/types";
import { ConfirmDeleteButton } from "../components/ConfirmDeleteButton";
import { sourceDisplayName, strings } from "../i18n";

const t = strings.today.diet;

interface MealListProps {
  meals: DietLoggedMeal[];
  onDeleted: () => void;
}

/** Logged meals for today, each with a two-step delete (`ConfirmDeleteButton`) — no browser
 * `confirm()` popup, to stay consistent with the rest of the app's own styling. */
export function MealList({ meals, onDeleted }: MealListProps) {
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

  if (meals.length === 0) {
    return <p className="text-sm text-neutral-500 dark:text-neutral-400">{t.noMealsYet}</p>;
  }

  return (
    <div className="flex flex-col gap-2">
      {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}
      {meals.map((meal) => (
        <div
          key={meal.entry_id}
          className="flex items-start justify-between gap-3 rounded-md border border-neutral-200 px-3 py-2 text-sm dark:border-neutral-700"
        >
          <div>
            <p className="font-medium text-neutral-900 dark:text-neutral-50">
              {meal.time} · {strings.mealType[meal.meal_type]} ·{" "}
              {t.kcalEaten(Math.round(meal.total.kcal))}
            </p>
            <p className="mt-1 text-neutral-600 dark:text-neutral-400">
              {meal.items.map((item, i) => (
                <span key={`${meal.entry_id}-${i}`} title={
                    item.source_name === null
                      ? undefined
                      : sourceDisplayName(item.source_kind, item.source_name)
                  }
                >
                  {i > 0 ? strings.common.listSeparator : ""}
                  {item.kind === "dish" ? (
                    <>
                      {item.brand ? (
                        <span className="text-neutral-400 dark:text-neutral-500">
                          {item.brand}{" "}
                        </span>
                      ) : null}
                      {item.name} {t.servingsShort(item.servings ?? 1)}
                    </>
                  ) : (
                    <>
                      {item.name} {t.gramsShort(Math.round(item.grams ?? 0))}
                    </>
                  )}
                  {item.estimated ? (
                    <span className="ml-1 rounded bg-neutral-200 px-1 text-[10px] text-neutral-600 dark:bg-neutral-700 dark:text-neutral-300">
                      {t.estimatedBadge}
                    </span>
                  ) : null}
                </span>
              ))}
            </p>
          </div>
          <ConfirmDeleteButton onConfirm={() => handleDelete(meal.entry_id)} />
        </div>
      ))}
    </div>
  );
}
