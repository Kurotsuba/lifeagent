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

export interface SettingsDto {
  plan_time: string; // "HH:MM"
  day_boundary: string; // "HH:MM"
  notifications_enabled: boolean;
  autostart: boolean;
}

// ---- Chat --------------------------------------------------------------------------------

export type MessageRole = "user" | "assistant" | "tool";

export interface ChatTurn {
  reply: string;
  activities: string[];
}

export interface HistoryMessage {
  role: MessageRole;
  content: string | null;
  created_at: string;
}

// ---- Today / plan --------------------------------------------------------------------------

export interface WeightPoint {
  kg: number;
  logical_date: string;
  occurred_at: string;
}

/** `{kcal, protein_g, carbs_g, fat_g, fiber_g}` — the shape every diet tool result and card
 * uses (`crates/plugin-diet/src/model.rs`'s `Macros::to_json`). Distinct from `NutritionTargets`
 * above, which is onboarding's richer preview shape (bmr/tdee/delta/floor_applied). */
export interface DietMacros {
  kcal: number;
  protein_g: number;
  carbs_g: number;
  fat_g: number;
  fiber_g: number;
}

export type MealType = "breakfast" | "lunch" | "dinner" | "snack";

/** `"food"`: a per-100 g item, as Phase 1 always logged. `"dish"`: a per-serving chain/menu item
 * (`docs/DISH_NUTRITION.md` §2, §6). Absent on events logged before dishes existed — treat a
 * missing `kind` as `"food"`, matching the Rust side's `#[serde(default)]`. */
export type MealItemKind = "food" | "dish";

export interface DietMealItem {
  name: string;
  kind: MealItemKind;
  /** Dish items only. */
  brand: string | null;
  /** `null` for a dish item whose `serving_g` is unknown. */
  grams: number | null;
  /** Dish items only. */
  servings: number | null;
  kcal: number;
  protein_g: number;
  estimated: boolean;
  /** The winning source's display name at logging time (`docs/NUTRITION_SOURCES.md` §5), for a
   * tooltip. `null` for meals logged before this field existed. */
  source_name: string | null;
}

/** One logged meal, as `cards.diet.meals` lists it (§8.8). */
export interface DietLoggedMeal {
  entry_id: string;
  meal_type: MealType;
  time: string; // "HH:MM"
  items: DietMealItem[];
  total: DietMacros;
}

/** `TodayView.cards["diet"]` (§8.8): `null` total/target when there's no profile/weight yet. */
export interface DietCard {
  date: string;
  total: DietMacros | null;
  target: DietMacros | null;
  meals: DietLoggedMeal[];
}

/** One planned meal inside a plan's `diet` section (§8.10's `finalize`). */
export interface DietPlanMeal {
  meal_type: MealType;
  title: string;
  prep_note: string | null;
  items: DietMealItem[];
  total: DietMacros;
}

/** `StoredPlan.content.sections["diet"]`. */
export interface DietPlanSection {
  targets: DietMacros;
  total: DietMacros;
  meals: DietPlanMeal[];
  tips: string[];
}

export type PlanStatus = "proposed" | "accepted";
export type PlanTrigger = "schedule" | "manual" | "revision";

export interface PlanContent {
  summary: string;
  /** Plugin id -> that plugin's finalized section data; only `"diet"` exists in Phase 1. */
  sections: Record<string, unknown>;
  disclaimer: string | null;
}

export interface StoredPlan {
  plan_date: string;
  version: number;
  status: PlanStatus;
  trigger: PlanTrigger;
  instruction: string | null;
  content: PlanContent;
  warnings: string[];
  created_at: string;
}

export interface TodayView {
  logical_date: string;
  /** Plugin id -> that plugin's `today_card`; only `"diet"` exists in Phase 1. */
  cards: Record<string, unknown>;
  today_plan: StoredPlan | null;
  tomorrow_plan: StoredPlan | null;
  latest_weight: WeightPoint | null;
}

/** `TodayView.cards["diet"]`, narrowed from the untyped `Record`. */
export function dietCard(view: TodayView): DietCard | null {
  const card = view.cards["diet"];
  return (card as DietCard | undefined) ?? null;
}

/** `StoredPlan.content.sections["diet"]`, narrowed from the untyped `Record`. */
export function dietPlanSection(plan: StoredPlan): DietPlanSection | null {
  const section = plan.content.sections["diet"];
  return (section as DietPlanSection | undefined) ?? null;
}

// ---- Nutrition sources / batch import (`docs/NUTRITION_SOURCES.md`) ------------------------

/** `SourceKind`'s serde names: manual is still `"user"`, so old `diet.meal_logged` payloads and
 * this type agree (`crates/plugin-diet/src/model.rs`). */
export type SourceKind = "user" | "dataset" | "llm_estimate";

export interface DatasetOrigin {
  file_name: string;
  format: "csv" | "json";
  imported_at: string;
  rows: number;
}

export interface SourceInfo {
  id: string;
  kind: SourceKind;
  name: string;
  priority: number;
  enabled: boolean;
  food_count: number;
  dish_count: number;
  origin: DatasetOrigin | null;
}

export interface Food {
  id: string;
  name: string;
  name_key: string;
  per_100g: DietMacros;
  source_id: string;
  source_kind: SourceKind;
  source_name: string;
  use_count: number;
}

/** A per-serving chain/menu item (`docs/DISH_NUTRITION.md` §4.1). `brand === ""` is brandless
 * (e.g. a canteen dataset). `serving_g` is `null` when the chain doesn't publish it. */
export interface Dish {
  id: string;
  brand: string;
  brand_key: string;
  name: string;
  name_key: string;
  serving_g: number | null;
  per_serving: DietMacros;
  source_id: string;
  source_kind: SourceKind;
  source_name: string;
  use_count: number;
}

export interface RowIssue {
  line: number;
  name: string | null;
  message: string;
}

/** `"100g"`: a food row. `"serving"`: a dish row, with `brand`/`serving_g` filled in
 * (`docs/DISH_NUTRITION.md` §7.3). */
export interface ImportRowView {
  basis: "100g" | "serving";
  brand: string | null;
  serving_g: number | null;
  name: string;
  kcal: number;
  protein_g: number;
  carbs_g: number;
  fat_g: number;
  fiber_g: number;
  aliases: string[];
}

/** `preview_food_import`'s result: the parse/shadowing preview, plus the token `commit_food_import`
 * / `cancel_food_import` need (§8.2 "Preview → commit token"). */
export interface ImportPreview {
  format: "csv" | "json";
  suggested_name: string;
  total_rows: number;
  valid_food_rows: number;
  valid_dish_rows: number;
  /** `valid_food_rows + valid_dish_rows`. */
  valid_rows: number;
  errors: RowIssue[];
  error_count: number;
  warnings: RowIssue[];
  warning_count: number;
  shadowed_by_manual: number;
  replaces_estimates: number;
  dish_shadowed_by_manual: number;
  /** Distinct dish brands in the file, non-empty, max 20. */
  brands: string[];
  sample: ImportRowView[];
  token: string;
}

export interface ImportReport {
  source: SourceInfo;
  imported: number;
  imported_dishes: number;
  skipped: number;
}

export interface MemoryNote {
  id: string;
  note: string;
  created_at: string;
}

export interface RoleUsage {
  role: Role;
  prompt_tokens: number;
  completion_tokens: number;
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
