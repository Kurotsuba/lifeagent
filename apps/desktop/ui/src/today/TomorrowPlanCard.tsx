import { useState } from "react";
import { acceptPlan, generatePlan } from "../api/commands";
import { commandErrorMessage, dietPlanSection, type StoredPlan } from "../api/types";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { zhCN } from "../i18n/zh-CN";

const t = zhCN.today.tomorrowPlan;

interface TomorrowPlanCardProps {
  plan: StoredPlan | null;
  /** "HH:MM" the scheduler will auto-generate at, for the no-plan-yet message. */
  planTime: string;
  onAdjust: (prefillText: string) => void;
  /** Refetch `get_today` after accept/generate — the event forwarder will also trigger one, but
   * this makes the button's own action feel immediate. */
  onChanged: () => void;
}

export function TomorrowPlanCard({ plan, planTime, onAdjust, onChanged }: TomorrowPlanCardProps) {
  const [busy, setBusy] = useState<"generate" | "accept" | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function handleGenerate(date?: string) {
    setBusy("generate");
    setError(null);
    try {
      await generatePlan(date);
      onChanged();
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setBusy(null);
    }
  }

  async function handleAccept(date: string) {
    setBusy("accept");
    setError(null);
    try {
      await acceptPlan(date);
      onChanged();
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setBusy(null);
    }
  }

  if (!plan) {
    return (
      <div>
        <h3 className="mb-2 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
          {t.title}
        </h3>
        <p className="mb-3 text-sm text-neutral-500 dark:text-neutral-400">
          {t.noPlanYet(planTime)}
        </p>
        {error ? <p className="mb-2 text-sm text-red-600 dark:text-red-400">{error}</p> : null}
        <button
          type="button"
          className={primaryButtonClass}
          disabled={busy !== null}
          onClick={() => handleGenerate()}
        >
          {busy === "generate" ? t.generating : t.generateNow}
        </button>
      </div>
    );
  }

  const section = dietPlanSection(plan);

  return (
    <div>
      <h3 className="mb-2 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
        {t.title}
      </h3>
      <p className="mb-3 text-sm text-neutral-700 dark:text-neutral-300">
        {plan.content.summary}
      </p>

      {section ? (
        <div className="mb-3 flex flex-col gap-2">
          {section.meals.map((meal, i) => (
            <div
              key={i}
              className="rounded-md border border-neutral-200 px-3 py-2 text-sm dark:border-neutral-700"
            >
              <p className="font-medium text-neutral-900 dark:text-neutral-50">
                {zhCN.mealType[meal.meal_type]} {meal.title} ·{" "}
                {zhCN.today.diet.kcalEaten(Math.round(meal.total.kcal))}
              </p>
              <p className="mt-1 text-neutral-600 dark:text-neutral-400">
                {meal.items.map((item, j) => (
                  <span key={j}>
                    {j > 0 ? "、" : ""}
                    {item.name} {zhCN.today.diet.gramsShort(Math.round(item.grams))}
                  </span>
                ))}
              </p>
              {meal.prep_note ? (
                <p className="mt-1 text-xs italic text-neutral-500 dark:text-neutral-400">
                  {meal.prep_note}
                </p>
              ) : null}
            </div>
          ))}
          <p className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
            {t.dayTotal(Math.round(section.total.kcal), Math.round(section.targets.kcal))}
          </p>
        </div>
      ) : null}

      {plan.warnings.length > 0 ? (
        <div className="mb-3 rounded-md bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-300">
          <p className="mb-1 font-medium">{t.warningsTitle}</p>
          <ul className="list-inside list-disc">
            {plan.warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </div>
      ) : null}

      {plan.content.disclaimer ? (
        <p className="mb-3 text-xs text-neutral-500 dark:text-neutral-400">
          {plan.content.disclaimer}
        </p>
      ) : null}

      {error ? <p className="mb-2 text-sm text-red-600 dark:text-red-400">{error}</p> : null}

      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          className={primaryButtonClass}
          disabled={busy !== null || plan.status === "accepted"}
          onClick={() => handleAccept(plan.plan_date)}
        >
          {plan.status === "accepted" ? t.accept + "✓" : t.accept}
        </button>
        <button
          type="button"
          className={secondaryButtonClass}
          disabled={busy !== null}
          onClick={() => handleGenerate(plan.plan_date)}
        >
          {busy === "generate" ? t.generating : t.regenerate}
        </button>
        <button
          type="button"
          className={secondaryButtonClass}
          disabled={busy !== null}
          onClick={() => onAdjust(t.adjustPrefill)}
        >
          {t.adjust}
        </button>
      </div>
    </div>
  );
}
