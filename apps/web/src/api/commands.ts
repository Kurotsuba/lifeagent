/**
 * Thin, typed wrappers for every command in `docs/PHASE1_SPEC.md` §10.2, over `call()`
 * (`./transport`, the HTTP API). One function per
 * command, no logic beyond the call itself —
 * business logic (validation, defaults, what to do with the result) stays in the components
 * that call these, per CLAUDE.md's "no business logic in the frontend" for anything that isn't
 * this kind of pure forwarding.
 */

import type { Locale } from "../i18n";
import { call, callRaw } from "./transport";
import type {
  AppState,
  ChatTurn,
  Dish,
  Food,
  HistoryMessage,
  AttendanceSlot,
  MembersView,
  MainMealType,
  Member,
  MemberDislike,
  MemberProfile,
  ImportPreview,
  ImportReport,
  NewPreference,
  NewStockItem,
  ShoppingLineRef,
  ShoppingStatus,
  Preference,
  NutritionTargets,
  PluginInfo,
  Profile,
  SettingsDto,
  SourceInfo,
  StockItem,
  StockSettings,
  StockView,
  StoredPlan,
  TodayView,
  TrainingPreferences,
  TrainingView,
  WeightGoal,
} from "./types";

export function getAppState(): Promise<AppState> {
  return call("get_app_state");
}

export function completeOnboarding(): Promise<void> {
  return call("complete_onboarding");
}

export function getProfile(): Promise<Profile | null> {
  return call("get_profile");
}

export function saveProfile(profile: Profile): Promise<void> {
  return call("save_profile", { profile });
}

export function getWeightGoal(): Promise<WeightGoal | null> {
  return call("get_weight_goal");
}

export function saveWeightGoal(goal: WeightGoal): Promise<void> {
  return call("save_weight_goal", { goal });
}

export function logWeight(kg: number): Promise<void> {
  return call("log_weight", { kg });
}

export function previewTargets(
  profile: Profile,
  weightKg: number,
  goal: WeightGoal | null,
): Promise<NutritionTargets> {
  return call("preview_targets", { profile, weightKg, goal });
}

export function getSettings(): Promise<SettingsDto> {
  return call("get_settings");
}

// ---- Chat --------------------------------------------------------------------------------

export function sendMessage(text: string): Promise<ChatTurn> {
  return call("send_message", { text });
}

export function getHistory(limit: number): Promise<HistoryMessage[]> {
  return call("get_history", { limit });
}

// ---- Today / plan --------------------------------------------------------------------------

export function getToday(): Promise<TodayView> {
  return call("get_today");
}

export function getPlan(date: string): Promise<StoredPlan | null> {
  return call("get_plan", { date });
}

/** `date` omitted means tomorrow (§10.2's own default). `request` is the user's optional wish
 * for this plan only. */
export function generatePlan(date?: string, request?: string): Promise<StoredPlan> {
  return call("generate_plan", { date: date ?? null, request: request ?? null });
}

/** Whether a plan generation is under way (from any page, tab, the chat or the scheduler);
 * `plan-generating` events report the changes after that. */
export function isPlanGenerating(): Promise<boolean> {
  return call("is_plan_generating");
}

export function acceptPlan(date: string): Promise<void> {
  return call("accept_plan", { date });
}

export function deleteEntry(id: string): Promise<void> {
  return call("delete_entry", { id });
}

// ---- Settings ----------------------------------------------------------------------------

export function saveSettings(settings: SettingsDto): Promise<void> {
  return call("save_settings", { settings });
}

export function setLanguage(language: Locale): Promise<void> {
  return call("set_language", { language });
}

// ---- Plugins -------------------------------------------------------------------------------

export function getPlugins(): Promise<PluginInfo[]> {
  return call("get_plugins");
}

export function setPluginEnabled(id: string, enabled: boolean): Promise<void> {
  return call("set_plugin_enabled", { id, enabled });
}

// ---- Training ------------------------------------------------------------------------------

export function getTrainingView(): Promise<TrainingView> {
  return call("get_training_view");
}

export function saveTrainingPreferences(preferences: TrainingPreferences): Promise<void> {
  return call("save_training_preferences", { preferences });
}

/** `date` omitted means tomorrow. Blank `text` clears the request. */
export function saveTrainingDayPreference(text: string, date?: string): Promise<void> {
  return call("save_training_day_preference", { text, date: date ?? null });
}

// ---- Members -------------------------------------------------------------------------------

export function getMembersView(): Promise<MembersView> {
  return call("get_members_view");
}

/** Without `attendance`, the new member eats dinner with the user every day. */
export function addMember(profile: MemberProfile, attendance?: AttendanceSlot[]): Promise<Member> {
  return call("add_member", { profile, attendance: attendance ?? null });
}

export function updateMember(id: string, profile: MemberProfile): Promise<Member> {
  return call("update_member", { id, profile });
}

export function archiveMember(id: string): Promise<void> {
  return call("archive_member", { id });
}

export function setAttendanceDefault(memberId: string, slot: AttendanceSlot): Promise<void> {
  return call("set_attendance_default", { memberId, slot });
}

export function setAttendanceOverride(
  memberId: string,
  date: string,
  mealType: MainMealType,
  present: boolean,
  note?: string,
): Promise<void> {
  return call("set_attendance_override", {
    memberId,
    date,
    mealType,
    present,
    note: note ?? null,
  });
}

export function clearAttendanceOverride(
  memberId: string,
  date: string,
  mealType: MainMealType,
): Promise<void> {
  return call("clear_attendance_override", { memberId, date, mealType });
}

export function addMemberDislike(memberId: string, text: string): Promise<MemberDislike> {
  return call("add_member_dislike", { memberId, text });
}

export function removeMemberDislike(id: string): Promise<void> {
  return call("remove_member_dislike", { id });
}

// ---- Preferences ---------------------------------------------------------------------------

export function getPreferences(limit: number): Promise<Preference[]> {
  return call("get_preferences", { limit });
}

export function addPreference(preference: NewPreference): Promise<Preference> {
  return call("add_preference", { preference });
}

export function deletePreference(id: string): Promise<void> {
  return call("delete_preference", { id });
}

// ---- Stock -------------------------------------------------------------------------------

export function getStock(): Promise<StockView> {
  return call("get_stock");
}

export function addStockItem(item: NewStockItem): Promise<StockItem> {
  return call("add_stock_item", { item });
}

export function setStockQuantity(id: string, quantity: number): Promise<StockItem> {
  return call("set_stock_quantity", { id, quantity });
}

export function deleteStockItem(id: string): Promise<void> {
  return call("delete_stock_item", { id });
}

export function saveStockSettings(settings: StockSettings): Promise<void> {
  return call("save_stock_settings", { settings });
}

export function setShoppingStatus(line: ShoppingLineRef, status: ShoppingStatus): Promise<void> {
  return call("set_shopping_status", { line, status });
}

// ---- Nutrition sources / batch import (`docs/NUTRITION_SOURCES.md` §8.2) -------------------

export function listFoodSources(): Promise<SourceInfo[]> {
  return call("list_food_sources");
}

/** `kind` selects which array comes back (`"food"` when omitted); `query` is a food-only
 * substring filter, `brand` a dish-only exact filter (`docs/DISH_NUTRITION.md` §8.2). */
export function listFoods(
  sourceId: string | null,
  query: string | null,
  limit: number,
  kind?: "food" | "dish" | null,
  brand?: string | null,
): Promise<Food[] | Dish[]> {
  return call("list_foods", {
    sourceId,
    query,
    limit,
    kind: kind ?? null,
    brand: brand ?? null,
  });
}

export function reorderFoodSources(ids: string[]): Promise<void> {
  return call("reorder_food_sources", { ids });
}

export function setFoodSourceEnabled(id: string, enabled: boolean): Promise<void> {
  return call("set_food_source_enabled", { id, enabled });
}

export function renameFoodSource(id: string, name: string): Promise<void> {
  return call("rename_food_source", { id, name });
}

export function deleteFoodSource(id: string): Promise<void> {
  return call("delete_food_source", { id });
}

/** Raw body: the file's bytes directly, with its name url-encoded in the `x-file-name`
 * header — no JSON-encoded byte array, no business logic here beyond the forwarding itself
 * (the file is never parsed in the frontend, per CLAUDE.md). */
export async function previewFoodImport(file: File): Promise<ImportPreview> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  return callRaw("preview_food_import", bytes, {
    "x-file-name": encodeURIComponent(file.name),
  });
}

export function commitFoodImport(
  token: string,
  name: string | null,
  replaceSourceId: string | null,
): Promise<ImportReport> {
  return call("commit_food_import", { token, name, replaceSourceId });
}

export function cancelFoodImport(token: string): Promise<void> {
  return call("cancel_food_import", { token });
}
