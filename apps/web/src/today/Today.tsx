import { useEffect, useState, type ReactNode } from "react";
import { getStock, getToday } from "../api/commands";
import { onAppDataEvent } from "../api/events";
import {
  commandErrorMessage,
  dietCard,
  dietPlanSection,
  trainingPlanSection,
  type MealType,
  type StockView,
  type TodayView,
} from "../api/types";
import { strings } from "../i18n";
import { IntakeSummary } from "../nutrition/IntakeSummary";
import { ShoppingList } from "../stock/ShoppingList";
import { TrainingPlanCard } from "../training/TrainingPlanCard";

const t = strings.overview;

function Card({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="rounded-lg border border-neutral-200 p-4 dark:border-neutral-700">
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-50">{title}</h2>
        {aside}
      </div>
      {children}
    </section>
  );
}

function Empty({ children }: { children: ReactNode }) {
  return <p className="text-sm text-neutral-500 dark:text-neutral-400">{children}</p>;
}

interface TodayProps {
  /** Ids of the plugins switched on; training and stock cards show only when theirs is. */
  enabledPlugins: string[];
}

/** The Today tab: a read-only view of the day — today's intake (kcal ring and macro bars),
 * today's diet plan, today's training plan and the shopping list. Editing happens on each plugin's own tab (and in chat). */
export function Today({ enabledPlugins }: TodayProps) {
  const showTraining = enabledPlugins.includes("training");
  const showStock = enabledPlugins.includes("stock");
  const [view, setView] = useState<TodayView | null>(null);
  const [stock, setStock] = useState<StockView | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function refetch() {
    try {
      const [nextView, nextStock] = await Promise.all([
        getToday(),
        showStock ? getStock() : Promise.resolve(null),
      ]);
      setView(nextView);
      setStock(nextStock);
      setError(null);
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  useEffect(() => {
    void refetch();
    return onAppDataEvent(() => void refetch());
    // Refetch when the stock plugin is switched on or off.
  }, [showStock]);

  if (!view) {
    return (
      <div className="p-4 md:p-6">
        <p
          className={
            "text-sm " +
            (error ? "text-red-600 dark:text-red-400" : "text-neutral-500 dark:text-neutral-400")
          }
        >
          {error ?? strings.common.loading}
        </p>
      </div>
    );
  }

  const training = view.today_plan ? trainingPlanSection(view.today_plan) : null;

  return (
    <div className="h-full overflow-y-auto p-4 md:p-6">
      <div className="mx-auto flex max-w-2xl flex-col gap-6">
        <h1 className="text-lg font-semibold text-neutral-900 dark:text-neutral-50">
          {strings.nav.today}
          <span className="ml-2 text-sm font-normal text-neutral-500 dark:text-neutral-400">
            {view.logical_date}
          </span>
        </h1>
        {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}

        <Card title={strings.today.diet.title}>
          <IntakeSummary card={dietCard(view)} />
        </Card>

        <DietPlanCard view={view} />

        {showTraining ? (
          <Card title={strings.training.todayPlan.title}>
            {training ? (
              <TrainingPlanCard section={training} />
            ) : (
              <Empty>{t.noTrainingPlan}</Empty>
            )}
          </Card>
        ) : null}

        {showStock && stock ? (
          <Card
            title={strings.stock.shoppingTitle}
            aside={
              stock.shopping ? (
                <span className="text-xs text-neutral-500 dark:text-neutral-400">
                  {strings.stock.shoppingFor(stock.shopping.plan_date)}
                </span>
              ) : null
            }
          >
            {stock.shopping ? (
              <ShoppingList shopping={stock.shopping} />
            ) : (
              <Empty>{strings.stock.noShopping}</Empty>
            )}
          </Card>
        ) : null}
      </div>
    </div>
  );
}

/** Today's planned meals with what's in them and a ✓ on meal types already logged (the same
 * heuristic as the Nutrition tab's checklist). */
function DietPlanCard({ view }: { view: TodayView }) {
  const card = dietCard(view);
  const section = view.today_plan ? dietPlanSection(view.today_plan) : null;
  const logged = new Set<MealType>((card?.meals ?? []).map((m) => m.meal_type));

  return (
    <Card title={t.dietTitle}>
      {!section || section.meals.length === 0 ? (
        <Empty>{t.noDietPlan}</Empty>
      ) : (
        <div className="flex flex-col gap-2">
          {section.meals.map((meal, i) => {
            const done = logged.has(meal.meal_type);
            return (
              <div
                key={i}
                className="rounded-md border border-neutral-200 px-3 py-2 text-sm dark:border-neutral-700"
              >
                <p className="flex items-baseline gap-2">
                  <span
                    aria-hidden
                    className={
                      done
                        ? "text-emerald-600 dark:text-emerald-400"
                        : "text-neutral-300 dark:text-neutral-600"
                    }
                  >
                    {done ? "✓" : "○"}
                  </span>
                  <span className="text-neutral-500 dark:text-neutral-400">
                    {strings.mealType[meal.meal_type]}
                  </span>
                  <span className="font-medium text-neutral-900 dark:text-neutral-50">
                    {meal.title}
                  </span>
                  <span className="ml-auto shrink-0 tabular-nums text-neutral-500 dark:text-neutral-400">
                    {strings.today.diet.kcalEaten(Math.round(meal.total.kcal))}
                  </span>
                </p>
                <p className="mt-1 text-neutral-600 dark:text-neutral-400">
                  {meal.items.map((item, j) => (
                    <span key={j}>
                      {j > 0 ? strings.common.listSeparator : ""}
                      {item.name} {strings.today.diet.gramsShort(Math.round(item.grams ?? 0))}
                    </span>
                  ))}
                </p>
                {meal.prep_note ? (
                  <p className="mt-1 text-xs italic text-neutral-500 dark:text-neutral-400">
                    {meal.prep_note}
                  </p>
                ) : null}
              </div>
            );
          })}
          <p className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
            {strings.today.tomorrowPlan.dayTotal(
              Math.round(section.total.kcal),
              Math.round(section.targets.kcal),
            )}
          </p>
        </div>
      )}
    </Card>
  );
}
