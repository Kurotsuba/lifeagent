import type {
  ActivityLevel,
  DietaryPattern,
  GoalDirection,
  Profile,
  Sex,
  WeightGoal,
} from "../api/types";

/**
 * The wizard's in-progress form state, shared across both steps. Numeric fields are kept
 * as strings while editing (so a field can be legitimately empty mid-edit) and parsed only when
 * building the `Profile`/`WeightGoal` values actually sent to the backend.
 */
export interface OnboardingDraft {
  sex: Sex | "";
  birthDate: string;
  heightCm: string;
  weightKg: string;
  activityLevel: ActivityLevel;
  dietaryPattern: DietaryPattern;
  allergies: string;
  dislikes: string;
  mealsPerDay: number;
  medicalNotes: string;

  direction: GoalDirection;
  targetWeightKg: string;
  ratePerWeek: number;
}

export const defaultDraft: OnboardingDraft = {
  sex: "",
  birthDate: "",
  heightCm: "",
  weightKg: "",
  activityLevel: "moderate",
  dietaryPattern: "omnivore",
  allergies: "",
  dislikes: "",
  mealsPerDay: 3,
  medicalNotes: "",

  direction: "maintain",
  targetWeightKg: "",
  ratePerWeek: 0.5,
};

/** Splits a comma-separated field into trimmed, non-empty entries. */
export function splitList(value: string): string[] {
  return value
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/** Builds the `Profile` the backend expects from the draft's step-1 fields. Only meaningful
 * once `isStepProfileValid` (see `StepProfile.tsx`) holds — `sex` in particular is cast past its
 * `""` placeholder state, which that check has already ruled out by the time this is called. */
export function buildProfile(draft: OnboardingDraft): Profile {
  return {
    display_name: null,
    sex: draft.sex as Profile["sex"],
    birth_date: draft.birthDate,
    height_cm: Number(draft.heightCm),
    activity_level: draft.activityLevel,
    dietary_pattern: draft.dietaryPattern,
    allergies: splitList(draft.allergies),
    dislikes: splitList(draft.dislikes),
    medical_notes: draft.medicalNotes.trim() === "" ? null : draft.medicalNotes.trim(),
    meals_per_day: draft.mealsPerDay,
  };
}

/** Builds the `WeightGoal` the backend expects from the draft's step-2 fields. `rate_kg_per_week`
 * is sent even for `maintain`, where the backend ignores and doesn't validate it (§7.4) — simpler
 * than threading a `null` case through for a value that's harmless either way. */
export function buildGoal(draft: OnboardingDraft): WeightGoal {
  const target = draft.targetWeightKg.trim();
  return {
    direction: draft.direction,
    target_weight_kg: target === "" ? null : Number(target),
    rate_kg_per_week: draft.ratePerWeek,
  };
}
