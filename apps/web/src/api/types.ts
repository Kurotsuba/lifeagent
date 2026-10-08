/**
 * TypeScript shapes mirroring the Rust types that cross the HTTP command boundary
 * (`docs/PHASE1_SPEC.md` §7/§10.2). Field names and enum values are snake_case, matching the
 * Rust `serde` derives exactly (see CLAUDE.md: "snake_case field names for everything crossing
 * a boundary"). Data shapes only, plus the `CommandError` guards at the end.
 */

import { describeError } from "../i18n";

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

export interface AppState {
  onboarding_complete: boolean;
  models_configured: boolean;
  logical_date: string;
  /** Ids of the plugins switched on (e.g. `"training"` once enabled). */
  enabled_plugins: string[];
  /** Whether to show the Activity tab; the log records either way. */
  activity_tab_visible: boolean;
}

/** `lifeagent_core::app::PluginInfo`. */
export interface PluginInfo {
  id: string;
  name: string;
  /** The user can switch it on or off; optional plugins start off. */
  optional: boolean;
  enabled: boolean;
}

export interface SettingsDto {
  plan_time: string; // "HH:MM"
  day_boundary: string; // "HH:MM"
}

// ---- Chat --------------------------------------------------------------------------------

export type MessageRole = "user" | "assistant" | "tool";

/** What a tool call did (`ToolSummary` in `crates/core/src/plugin.rs`), worded by the UI's
 * `toolSummaries` table. Kinds start with the owning plugin's id. */
export type ToolSummary =
  | { kind: "core_plan_viewed"; date: string }
  | { kind: "core_plan_generated"; date: string; version: number }
  | { kind: "core_plan_revised"; date: string; version: number }
  | { kind: "core_weight_logged"; kg: number }
  | { kind: "core_allergies_updated"; allergies: string[] }
  | {
      kind: "core_preference_added";
      preference_kind: PreferenceKind;
      text: string;
      valid_from: string | null;
      valid_until: string | null;
    }
  | { kind: "core_preference_removed" }
  | { kind: "core_entry_deleted"; event_type: string }
  | { kind: "diet_meal_logged"; meal_type: MealType; kcal: number }
  | { kind: "diet_intake_viewed"; date: string }
  | { kind: "diet_food_updated"; name: string }
  | { kind: "diet_dish_updated"; name: string }
  | { kind: "diet_alias_added"; alias: string; food: string }
  | { kind: "diet_alias_removed"; alias: string }
  | { kind: "stock_added"; names: string[] }
  | { kind: "stock_viewed" }
  | { kind: "stock_updated"; name: string }
  | { kind: "training_session_logged"; title: string; minutes: number }
  | { kind: "training_log_viewed"; start_date: string; end_date: string }
  | { kind: "training_day_preference_set"; date: string; cleared: boolean }
  | { kind: "training_preferences_updated" }
  | { kind: "multimember_members_viewed"; count: number }
  | { kind: "multimember_member_added"; name: string }
  | { kind: "multimember_member_updated"; name: string }
  | { kind: "multimember_member_archived"; name: string }
  | {
      kind: "multimember_attendance_set";
      name: string;
      meal_type: MainMealType;
      present: boolean;
      /** `null`: the member's usual attendance on the given weekdays. */
      date: string | null;
    }
  | { kind: "multimember_attendance_cleared"; name: string; meal_type: MainMealType; date: string }
  | { kind: "multimember_dislike_added"; name: string; text: string }
  | { kind: "multimember_dislike_removed" };

/** Why a turn has no model reply (`TurnNotice` in `crates/core/src/app.rs`). */
export type TurnNotice = "empty_reply" | "too_many_steps";

export interface ChatTurn {
  /** Empty when `notice` is set. */
  reply: string;
  notice: TurnNotice | null;
  activities: ToolSummary[];
  /** Tool calls from this turn that wait for the user's approval. */
  pending_actions: PendingAction[];
}

// ---- Approvals (`crates/core/src/approvals.rs`) --------------------------------------------

export type ActionStatus = "pending" | "approved" | "rejected" | "failed" | "expired";

/** What an action will do (`approval_preview` in core and the plugins), worded by the UI. */
export type ActionPreview =
  | { kind: "update_allergies"; add: string[]; remove: string[] }
  | {
      kind: "add_preference";
      preference_kind: PreferenceKind;
      text: string;
      valid_from: string | null;
      valid_until: string | null;
      permanent: boolean;
    }
  | { kind: "remove_preference"; preference_kind: PreferenceKind; text: string }
  | {
      kind: "delete_past_entry";
      event_type: string;
      logical_date: string;
      description: string | null;
    }
  | { kind: "archive_member"; name: string }
  | { kind: "update_member_allergies"; name: string; allergies: string[] }
  | { kind: "add_member"; name: string; allergies: string[] }
  | {
      kind: "update_training_notes";
      notes: string;
      previous_notes: string;
      /** The same call also changes goal, days, minutes or equipment. */
      other_changes: boolean;
    }
  | {
      kind: "update_nutrition";
      name: string;
      brand: string | null;
      basis: NutritionBasis;
      /** The current values; null for a new food or dish. */
      before: DietMacros | null;
      after: DietMacros;
      reasons: NutritionChangeReason[];
    }
  | { kind: "add_food_alias"; alias: string; food: string }
  | { kind: "remove_food_alias"; alias: string; target: string };

export type NutritionBasis = "per_100g" | "per_serving";
export type NutritionChangeReason = "large_change" | "kcal_mismatch";

export interface PendingAction {
  id: string;
  tool_name: string;
  preview: ActionPreview;
  status: ActionStatus;
  /** What the tool did once approved. */
  result: ToolSummary | null;
  /** Why an approved action failed. */
  result_summary: string | null;
  created_at: string;
  resolved_at: string | null;
}

// ---- Activity log (`crates/core/src/audit.rs`, `docs/AUDIT_LOG_SPEC.md`) --------------------

export type ActivityActor = "user" | "scheduler";
export type RunTrigger =
  | "chat_turn"
  | "plan_scheduled"
  | "plan_manual"
  | "plan_revision"
  | "action_approved"
  | "action_rejected";
export type RunStatus = "running" | "succeeded" | "needs_approval" | "failed" | "interrupted";
export type ModelRole = "conversation" | "parser" | "planner";
export type ToolCallOutcome = "ran" | "invalid_args" | "not_found" | "failed";
export type GuardrailRule = "invalid_args" | "unknown_tool" | "token_budget" | "iteration_cap";

export interface RunOutcome {
  model_calls: number;
  tool_calls: number;
  tokens: number;
  latency_ms: number;
  guardrails: number;
  pending_actions: number;
  plan: { date: string; version: number } | null;
}

export interface ActivityRunInfo {
  id: string;
  trigger: RunTrigger;
  actor: ActivityActor;
  status: RunStatus;
  input_excerpt: string | null;
  /** Null while the run is still going. */
  outcome: RunOutcome | null;
  error_code: string | null;
  logical_date: string;
  started_at: string;
  finished_at: string | null;
}

export interface RunPage {
  runs: ActivityRunInfo[];
  /** Pass as `before` for the next page; null at the end. */
  next_before: string | null;
}

/** One step of a run, by kind (`StepKind` and the details in `docs/AUDIT_LOG_SPEC.md` §4.2). */
export type ActivityStepDetail =
  | {
      kind: "model_call";
      detail: {
        role: ModelRole;
        provider: string;
        model: string;
        prompt_tokens: number;
        completion_tokens: number;
        latency_ms: number;
        ok: boolean;
      };
    }
  | {
      kind: "tool_call";
      detail: {
        tool: string;
        arguments: unknown;
        outcome: ToolCallOutcome;
        summary?: ToolSummary;
        error?: string;
      };
    }
  | {
      kind: "approval_requested";
      detail: { tool: string; action_id: string; arguments: unknown; preview: ActionPreview };
    }
  | {
      kind: "approval_resolved";
      detail: {
        tool: string;
        action_id: string;
        decision: ActionStatus;
        requested_in: string | null;
        result?: ToolSummary;
      };
    }
  | {
      kind: "guardrail";
      detail: {
        rule: GuardrailRule;
        tool?: string;
        limit?: number;
        used?: number;
        budget?: number;
      };
    }
  | {
      kind: "plan_validation";
      detail: {
        date: string;
        sections: string[];
        violations: StoredPlanNote[];
        retried: boolean;
        violations_left: number;
      };
    }
  | { kind: "plan_saved"; detail: { date: string; version: number; trigger: string } };

export type ActivityStep = ActivityStepDetail & {
  id: string;
  created_at: string;
  /** For `approval_requested`: what became of the action since. */
  action_status: ActionStatus | null;
};

export interface ActivityRun {
  run: ActivityRunInfo;
  steps: ActivityStep[];
}

export interface ActivitySummary {
  days: number;
  runs_by_user: number;
  runs_by_scheduler: number;
  approvals_requested: number;
  approvals_approved: number;
  approvals_rejected: number;
  guardrails: number;
  tokens: number;
  tokens_last_24h: number;
  daily_token_budget: number | null;
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
  source_kind: SourceKind;
  /** The winning source's display name at logging time (`docs/NUTRITION_SOURCES.md` §5), for a
   * tooltip. `null` for meals logged before this field existed. */
  source_name: string | null;
  /** Planned items of a shared meal only: grams to cook for the whole table (computed in Rust).
   * `grams`, `kcal` and `protein_g` stay the user's own portion. */
  table_grams?: number;
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
  /** Shared meals only: each other member's portion (`docs/MULTIMEMBER_SPEC.md` §6.3). */
  servings?: GuestServing[];
  /** Shared meals only: portions cooked in total, the user's included. */
  portions?: number;
}

/** One other member's portion of a shared meal; `total` is computed in Rust. */
export interface GuestServing {
  member_id: string;
  name: string;
  scale: number;
  total: DietMacros;
}

/** `StoredPlan.content.sections["diet"]`. */
export interface DietPlanSection {
  targets: DietMacros;
  /** kcal the targets were raised by for the day's planned training (0 without one). Absent in
   * plans made before the training plugin existed. */
  exercise_kcal_added?: number;
  total: DietMacros;
  meals: DietPlanMeal[];
  tips: string[];
  /** Plans with shared meals: each guest's maintenance estimate, by member id. */
  guest_estimates?: Record<string, DietMacros>;
}

export type PlanStatus = "proposed" | "accepted";
export type PlanTrigger = "schedule" | "manual" | "revision";

export interface PlanContent {
  summary: string;
  /** Plugin id -> that plugin's finalized section data (`"diet"`, and `"training"` when that
   * plugin is on). */
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
  warnings: StoredPlanNote[];
  created_at: string;
}

export type ExerciseProblem =
  | "name_empty"
  | "name_too_long"
  | "sets"
  | "reps"
  | "weight"
  | "duration"
  | "distance";

/** A plan warning or violation's data (`PlanNote` in `crates/core/src/plugin.rs`), worded by
 * the UI's `planNotes` table. Kinds start with the owning plugin's id. Numbers are the ones Rust
 * computed and rounded. */
export type PlanNoteData =
  | { kind: "core_section_missing"; section: string }
  | { kind: "diet_invalid_structure"; reason: string }
  | { kind: "diet_item_unnamed" }
  | { kind: "diet_grams_out_of_range"; name: string; grams: number; min: number; max: number }
  | { kind: "diet_allergen"; meal_type: MealType; meal_title: string; name: string; allergen: string }
  | { kind: "diet_disliked"; name: string; term: string }
  | { kind: "diet_main_meal_count"; count: number }
  | { kind: "diet_meal_count"; count: number; max: number }
  | { kind: "diet_kcal_off_target"; actual: number; target: number; low: number; high: number }
  | { kind: "diet_protein_low"; actual: number; target: number }
  | { kind: "diet_fat_off_target"; actual: number; target: number }
  | { kind: "diet_carbs_off_target"; actual: number; target: number }
  | { kind: "diet_fiber_low"; actual: number; target: number }
  | {
      kind: "diet_guest_allergen";
      meal_type: MealType;
      meal_title: string;
      name: string;
      guest: string;
      allergen: string;
    }
  | { kind: "diet_guest_disliked"; name: string; guest: string; term: string }
  | {
      kind: "diet_shared_pattern";
      meal_type: MealType;
      meal_title: string;
      pattern: DietaryPattern;
      name: string;
      term: string;
    }
  | { kind: "diet_guest_scale_clamped"; guest: string; scale: number; min: number; max: number }
  | { kind: "stock_invalid_structure"; reason: string }
  | { kind: "stock_not_in_stock"; name: string }
  | { kind: "stock_wrong_unit"; name: string; unit: StockUnit; planned_unit: StockUnit }
  | { kind: "stock_over_quantity"; name: string; amount: number; quantity: number; unit: StockUnit }
  | { kind: "stock_expiring_unused"; name: string; expires_on: string }
  | { kind: "training_invalid_structure"; reason: string }
  | { kind: "training_invalid_exercise"; name: string; problem: ExerciseProblem }
  | { kind: "training_rest_day_too_hard" }
  | { kind: "training_no_exercises" }
  | { kind: "training_too_long"; minutes: number; max: number }
  | { kind: "training_over_usual"; minutes: number; usual: number }
  | { kind: "training_hard_after_break" }
  | { kind: "training_big_jump"; longest: number }
  | { kind: "training_third_hard_day" }
  | { kind: "multimember_out_of_age_range"; name: string; age: number; min: number; max: number };

/** `text` is the server's English wording, for a kind this build doesn't know; `unmet` marks a
 * violation the planner's retry didn't fix. */
export type PlanNote = PlanNoteData & { text: string; unmet?: boolean };

/** Plans saved before notes were data hold plain zh-CN strings. */
export type StoredPlanNote = PlanNote | string;

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

// ---- Members (`crates/plugin-multimember`) ---------------------------------------------------

export type MainMealType = "breakfast" | "lunch" | "dinner";

/** `MemberProfile`: everything the user enters about another member. */
export interface MemberProfile {
  display_name: string;
  sex: Sex;
  /** "YYYY-MM-DD". */
  birth_date: string;
  height_cm: number;
  weight_kg: number;
  activity_level: ActivityLevel;
  dietary_pattern: DietaryPattern;
  allergies: string[];
}

export interface Member extends MemberProfile {
  id: string;
  created_at: string;
  updated_at: string;
}

/** A weekly default: whether the member usually eats `meal_type` on `weekday` (0 = Monday). */
export interface AttendanceSlot {
  weekday: number;
  meal_type: MainMealType;
  present: boolean;
}

export interface MemberDislike {
  id: string;
  member_id: string;
  text: string;
  valid_from: string | null;
  valid_until: string | null;
  created_at: string;
}

export interface MemberView extends Member {
  age: number;
  /** 21 slots: every weekday × main meal. */
  attendance: AttendanceSlot[];
  dislikes: MemberDislike[];
}

export interface DaySlot {
  member_id: string;
  meal_type: MainMealType;
  present: boolean;
  /** A one-off change decides it, rather than the weekly default. */
  overridden: boolean;
  note: string | null;
}

export interface MembersView {
  today: string;
  members: MemberView[];
  /** The next 7 days, today first, resolved. */
  week: { date: string; slots: DaySlot[] }[];
}

// ---- Stock (`crates/plugin-stock`) ----------------------------------------------------------

/** `Unit` in `crates/plugin-stock/src/model.rs`. */
export type StockUnit = "g" | "ml" | "piece";

export interface StockItem {
  id: string;
  name: string;
  quantity: number;
  unit: StockUnit;
  /** "YYYY-MM-DD" best-before, if the user gave one. */
  expires_on: string | null;
  updated_at: string;
}

/** An amount a logged meal took out of stock. */
export interface StockUse {
  event_id: string;
  logical_date: string;
  item_name: string;
  unit: StockUnit;
  amount: number;
  used_at: string;
}

export interface StockSettings {
  auto_consume: boolean;
  plan_from_stock: boolean;
}

export interface ShoppingItem {
  name: string;
  amount: number;
  unit: StockUnit;
  /** How much of it is in stock now (bought since the plan was made, or never needed). */
  in_stock: number;
  status: ShoppingStatus;
}

/** `ShoppingStatus` in `crates/plugin-stock/src/shopping.rs`: `done` put the line's amount into
 * stock; `cancelled` dropped it from the list. */
export type ShoppingStatus = "open" | "done" | "cancelled";

/** Names one line of one plan version's shopping list. */
export interface ShoppingLineRef {
  plan_date: string;
  plan_version: number;
  name: string;
  unit: StockUnit;
}

/** `get_stock`: everything the Stock tab shows. */
export interface StockView {
  logical_date: string;
  items: StockItem[];
  recent_uses: StockUse[];
  settings: StockSettings;
  /** From tomorrow's plan, or else today's; `null` without one. */
  shopping: { plan_date: string; plan_version: number; items: ShoppingItem[] } | null;
}

export interface NewStockItem {
  name: string;
  quantity: number;
  unit: StockUnit;
  expires_on: string | null;
}

// ---- Training (`crates/plugin-training`) ----------------------------------------------------

export type ExerciseKind = "strength" | "cardio" | "mobility" | "sport" | "other";
export type Intensity = "low" | "moderate" | "high";
export type TimeOfDay = "morning" | "midday" | "afternoon" | "evening";
export type TrainingGoal = "general_fitness" | "strength" | "muscle" | "endurance" | "fat_loss";

export interface Exercise {
  name: string;
  kind: ExerciseKind;
  intensity: Intensity;
  sets: number | null;
  reps: number | null;
  weight_kg: number | null;
  duration_min: number | null;
  distance_km: number | null;
  note: string | null;
}

/** One logged session (`log::SessionView`). `duration_min`/`exercise_kcal` are computed by
 * the app; `exercise_kcal` is `null` when no weight was recorded. */
export interface TrainingSession {
  entry_id: string;
  date: string;
  time: string; // "HH:MM"
  title: string;
  raw_text: string;
  exercises: Exercise[];
  duration_min: number;
  exercise_kcal: number | null;
  intensity: Intensity;
  rpe: number | null;
  notes: string | null;
}

export interface TrainingDay {
  date: string;
  sessions: TrainingSession[];
  minutes: number;
  kcal: number;
}

/** `StoredPlan.content.sections["training"]` (the training plugin's `validate::finalize`). */
export interface TrainingPlanSection {
  rest_day: boolean;
  title: string;
  focus: string | null;
  time_of_day: TimeOfDay | null;
  exercises: (Exercise & { minutes: number })[];
  duration_min: number;
  exercise_kcal: number | null;
  intensity: Intensity;
  tips: string[];
}

export interface TrainingPreferences {
  goal: TrainingGoal;
  days_per_week: number;
  session_minutes: number;
  equipment: string;
  notes: string;
}

export interface TrainingView {
  logical_date: string;
  preferences: TrainingPreferences;
  /** Last 7 days up to today, oldest first. */
  days: TrainingDay[];
  week_sessions: number;
  week_minutes: number;
  week_kcal: number;
  today_plan: TrainingPlanSection | null;
  tomorrow_date: string;
  tomorrow_plan: TrainingPlanSection | null;
  tomorrow_has_plan: boolean;
  tomorrow_preference: string | null;
}

/** `StoredPlan.content.sections["training"]`, narrowed from the untyped `Record`; `null` when
 * absent or invalid. */
export function trainingPlanSection(plan: StoredPlan): TrainingPlanSection | null {
  const section = plan.content.sections["training"];
  return section && typeof section === "object" ? (section as TrainingPlanSection) : null;
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
  message: ImportMessage;
}

/** The first nutrition rule a row broke (`MacroViolation`, `crates/plugin-diet/src/foods.rs`). */
export type MacroViolation =
  | { rule: "negative" }
  | { rule: "fiber_exceeds_carbs"; fiber_g: number; carbs_g: number }
  | { rule: "macros_exceed_100g"; sum: number }
  | { rule: "kcal_per_100g_too_high"; got: number }
  | { rule: "kcal_per_serving_too_high"; got: number }
  | { rule: "serving_not_positive" }
  | { rule: "macros_exceed_serving"; sum: number; serving_g: number }
  | { rule: "kcal_exceeds_serving"; kcal: number; serving_g: number };

/** An import error or warning (`ImportMessage`, `crates/plugin-diet/src/import.rs`). */
export type ImportMessage =
  | { kind: "file_too_large"; max_mb: number }
  | { kind: "not_utf8" }
  | { kind: "csv_header"; error: string }
  | { kind: "csv_row"; line: number; error: string }
  | { kind: "json_parse"; error: string }
  | { kind: "json_item_not_object"; line: number }
  | { kind: "json_missing_rows" }
  | { kind: "json_foods_not_array" }
  | { kind: "json_dishes_not_array" }
  | { kind: "json_wrong_shape" }
  | { kind: "too_many_rows"; max: number }
  | { kind: "no_valid_rows" }
  | { kind: "missing_file_content" }
  | { kind: "import_expired" }
  | { kind: "source_name_length" }
  | { kind: "not_a_dataset" }
  | { kind: "source_not_found" }
  | { kind: "unknown_basis"; value: string }
  | { kind: "not_a_number"; field: string }
  | { kind: "missing_field"; field: string }
  | { kind: "missing_energy" }
  | { kind: "not_positive"; field: string; got: number }
  | { kind: "invalid_macros"; violation: MacroViolation }
  | { kind: "kcal_kj_mismatch"; kcal: number; kcal_from_kj: number }
  | { kind: "energy_mismatch"; kcal: number; estimated: number }
  | { kind: "too_many_aliases"; max: number }
  | { kind: "brand_on_food_row" }
  | { kind: "food_name_empty" }
  | { kind: "food_name_too_long"; max: number }
  | { kind: "dish_name_empty" }
  | { kind: "dish_name_too_long"; max: number }
  | { kind: "brand_too_long"; max: number }
  | { kind: "duplicate_food" }
  | { kind: "duplicate_dish" }
  | { kind: "duplicate_food_alias"; alias: string }
  | { kind: "duplicate_dish_alias"; alias: string }
  | { kind: "trace_values"; rows: number }
  | { kind: "unknown_columns"; columns: string[] };

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

/** `dislike`: one food to avoid (`text` is its name); `general`: any other wish. */
export type PreferenceKind = "dislike" | "general";

/** `valid_from`/`valid_until` are inclusive logical dates ("YYYY-MM-DD"); `null` is open. */
export interface Preference {
  id: string;
  kind: PreferenceKind;
  text: string;
  valid_from: string | null;
  valid_until: string | null;
  created_at: string;
}

/** Without `valid_until` the backend ends it one month after its start, unless `permanent`. */
export interface NewPreference {
  kind: PreferenceKind;
  text: string;
  valid_from: string | null;
  valid_until: string | null;
  permanent: boolean;
}

export type CommandErrorCode =
  | "not_configured"
  | "auth"
  | "llm"
  | "invalid_input"
  | "not_found"
  | "internal"
  // Web build only (`crates/server`): accounts and sessions.
  | "unauthorized"
  | "invalid_credentials"
  | "username_taken"
  | "forbidden"
  | "rate_limited"
  | "budget_exceeded"
  | "action_not_pending";

/** What went wrong (`ErrorDetail`, `crates/api/src/error.rs`), worded by `describeError`. */
export type ErrorDetail =
  | { kind: "not_configured" }
  | { kind: "invalid_input" }
  | { kind: "not_found" }
  | { kind: "nothing_to_plan" }
  | { kind: "plan_busy" }
  | { kind: "internal" }
  | { kind: "llm_auth" }
  | { kind: "llm_rate_limited" }
  | { kind: "llm_timeout" }
  | { kind: "llm_unavailable" }
  | { kind: "token_budget_exceeded" }
  | { kind: "action_not_pending" }
  | { kind: "height_out_of_range"; got: number }
  | { kind: "age_out_of_range"; got: number }
  | { kind: "meals_per_day_out_of_range"; got: number }
  | { kind: "goal_rate_out_of_range"; got: number }
  | { kind: "goal_target_out_of_range"; got: number }
  | { kind: "weight_out_of_range"; got: number }
  | { kind: "note_empty" }
  | { kind: "note_too_long"; got: number; max: number }
  | { kind: "plan_time_before_day_boundary"; plan_time: string; day_boundary: string }
  | { kind: "instruction_empty" }
  | { kind: "preference_dates_reversed"; valid_from: string; valid_until: string }
  | { kind: "food_data"; detail: ImportMessage }
  | { kind: "member_name_invalid" }
  | { kind: "member_name_taken" }
  | { kind: "member_age_out_of_range"; got: number }
  | { kind: "member_allergies_invalid" }
  | { kind: "too_many_members"; max: number }
  | { kind: "attendance_invalid" }
  | { kind: "unauthorized" }
  | { kind: "invalid_username" }
  | { kind: "invalid_password" }
  | { kind: "too_many_attempts" }
  | { kind: "too_many_requests" }
  | { kind: "invalid_credentials" }
  | { kind: "username_taken" }
  | { kind: "bad_request" }
  | { kind: "unknown_endpoint" };

export interface CommandError {
  code: CommandErrorCode;
  /** zh-CN, from the server; shown only when `detail` is missing or of an unknown kind. */
  message: string;
  /** Missing on errors made on this side (e.g. the network being down). */
  detail?: ErrorDetail;
}

/** A rejected command surfaces the Rust `CommandError` value directly as the rejection reason
 * (not wrapped in a JS `Error`), from `invoke()` and the web transport alike — this narrows
 * `unknown` down to it. */
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

/** The message from a rejected command call, whether it's a `CommandError` or something else. */
export function commandErrorMessage(error: unknown): string {
  if (isCommandError(error)) {
    return describeError(error);
  }
  if (typeof error === "string") {
    return error;
  }
  return String(error);
}
