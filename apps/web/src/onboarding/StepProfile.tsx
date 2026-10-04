import { BirthDateInput } from "../components/BirthDateInput";
import { Field, inputClass } from "../components/Field";
import { strings } from "../i18n";
import type { ActivityLevel, DietaryPattern, Sex } from "../api/types";
import type { OnboardingDraft } from "./draft";

const t = strings.onboarding.profile;

const activityLevels: ActivityLevel[] = [
  "sedentary",
  "light",
  "moderate",
  "active",
  "very_active",
];

const dietaryPatterns: DietaryPattern[] = ["omnivore", "vegetarian", "vegan", "pescatarian"];

interface StepProfileProps {
  draft: OnboardingDraft;
  update: (patch: Partial<OnboardingDraft>) => void;
  /** Settings hides it: dislikes are preferences there, edited in their own section. */
  showDislikes?: boolean;
}

export function StepProfile({ draft, update, showDislikes = true }: StepProfileProps) {
  return (
    <div className="space-y-5">
      <h2 className="text-lg font-semibold text-neutral-900 dark:text-neutral-50">{t.title}</h2>

      <fieldset>
        <legend className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
          {t.sex}
        </legend>
        <div className="mt-1 flex gap-4">
          {(Object.entries(t.sexOptions) as [Sex, string][]).map(([value, label]) => (
            <label key={value} className="flex items-center gap-1.5 text-sm">
              <input
                type="radio"
                name="sex"
                checked={draft.sex === value}
                onChange={() => update({ sex: value })}
              />
              {label}
            </label>
          ))}
        </div>
      </fieldset>

      <Field label={t.birthDate}>
        <BirthDateInput
          value={draft.birthDate}
          onChange={(birthDate) => update({ birthDate })}
          minAge={16}
          maxAge={100}
        />
      </Field>

      <div className="grid grid-cols-2 gap-4">
        <Field label={t.heightCm}>
          <input
            type="number"
            min={120}
            max={230}
            step="0.1"
            className={inputClass}
            value={draft.heightCm}
            onChange={(e) => update({ heightCm: e.target.value })}
          />
        </Field>
        <Field label={t.weightKg}>
          <input
            type="number"
            min={30}
            max={300}
            step="0.1"
            className={inputClass}
            value={draft.weightKg}
            onChange={(e) => update({ weightKg: e.target.value })}
          />
        </Field>
      </div>

      <fieldset>
        <legend className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
          {t.activityLevel}
        </legend>
        <div className="mt-1 space-y-2">
          {activityLevels.map((level) => (
            <label key={level} className="flex items-start gap-2 text-sm">
              <input
                type="radio"
                name="activityLevel"
                className="mt-0.5"
                checked={draft.activityLevel === level}
                onChange={() => update({ activityLevel: level })}
              />
              <span>{t.activityLevelHints[level]}</span>
            </label>
          ))}
        </div>
      </fieldset>

      <Field label={t.dietaryPattern}>
        <select
          className={inputClass}
          value={draft.dietaryPattern}
          onChange={(e) => update({ dietaryPattern: e.target.value as DietaryPattern })}
        >
          {dietaryPatterns.map((pattern) => (
            <option key={pattern} value={pattern}>
              {t.dietaryPatternOptions[pattern]}
            </option>
          ))}
        </select>
      </Field>

      <div className="grid grid-cols-2 gap-4">
        <Field label={t.allergies} hint={t.allergiesPlaceholder}>
          <input
            type="text"
            className={inputClass}
            value={draft.allergies}
            onChange={(e) => update({ allergies: e.target.value })}
          />
        </Field>
        {showDislikes ? (
          <Field label={t.dislikes} hint={t.dislikesPlaceholder}>
            <input
              type="text"
              className={inputClass}
              value={draft.dislikes}
              onChange={(e) => update({ dislikes: e.target.value })}
            />
          </Field>
        ) : null}
      </div>

      <Field label={t.mealsPerDay}>
        <select
          className={inputClass}
          value={draft.mealsPerDay}
          onChange={(e) => update({ mealsPerDay: Number(e.target.value) })}
        >
          {[3, 4, 5].map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      </Field>

      <Field label={strings.common.withOptional(t.medicalNotes)}>
        <textarea
          className={inputClass}
          rows={2}
          placeholder={t.medicalNotesPlaceholder}
          value={draft.medicalNotes}
          onChange={(e) => update({ medicalNotes: e.target.value })}
        />
      </Field>
    </div>
  );
}

/** Whether step 1's required fields are filled and in range — gates the "下一步" button. Range
 * checks mirror `crates/core/src/profile.rs`'s `validate` (height 120–230, meals_per_day 3–5);
 * the backend re-validates regardless, this is just so the user isn't sent to step 2 with data
 * `save_profile` would reject later. */
export function isStepProfileValid(draft: OnboardingDraft): boolean {
  const height = Number(draft.heightCm);
  const weight = Number(draft.weightKg);
  return (
    draft.sex !== "" &&
    draft.birthDate !== "" &&
    draft.heightCm !== "" &&
    height >= 120 &&
    height <= 230 &&
    draft.weightKg !== "" &&
    weight >= 30 &&
    weight <= 300
  );
}
