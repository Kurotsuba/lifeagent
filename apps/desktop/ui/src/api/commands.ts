/**
 * Thin, typed wrappers around `invoke()` for every Tauri command in `docs/PHASE1_SPEC.md`
 * §10.2. One function per command, no logic beyond the call itself —
 * business logic (validation, defaults, what to do with the result) stays in the components
 * that call these, per CLAUDE.md's "no business logic in the frontend" for anything that isn't
 * this kind of pure forwarding.
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  AppState,
  ChatTurn,
  Dish,
  Food,
  HistoryMessage,
  ImportPreview,
  ImportReport,
  MemoryNote,
  ModelsConfig,
  NutritionTargets,
  Profile,
  Role,
  RoleUsage,
  SettingsDto,
  SourceInfo,
  StoredPlan,
  TestModelResult,
  TodayView,
  WeightGoal,
} from "./types";

export function getAppState(): Promise<AppState> {
  return invoke("get_app_state");
}

export function completeOnboarding(): Promise<void> {
  return invoke("complete_onboarding");
}

export function getProfile(): Promise<Profile | null> {
  return invoke("get_profile");
}

export function saveProfile(profile: Profile): Promise<void> {
  return invoke("save_profile", { profile });
}

export function getWeightGoal(): Promise<WeightGoal | null> {
  return invoke("get_weight_goal");
}

export function saveWeightGoal(goal: WeightGoal): Promise<void> {
  return invoke("save_weight_goal", { goal });
}

export function logWeight(kg: number): Promise<void> {
  return invoke("log_weight", { kg });
}

export function previewTargets(
  profile: Profile,
  weightKg: number,
  goal: WeightGoal | null,
): Promise<NutritionTargets> {
  return invoke("preview_targets", { profile, weightKg, goal });
}

export function getModels(): Promise<ModelsConfig | null> {
  return invoke("get_models");
}

export function saveModels(models: ModelsConfig): Promise<void> {
  return invoke("save_models", { models });
}

export function setApiKey(providerId: string, key: string): Promise<void> {
  return invoke("set_api_key", { providerId, key });
}

export function hasApiKey(providerId: string): Promise<boolean> {
  return invoke("has_api_key", { providerId });
}

export function testModel(role: Role): Promise<TestModelResult> {
  return invoke("test_model", { role });
}

export function getSettings(): Promise<SettingsDto> {
  return invoke("get_settings");
}

// ---- Chat --------------------------------------------------------------------------------

export function sendMessage(text: string): Promise<ChatTurn> {
  return invoke("send_message", { text });
}

export function getHistory(limit: number): Promise<HistoryMessage[]> {
  return invoke("get_history", { limit });
}

// ---- Today / plan --------------------------------------------------------------------------

export function getToday(): Promise<TodayView> {
  return invoke("get_today");
}

export function getPlan(date: string): Promise<StoredPlan | null> {
  return invoke("get_plan", { date });
}

/** `date` omitted means tomorrow (§10.2's own default). */
export function generatePlan(date?: string): Promise<StoredPlan> {
  return invoke("generate_plan", { date: date ?? null });
}

export function acceptPlan(date: string): Promise<void> {
  return invoke("accept_plan", { date });
}

export function deleteEntry(id: string): Promise<void> {
  return invoke("delete_entry", { id });
}

// ---- Settings ----------------------------------------------------------------------------

export function saveSettings(settings: SettingsDto): Promise<void> {
  return invoke("save_settings", { settings });
}

// ---- Memory --------------------------------------------------------------------------------

export function getMemory(limit: number): Promise<MemoryNote[]> {
  return invoke("get_memory", { limit });
}

export function deleteMemory(id: string): Promise<void> {
  return invoke("delete_memory", { id });
}

// ---- Usage ----------------------------------------------------------------------------------

export function getUsageSummary(): Promise<RoleUsage[]> {
  return invoke("get_usage_summary");
}

// ---- Nutrition sources / batch import (`docs/NUTRITION_SOURCES.md` §8.2) -------------------

export function listFoodSources(): Promise<SourceInfo[]> {
  return invoke("list_food_sources");
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
  return invoke("list_foods", {
    sourceId,
    query,
    limit,
    kind: kind ?? null,
    brand: brand ?? null,
  });
}

export function reorderFoodSources(ids: string[]): Promise<void> {
  return invoke("reorder_food_sources", { ids });
}

export function setFoodSourceEnabled(id: string, enabled: boolean): Promise<void> {
  return invoke("set_food_source_enabled", { id, enabled });
}

export function renameFoodSource(id: string, name: string): Promise<void> {
  return invoke("rename_food_source", { id, name });
}

export function deleteFoodSource(id: string): Promise<void> {
  return invoke("delete_food_source", { id });
}

/** Raw IPC body: the file's bytes directly, with its name url-encoded in the `x-file-name`
 * header — no JSON-encoded byte array, no business logic here beyond the forwarding itself
 * (the file is never parsed in the frontend, per CLAUDE.md). */
export async function previewFoodImport(file: File): Promise<ImportPreview> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke("preview_food_import", bytes, {
    headers: { "x-file-name": encodeURIComponent(file.name) },
  });
}

export function commitFoodImport(
  token: string,
  name: string | null,
  replaceSourceId: string | null,
): Promise<ImportReport> {
  return invoke("commit_food_import", { token, name, replaceSourceId });
}

export function cancelFoodImport(token: string): Promise<void> {
  return invoke("cancel_food_import", { token });
}
