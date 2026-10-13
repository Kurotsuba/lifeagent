
import { useEffect, useState } from "react";
import { getSettings, getToday } from "../api/commands";
import { onAppDataEvent } from "../api/events";
import { commandErrorMessage, dietCard, type MealType, type TodayView } from "../api/types";
import { HelpButton } from "../components/Guide";
import { strings } from "../i18n";
import { IntakeSummary } from "./IntakeSummary";
import { MealList } from "./MealList";
import { TodayPlanList } from "./TodayPlanList";
import { TomorrowPlanCard } from "./TomorrowPlanCard";
import { WeightLogDialog } from "./WeightLogDialog";

interface NutritionProps {
  onAdjustPlan: (prefillText: string) => void;
}

/** The diet plugin's page, where its data is edited: today's intake and logged meals (with
 * delete), weight logging, today's plan checklist, and tomorrow's plan (generate, accept,
 * adjust). The Today tab only shows a read-only summary. */
export function Nutrition({ onAdjustPlan }: NutritionProps) {
  const [view, setView] = useState<TodayView | null>(null);
  const [planTime, setPlanTime] = useState("21:00");
  const [error, setError] = useState<string | null>(null);
  const [showWeightDialog, setShowWeightDialog] = useState(false);

  async function refetch() {
    try {
      const [nextView] = await Promise.all([
        getToday(),
        getSettings().then((s) => setPlanTime(s.plan_time)),
      ]);
      setView(nextView);
      setError(null);
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  useEffect(() => {
    void refetch();
    return onAppDataEvent(() => void refetch());
  }, []);

  if (error && !view) {
    return (
      <div className="p-4 md:p-6">
        <p className="text-sm text-red-600 dark:text-red-400">{error}</p>
      </div>
    );
  }

  if (!view) {
    return (
      <div className="p-4 md:p-6">
        <p className="text-sm text-neutral-500 dark:text-neutral-400">{strings.common.loading}</p>
      </div>
    );
  }

  const card = dietCard(view);
  const loggedMealTypes = new Set<MealType>((card?.meals ?? []).map((m) => m.meal_type));

  return (
    <div className="h-full overflow-y-auto p-4 md:p-6">
      <div className="mx-auto flex max-w-2xl flex-col gap-6">
        <header className="relative flex items-center justify-between gap-3">
          <div className="flex shrink-0 items-center gap-2">
            <h1 className="text-lg font-semibold text-neutral-900 dark:text-neutral-50">
              {strings.nav.nutrition}
              <span className="ml-2 text-sm font-normal text-neutral-500 dark:text-neutral-400">
                {view.logical_date}
              </span>
            </h1>
            <HelpButton topic="nutrition" />
          </div>
          <div className="relative">
            <button
              type="button"
              className="text-right text-sm text-neutral-600 underline-offset-2 hover:underline dark:text-neutral-400"
              onClick={() => setShowWeightDialog((v) => !v)}
            >
              {view.latest_weight
                ? strings.today.latestWeight(view.latest_weight.kg)
                : strings.today.noWeightYet}{" "}
              · {strings.today.logWeight}
            </button>
            {showWeightDialog ? (
              <WeightLogDialog
                onClose={() => setShowWeightDialog(false)}
                onSaved={() => void refetch()}
              />
            ) : null}
          </div>
        </header>

        <section className="rounded-lg border border-neutral-200 p-4 dark:border-neutral-700">
          <h2 className="mb-3 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
            {strings.today.diet.title}
          </h2>
          <IntakeSummary card={card} />
          <div className="mt-4">
            <MealList meals={card?.meals ?? []} onDeleted={() => void refetch()} />
          </div>
        </section>

        {view.today_plan ? (
          <section className="rounded-lg border border-neutral-200 p-4 dark:border-neutral-700">
            <TodayPlanList plan={view.today_plan} loggedMealTypes={loggedMealTypes} />
          </section>
        ) : null}

        <section className="rounded-lg border border-neutral-200 p-4 dark:border-neutral-700">
          <TomorrowPlanCard
            plan={view.tomorrow_plan}
            planTime={planTime}
            onAdjust={onAdjustPlan}
            onChanged={() => void refetch()}
          />
        </section>
      </div>
    </div>
  );
}
