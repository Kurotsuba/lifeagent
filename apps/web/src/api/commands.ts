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
  ImportPreview,
  ImportReport,
  NewPreference,
  NewStockItem,
  Preference,
  NutritionTargets,
  Profile,
  SettingsDto,
  SourceInfo,
  StockItem,
  StockSettings,
  StockView,
  StoredPlan,
  TodayView,
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
