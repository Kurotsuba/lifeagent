import { useEffect, useState } from "react";
import { previewTargets } from "../api/commands";
import { commandErrorMessage } from "../api/types";
import type { GoalDirection, NutritionTargets } from "../api/types";
import { Field, inputClass } from "../components/Field";
import { zhCN } from "../i18n/zh-CN";
import { buildGoal, buildProfile, type OnboardingDraft } from "./draft";

const t = zhCN.onboarding.goal;

const directions: GoalDirection[] = ["lose", "maintain", "gain"];
const rates = [0.25, 0.5, 0.75];

/** The spec's own default rate per direction (§10.3): 0.5 for lose, 0.25 for gain. `maintain`
 * doesn't use a rate at all, so its default is arbitrary — kept at whatever was already set. */
function defaultRateFor(direction: GoalDirection, current: number): number {
  if (direction === "lose") return 0.5;
  if (direction === "gain") return 0.25;
  return current;
}

interface StepGoalProps {
  draft: OnboardingDraft;
  update: (patch: Partial<OnboardingDraft>) => void;
}

export function StepGoal({ draft, update }: StepGoalProps) {
  const [preview, setPreview] = useState<NutritionTargets | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const profile = buildProfile(draft);
    const goal = buildGoal(draft);
    const weightKg = Number(draft.weightKg);

    previewTargets(profile, weightKg, goal)
      .then((targets) => {
        if (!cancelled) {
          setPreview(targets);
          setPreviewError(null);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setPreview(null);
          setPreviewError(commandErrorMessage(error));
        }
      });

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    draft.weightKg,
    draft.direction,
    draft.targetWeightKg,
    draft.ratePerWeek,
    draft.sex,
    draft.birthDate,
    draft.heightCm,
    draft.activityLevel,
    draft.dietaryPattern,
  ]);

  return (
    <div className="space-y-5">
      <h2 className="text-lg font-semibold text-neutral-900 dark:text-neutral-50">{t.title}</h2>

      <fieldset>
        <legend className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
          {t.direction}
        </legend>
        <div className="mt-1 flex gap-4">
          {directions.map((direction) => (
            <label key={direction} className="flex items-center gap-1.5 text-sm">
              <input
                type="radio"
                name="direction"
                checked={draft.direction === direction}
                onChange={() =>
                  update({
                    direction,
                    ratePerWeek: defaultRateFor(direction, draft.ratePerWeek),
                  })
                }
              />
              {t.directionOptions[direction]}
            </label>
          ))}
        </div>
      </fieldset>

      <Field label={t.targetWeightKg} hint={t.targetWeightPlaceholder}>
        <input
          type="number"
          min={30}
          max={300}
          step="0.1"
          className={inputClass}
          value={draft.targetWeightKg}
          onChange={(e) => update({ targetWeightKg: e.target.value })}
        />
      </Field>

      {draft.direction !== "maintain" ? (
        <fieldset>
          <legend className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
            {t.ratePerWeek}
          </legend>
          <div className="mt-1 flex gap-4">
            {rates.map((rate) => (
              <label key={rate} className="flex items-center gap-1.5 text-sm">
                <input
                  type="radio"
                  name="rate"
                  checked={draft.ratePerWeek === rate}
                  onChange={() => update({ ratePerWeek: rate })}
                />
                {t.rateOptions[String(rate) as keyof typeof t.rateOptions]}
              </label>
            ))}
          </div>
        </fieldset>
      ) : null}

      <div className="rounded-md border border-neutral-200 bg-neutral-50 p-4 dark:border-neutral-700 dark:bg-neutral-800/50">
        <h3 className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
          {t.previewTitle}
        </h3>
        {preview ? (
          <div className="mt-1">
            <p className="text-2xl font-semibold text-neutral-900 dark:text-neutral-50">
              {t.previewKcal(preview.kcal)}
            </p>
            <p className="text-sm text-neutral-600 dark:text-neutral-400">
              {t.previewMacros(preview.protein_g, preview.carbs_g, preview.fat_g)}
            </p>
            {preview.floor_applied ? (
              <p className="mt-1 text-xs text-amber-600 dark:text-amber-400">
                {t.previewFloorApplied}
              </p>
            ) : null}
          </div>
        ) : (
          <p className="mt-1 text-sm text-neutral-500 dark:text-neutral-400">
            {previewError ?? t.previewError}
          </p>
        )}
      </div>
    </div>
  );
}
