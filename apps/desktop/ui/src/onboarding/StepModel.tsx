import { Field, inputClass } from "../components/Field";
import { secondaryButtonClass } from "../components/buttonStyles";
import { zhCN } from "../i18n/zh-CN";
import type { ProviderKind } from "../api/types";
import type { OnboardingDraft } from "./draft";

const t = zhCN.onboarding.model;

const providerKinds: ProviderKind[] = ["anthropic", "open_ai_compatible"];

/** Anthropic's own defaults (§10.3): base URL with no `/v1` suffix, and suggested per-role
 * models (opus for planning, sonnet for conversation, haiku for parsing). Switching away clears
 * these rather than leaving Anthropic-specific values under a different provider kind. */
function prefillFor(kind: ProviderKind): Partial<OnboardingDraft> {
  if (kind === "anthropic") {
    return {
      providerKind: kind,
      providerId: "anthropic",
      baseUrl: "https://api.anthropic.com",
      modelName: "claude-sonnet-5",
      parserModel: "claude-haiku-4-5",
      plannerModel: "claude-opus-5",
      conversationModel: "claude-sonnet-5",
    };
  }
  return {
    providerKind: kind,
    providerId: "openai",
    baseUrl: "",
    modelName: "",
    parserModel: "",
    plannerModel: "",
    conversationModel: "",
  };
}

interface StepModelProps {
  draft: OnboardingDraft;
  update: (patch: Partial<OnboardingDraft>) => void;
  testing: boolean;
  testError: string | null;
  testSucceeded: boolean;
  onTest: () => void;
}

export function StepModel({
  draft,
  update,
  testing,
  testError,
  testSucceeded,
  onTest,
}: StepModelProps) {
  return (
    <div className="space-y-5">
      <h2 className="text-lg font-semibold text-neutral-900 dark:text-neutral-50">{t.title}</h2>

      <Field label={t.providerKind}>
        <select
          className={inputClass}
          value={draft.providerKind}
          onChange={(e) => update(prefillFor(e.target.value as ProviderKind))}
        >
          {providerKinds.map((kind) => (
            <option key={kind} value={kind}>
              {t.providerKindOptions[kind]}
            </option>
          ))}
        </select>
      </Field>

      <div className="grid grid-cols-2 gap-4">
        <Field label={t.providerId} hint={t.providerIdHint}>
          <input
            type="text"
            className={inputClass}
            value={draft.providerId}
            onChange={(e) => update({ providerId: e.target.value })}
          />
        </Field>
        <Field label={t.baseUrl}>
          <input
            type="text"
            className={inputClass}
            value={draft.baseUrl}
            onChange={(e) => update({ baseUrl: e.target.value })}
          />
        </Field>
      </div>

      <Field label={t.apiKey}>
        <input
          type="password"
          autoComplete="off"
          className={inputClass}
          placeholder={t.apiKeyPlaceholder}
          value={draft.apiKey}
          onChange={(e) => update({ apiKey: e.target.value })}
        />
      </Field>

      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={draft.sameModelForAll}
          onChange={(e) => update({ sameModelForAll: e.target.checked })}
        />
        {t.sameModelForAll}
      </label>

      {draft.sameModelForAll ? (
        <Field label={t.modelName}>
          <input
            type="text"
            className={inputClass}
            value={draft.modelName}
            onChange={(e) => update({ modelName: e.target.value })}
          />
        </Field>
      ) : (
        <div className="grid grid-cols-3 gap-4">
          <Field label={t.parserModel}>
            <input
              type="text"
              className={inputClass}
              value={draft.parserModel}
              onChange={(e) => update({ parserModel: e.target.value })}
            />
          </Field>
          <Field label={t.plannerModel}>
            <input
              type="text"
              className={inputClass}
              value={draft.plannerModel}
              onChange={(e) => update({ plannerModel: e.target.value })}
            />
          </Field>
          <Field label={t.conversationModel}>
            <input
              type="text"
              className={inputClass}
              value={draft.conversationModel}
              onChange={(e) => update({ conversationModel: e.target.value })}
            />
          </Field>
        </div>
      )}

      <div className="flex items-center gap-3">
        <button
          type="button"
          className={secondaryButtonClass}
          disabled={testing || !isStepModelFillable(draft)}
          onClick={onTest}
        >
          {testing ? t.testing : t.testConnection}
        </button>
        {!testing && testSucceeded ? (
          <span className="text-sm text-emerald-600 dark:text-emerald-400">
            {t.testSucceeded}
          </span>
        ) : null}
        {!testing && !testSucceeded && testError ? (
          <span className="text-sm text-red-600 dark:text-red-400">
            {t.testFailed}：{testError}
          </span>
        ) : null}
      </div>
      {!testSucceeded ? (
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          {t.testRequiredBeforeFinish}
        </p>
      ) : null}
    </div>
  );
}

/** Whether every field `测试连接` needs is filled — gates the button itself, separate from
 * `testSucceeded` (whether the *last* attempt with the *current* values passed). */
export function isStepModelFillable(draft: OnboardingDraft): boolean {
  const modelsFilled = draft.sameModelForAll
    ? draft.modelName.trim() !== ""
    : draft.parserModel.trim() !== "" &&
      draft.plannerModel.trim() !== "" &&
      draft.conversationModel.trim() !== "";
  return (
    draft.providerId.trim() !== "" &&
    draft.baseUrl.trim() !== "" &&
    draft.apiKey.trim() !== "" &&
    modelsFilled
  );
}
