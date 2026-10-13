import { useState } from "react";
import { acceptPlan } from "../api/commands";
import {
  startPlanGeneration,
  startPlanRevision,
  usePlanGeneration,
} from "../api/planGeneration";
import {
  commandErrorMessage,
  dietPlanSection,
  mealItemName,
  trainingPlanSection,
  type StoredPlan,
} from "../api/types";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { inputClass } from "../components/Field";
import { describePlanNote, mealItemSource, strings } from "../i18n";
import { SharedMeal } from "./SharedMeal";

const t = strings.today.tomorrowPlan;

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
  const [accepting, setAccepting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Shared with the Training tab and kept across page switches, so a running generation can't
  // be started twice by leaving and coming back.
  const generation = usePlanGeneration();
  const busy = generation.revising
    ? "revise"
    : generation.pending
      ? "generate"
      : accepting
        ? "accept"
        : null;
  const shownError = error ?? generation.error;
  // Optional wishes for this plan ("working late, convenience-store food"): Regenerate makes a
  // new plan with them, Adjust changes the current one. Saying them up front saves a chat round
  // trip, and its tokens.
  const [request, setRequest] = useState("");

  async function handleGenerate(date?: string) {
    setError(null);
    if (await startPlanGeneration(date, request.trim() || undefined)) {
      setRequest("");
      onChanged();
    }
  }

  /** With wishes typed, Adjust applies them right here; without, it opens the chat to talk the
   * change through. */
  async function handleAdjust(date: string) {
    const instruction = request.trim();
    if (!instruction) {
      onAdjust(t.adjustPrefill);
      return;
    }
    setError(null);
    if (await startPlanRevision(date, instruction)) {
      setRequest("");
      onChanged();
    }
  }

  async function handleAccept(date: string) {
    setAccepting(true);
    setError(null);
    try {
      await acceptPlan(date);
      onChanged();
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setAccepting(false);
    }
  }

  const requestBox = (
    <textarea
      className={inputClass + " mb-3 resize-y"}
      rows={2}
      value={request}
      placeholder={plan ? t.requestPlaceholderWithPlan : t.requestPlaceholder}
      aria-label={t.requestLabel}
      disabled={busy !== null}
      onChange={(e) => setRequest(e.target.value)}
    />
  );

  if (!plan) {
    return (
      <div>
        <h3 className="mb-2 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
          {t.title}
        </h3>
        <p className="mb-3 text-sm text-neutral-500 dark:text-neutral-400">
          {t.noPlanYet(planTime)}
        </p>
        {shownError ? (
        <p className="mb-2 text-sm text-red-600 dark:text-red-400">{shownError}</p>
      ) : null}
        {requestBox}
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
  const training = trainingPlanSection(plan);
  const kcalAdded = section?.exercise_kcal_added ?? 0;

  return (
    <div>
      <h3 className="mb-2 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
        {t.title}
      </h3>
      <p className="mb-3 text-sm text-neutral-700 dark:text-neutral-300">
        {plan.content.summary}
      </p>

      {training ? (
        <p className="mb-3 text-sm font-medium text-neutral-700 dark:text-neutral-300">
          {training.rest_day
            ? t.trainingRest
            : t.trainingLine(
                training.title || strings.training.plan.untitled,
                Math.round(training.duration_min),
              )}
        </p>
      ) : null}

      {section ? (
        <div className="mb-3 flex flex-col gap-2">
          {section.meals.map((meal, i) => (
            <div
              key={i}
              className="rounded-md border border-neutral-200 px-3 py-2 text-sm dark:border-neutral-700"
            >
              <p className="font-medium text-neutral-900 dark:text-neutral-50">
                {strings.mealType[meal.meal_type]} {meal.title} ·{" "}
                {strings.today.diet.kcalEaten(Math.round(meal.total.kcal))}
              </p>
              <p className="mt-1 text-neutral-600 dark:text-neutral-400">
                {meal.items.map((item, j) => (
                  <span key={j} title={mealItemSource(item)}>
                    {j > 0 ? strings.common.listSeparator : ""}
                    {mealItemName(item)} {strings.today.diet.gramsShort(Math.round(item.table_grams ?? item.grams ?? 0))}
                  </span>
                ))}
              </p>
              {meal.prep_note ? (
                <p className="mt-1 text-xs italic text-neutral-500 dark:text-neutral-400">
                  {meal.prep_note}
                </p>
              ) : null}
              <SharedMeal meal={meal} />
            </div>
          ))}
          <p className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
            {t.dayTotal(Math.round(section.total.kcal), Math.round(section.targets.kcal))}
            {kcalAdded > 0 ? (
              <span className="ml-2 text-xs font-normal text-neutral-500 dark:text-neutral-400">
                {t.trainingKcalAdded(Math.round(kcalAdded))}
              </span>
            ) : null}
          </p>
        </div>
      ) : null}

      {plan.warnings.length > 0 ? (
        <div className="mb-3 rounded-md bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-300">
          <p className="mb-1 font-medium">{t.warningsTitle}</p>
          <ul className="list-inside list-disc">
            {plan.warnings.map((w, i) => (
              <li key={i}>{describePlanNote(w)}</li>
            ))}
          </ul>
        </div>
      ) : null}

      {/* The stored text is fixed zh-CN; its presence is what matters. */}
      {plan.content.disclaimer ? (
        <p className="mb-3 text-xs text-neutral-500 dark:text-neutral-400">{t.medicalDisclaimer}</p>
      ) : null}

      {shownError ? (
        <p className="mb-2 text-sm text-red-600 dark:text-red-400">{shownError}</p>
      ) : null}

      {requestBox}
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
          onClick={() => handleAdjust(plan.plan_date)}
        >
          {busy === "revise" ? t.adjusting : t.adjust}
        </button>
      </div>
    </div>
  );
}
