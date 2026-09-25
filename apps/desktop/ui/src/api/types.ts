/**
 * TypeScript shapes mirroring the Rust types that cross the Tauri command boundary
 * (`docs/PHASE1_SPEC.md` §7/§10.2). Field names and enum values are snake_case, matching the
 * Rust `serde` derives exactly (see CLAUDE.md: "snake_case field names for everything crossing
 * a boundary"). No logic here — this file is data shapes only.
 */

export type Sex = "male" | "female";

export type ActivityLevel =
  | "sedentary"
  | "light"
  | "moderate"
  | "active"
  | "very_active";

export type DietaryPattern = "omnivore" | "vegetarian" | "vegan" | "pescatarian";

export interface Profile {
  display_name: string | null;
  sex: Sex;
  birth_date: string; // "YYYY-MM-DD"
  height_cm: number;
  activity_level: ActivityLevel;
  dietary_pattern: DietaryPattern;
  allergies: string[];
  dislikes: string[];
  medical_notes: string | null;
  meals_per_day: number;
}

export type GoalDirection = "lose" | "maintain" | "gain";

export interface WeightGoal {
  direction: GoalDirection;
  target_weight_kg: number | null;
  rate_kg_per_week: number;
}

export interface NutritionTargets {
  kcal: number;
  protein_g: number;
  carbs_g: number;
  fat_g: number;
  fiber_g: number;
  bmr: number;
  tdee: number;
  delta: number;
  floor_applied: boolean;
}

/** `open_ai_compatible`, not `openai_compatible` — verified against the actual Rust serde
 * output (`ProviderKind::OpenAiCompatible` splits at each capital, like any other snake_case
 * rename), not guessed from the natural-English spelling. */
export type ProviderKind = "open_ai_compatible" | "anthropic";

export type Effort = "low" | "medium" | "high";

export interface ProviderConfig {
  id: string;
  kind: ProviderKind;
  base_url: string;
}

export interface RoleBinding {
  provider_id: string;
  model: string;
  temperature: number | null;
  effort: Effort | null;
}

export interface ModelsConfig {
  providers: ProviderConfig[];
  parser: RoleBinding;
  planner: RoleBinding;
  conversation: RoleBinding;
}

export type Role = "parser" | "planner" | "conversation";

export interface TestModelResult {
  ok: boolean;
  latency_ms: number;
  error: string | null;
}

export interface AppState {
  onboarding_complete: boolean;
  models_configured: boolean;
  logical_date: string;
}

export type CommandErrorCode =
  | "not_configured"
  | "auth"
  | "llm"
  | "invalid_input"
  | "not_found"
  | "internal";

export interface CommandError {
  code: CommandErrorCode;
  message: string;
}

/** A rejected `invoke()` call surfaces the Rust `CommandError` value directly as the rejection
 * reason (not wrapped in a JS `Error`) — this narrows `unknown` down to it. */
export function isCommandError(value: unknown): value is CommandError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value &&
    typeof (value as CommandError).code === "string" &&
    typeof (value as CommandError).message === "string"
  );
}

/** The message from a rejected `invoke()` call, whether it's a `CommandError` or something else
 * (e.g. an argument (de)serialization failure, which Tauri surfaces as a bare string). */
export function commandErrorMessage(error: unknown): string {
  if (isCommandError(error)) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return String(error);
}
