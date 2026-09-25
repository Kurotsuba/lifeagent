/**
 * Thin, typed wrappers around `invoke()` for every Tauri command the onboarding flow uses
 * (`docs/PHASE1_SPEC.md` §10.2). One function per command, no logic beyond the call itself —
 * business logic (validation, defaults, what to do with the result) stays in the components
 * that call these, per CLAUDE.md's "no business logic in the frontend" for anything that isn't
 * this kind of pure forwarding.
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  AppState,
  ModelsConfig,
  NutritionTargets,
  Profile,
  Role,
  TestModelResult,
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
