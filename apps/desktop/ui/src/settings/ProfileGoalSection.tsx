import { useEffect, useState } from "react";
import {
  getProfile,
  getToday,
  getWeightGoal,
  logWeight,
  saveProfile,
  saveWeightGoal,
} from "../api/commands";
import { commandErrorMessage } from "../api/types";
import { primaryButtonClass } from "../components/buttonStyles";
import { zhCN } from "../i18n/zh-CN";
import { LoadState } from "./LoadState";
import { buildGoal, buildProfile, defaultDraft, type OnboardingDraft } from "../onboarding/draft";
import { StepGoal } from "../onboarding/StepGoal";
import { isStepProfileValid, StepProfile } from "../onboarding/StepProfile";

const t = zhCN.settings;

/** Reuses onboarding's `StepProfile`/`StepGoal` components verbatim (§10.3: "same forms as
 * onboarding") against one shared draft, since `StepGoal`'s live kcal preview already needs the
 * full profile in scope to compute against. Model-related `OnboardingDraft` fields are carried
 * along unused (`StepProfile`/`StepGoal` never read them) rather than forcing a narrower type
 * onto components designed for the wizard's single shared draft. */
export function ProfileGoalSection() {
  const [draft, setDraft] = useState<OnboardingDraft | null>(null);
  const [initialWeightKg, setInitialWeightKg] = useState<number | null>(null);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; message: string } | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([getProfile(), getWeightGoal(), getToday()])
      .then(([profile, goal, today]) => {
        if (cancelled) return;
        const weightKg = today.latest_weight?.kg ?? null;
        setInitialWeightKg(weightKg);
        setDraft({
          ...defaultDraft,
          ...(profile
            ? {
                sex: profile.sex,
                birthDate: profile.birth_date,
                heightCm: String(profile.height_cm),
                activityLevel: profile.activity_level,
                dietaryPattern: profile.dietary_pattern,
                allergies: profile.allergies.join(", "),
                dislikes: profile.dislikes.join(", "),
                mealsPerDay: profile.meals_per_day,
                medicalNotes: profile.medical_notes ?? "",
              }
            : {}),
          weightKg: weightKg !== null ? String(weightKg) : "",
          ...(goal
            ? {
                direction: goal.direction,
                targetWeightKg: goal.target_weight_kg !== null ? String(goal.target_weight_kg) : "",
                ratePerWeek: goal.rate_kg_per_week,
              }
            : {}),
        });
      })
      .catch((e: unknown) => {
        if (!cancelled) setStatus({ ok: false, message: commandErrorMessage(e) });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  function update(patch: Partial<OnboardingDraft>) {
    setDraft((d) => (d ? { ...d, ...patch } : d));
  }

  async function handleSave() {
    if (!draft) return;
    setSaving(true);
    setStatus(null);
    try {
      await saveProfile(buildProfile(draft));
      await saveWeightGoal(buildGoal(draft));
      const weightKg = Number(draft.weightKg);
      if (draft.weightKg.trim() !== "" && weightKg !== initialWeightKg) {
        await logWeight(weightKg);
        setInitialWeightKg(weightKg);
      }
      setStatus({ ok: true, message: t.saveSucceeded });
    } catch (e) {
      setStatus({ ok: false, message: commandErrorMessage(e) });
    } finally {
      setSaving(false);
    }
  }

  if (!draft) {
    return <LoadState error={status && !status.ok ? status.message : null} />;
  }

  return (
    <div className="space-y-8">
      <StepProfile draft={draft} update={update} />
      <StepGoal draft={draft} update={update} />
      <div className="flex items-center gap-3">
        <button
          type="button"
          className={primaryButtonClass}
          disabled={saving || !isStepProfileValid(draft)}
          onClick={() => void handleSave()}
        >
          {saving ? zhCN.common.saving : zhCN.common.save}
        </button>
        {status ? (
          <span
            className={
              status.ok
                ? "text-sm text-emerald-600 dark:text-emerald-400"
                : "text-sm text-red-600 dark:text-red-400"
            }
          >
            {status.message}
          </span>
        ) : null}
      </div>
    </div>
  );
}
