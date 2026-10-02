import { useEffect, useState } from "react";
import {
  getModels,
  getUsageSummary,
  hasApiKey,
  saveModels,
  setApiKey,
  testModel,
} from "../api/commands";
import {
  commandErrorMessage,
  type ModelsConfig,
  type ProviderKind,
  type Role,
  type RoleBinding,
  type RoleUsage,
  type TestModelResult,
} from "../api/types";
import { Field, inputClass } from "../components/Field";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { describeError, strings } from "../i18n";
import { defaultDraft } from "../onboarding/draft";
import { prefillFor } from "../onboarding/StepModel";
import { LoadState } from "./LoadState";

const tm = strings.onboarding.model;
const ts = strings.settings.model;
const tRole = strings.settings.role;

const providerKinds: ProviderKind[] = ["anthropic", "open_ai_compatible"];
const ROLES: Role[] = ["parser", "planner", "conversation"];

interface ModelDraft {
  providerKind: ProviderKind;
  providerId: string;
  baseUrl: string;
  /** Blank means "keep the existing key" — the backend never returns a stored key to mask. */
  newApiKey: string;
  sameModelForAll: boolean;
  modelName: string;
  parserModel: string;
  plannerModel: string;
  conversationModel: string;
}

function draftFromConfig(config: ModelsConfig): ModelDraft {
  const provider = config.providers[0];
  const same =
    config.parser.model === config.planner.model &&
    config.planner.model === config.conversation.model;
  return {
    providerKind: provider.kind,
    providerId: provider.id,
    baseUrl: provider.base_url,
    newApiKey: "",
    sameModelForAll: same,
    modelName: same ? config.parser.model : "",
    parserModel: config.parser.model,
    plannerModel: config.planner.model,
    conversationModel: config.conversation.model,
  };
}

/** Same provider-kind prefill onboarding uses (Anthropic's base URL and suggested models, or
 * blanks for OpenAI-compatible), narrowed to the fields `ModelDraft` has. */
function providerPrefill(kind: ProviderKind): Partial<ModelDraft> {
  const p = prefillFor(kind);
  return {
    providerKind: p.providerKind,
    providerId: p.providerId,
    baseUrl: p.baseUrl,
    modelName: p.modelName,
    parserModel: p.parserModel,
    plannerModel: p.plannerModel,
    conversationModel: p.conversationModel,
  };
}

/** Switching between one shared model and per-role models carries the current value(s) across,
 * so the newly shown field(s) never start blank (a blank model would otherwise be saved for every
 * role). */
function toggleSameModel(draft: ModelDraft, same: boolean): Partial<ModelDraft> {
  if (same) {
    return {
      sameModelForAll: true,
      modelName: draft.modelName.trim() !== "" ? draft.modelName : draft.conversationModel,
    };
  }
  const fill = (model: string) => (model.trim() !== "" ? model : draft.modelName);
  return {
    sameModelForAll: false,
    parserModel: fill(draft.parserModel),
    plannerModel: fill(draft.plannerModel),
    conversationModel: fill(draft.conversationModel),
  };
}

function defaultModelDraft(): ModelDraft {
  return {
    providerKind: defaultDraft.providerKind,
    providerId: defaultDraft.providerId,
    baseUrl: defaultDraft.baseUrl,
    newApiKey: "",
    sameModelForAll: defaultDraft.sameModelForAll,
    modelName: defaultDraft.modelName,
    parserModel: defaultDraft.parserModel,
    plannerModel: defaultDraft.plannerModel,
    conversationModel: defaultDraft.conversationModel,
  };
}

/** Settings' counterpart to onboarding's `isStepModelFillable`: every field a save/test sends
 * is non-empty, and there's a key to use — either one already stored for this provider ID, or a
 * new one typed in. Without this, e.g. renaming the provider ID would save a config pointing at a
 * provider with no key. */
function isModelDraftFillable(draft: ModelDraft, hasKey: boolean): boolean {
  const modelsFilled = draft.sameModelForAll
    ? draft.modelName.trim() !== ""
    : draft.parserModel.trim() !== "" &&
      draft.plannerModel.trim() !== "" &&
      draft.conversationModel.trim() !== "";
  return (
    draft.providerId.trim() !== "" &&
    draft.baseUrl.trim() !== "" &&
    (hasKey || draft.newApiKey.trim() !== "") &&
    modelsFilled
  );
}

function buildModelsConfig(draft: ModelDraft): ModelsConfig {
  const binding = (model: string): RoleBinding => ({
    provider_id: draft.providerId,
    model,
    temperature: null,
    effort: null,
  });
  return {
    providers: [{ id: draft.providerId, kind: draft.providerKind, base_url: draft.baseUrl }],
    parser: binding(draft.sameModelForAll ? draft.modelName : draft.parserModel),
    planner: binding(draft.sameModelForAll ? draft.modelName : draft.plannerModel),
    conversation: binding(draft.sameModelForAll ? draft.modelName : draft.conversationModel),
  };
}

function TestResultLabel({ result }: { result: TestModelResult | undefined }) {
  if (!result) return null;
  if (result.ok) {
    return (
      <span className="text-xs text-emerald-600 dark:text-emerald-400">
        {ts.testResultOk(result.latency_ms)}
      </span>
    );
  }
  return (
    <span className="text-xs text-red-600 dark:text-red-400">{result.error ? describeError(result.error) : tm.testFailed}</span>
  );
}

export function ModelSection() {
  const [draft, setDraft] = useState<ModelDraft | null>(null);
  const [hasKey, setHasKey] = useState(false);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; message: string } | null>(null);
  const [testing, setTesting] = useState<Role | null>(null);
  const [results, setResults] = useState<Partial<Record<Role, TestModelResult>>>({});
  const [usage, setUsage] = useState<RoleUsage[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    getModels()
      .then((config) => {
        if (cancelled) return;
        // No stored config (onboarding normally prevents this): start from the same Anthropic
        // defaults onboarding does, so the section is still usable to configure one. `hasKey`
        // follows from the provider-ID effect below.
        setDraft(config ? draftFromConfig(config) : defaultModelDraft());
      })
      .catch((e: unknown) => {
        if (!cancelled) setStatus({ ok: false, message: commandErrorMessage(e) });
      });
    getUsageSummary()
      .then((u) => !cancelled && setUsage(u))
      .catch(() => !cancelled && setUsage([]));
    return () => {
      cancelled = true;
    };
  }, []);

  const providerId = draft?.providerId;
  useEffect(() => {
    if (providerId === undefined) return;
    let cancelled = false;
    hasApiKey(providerId)
      .then((has) => !cancelled && setHasKey(has))
      .catch(() => !cancelled && setHasKey(false));
    return () => {
      cancelled = true;
    };
  }, [providerId]);

  function update(patch: Partial<ModelDraft>) {
    setDraft((d) => (d ? { ...d, ...patch } : d));
  }

  /** Saves the candidate config (and the API key, if a replacement was entered) so a following
   * `test_model` call actually exercises the values on screen, not whatever was last saved —
   * same approach onboarding's own 测试连接 uses. */
  async function persist(current: ModelDraft) {
    if (current.newApiKey.trim() !== "") {
      await setApiKey(current.providerId, current.newApiKey.trim());
      setHasKey(true);
    }
    await saveModels(buildModelsConfig(current));
    update({ newApiKey: "" });
  }

  async function handleSave() {
    if (!draft) return;
    setSaving(true);
    setStatus(null);
    try {
      await persist(draft);
      setStatus({ ok: true, message: strings.settings.saveSucceeded });
    } catch (e) {
      setStatus({ ok: false, message: commandErrorMessage(e) });
    } finally {
      setSaving(false);
    }
  }

  async function handleTest(role: Role) {
    if (!draft) return;
    setTesting(role);
    try {
      await persist(draft);
      const result = await testModel(role);
      setResults((r) => ({ ...r, [role]: result }));
    } catch (e) {
      setResults((r) => ({ ...r, [role]: { ok: false, latency_ms: 0, error: commandErrorMessage(e) } }));
    } finally {
      setTesting(null);
    }
  }

  if (!draft) {
    return <LoadState error={status && !status.ok ? status.message : null} />;
  }
  const fillable = isModelDraftFillable(draft, hasKey);

  return (
    <div className="space-y-5">
      <p className="text-xs text-neutral-500 dark:text-neutral-400">{ts.dataSentNotice}</p>

      <Field label={tm.providerKind}>
        <select
          className={inputClass}
          value={draft.providerKind}
          onChange={(e) => update(providerPrefill(e.target.value as ProviderKind))}
        >
          {providerKinds.map((kind) => (
            <option key={kind} value={kind}>
              {tm.providerKindOptions[kind]}
            </option>
          ))}
        </select>
      </Field>

      <div className="grid grid-cols-2 gap-4">
        <Field label={tm.providerId} hint={tm.providerIdHint}>
          <input
            type="text"
            className={inputClass}
            value={draft.providerId}
            onChange={(e) => update({ providerId: e.target.value })}
          />
        </Field>
        <Field label={tm.baseUrl}>
          <input
            type="text"
            className={inputClass}
            value={draft.baseUrl}
            onChange={(e) => update({ baseUrl: e.target.value })}
          />
        </Field>
      </div>

      <Field label={ts.apiKeyLabel} hint={hasKey ? `${ts.apiKeySet} · ${ts.apiKeyReplaceHint}` : ts.apiKeyNotSet}>
        <input
          type="password"
          autoComplete="off"
          className={inputClass}
          value={draft.newApiKey}
          onChange={(e) => update({ newApiKey: e.target.value })}
        />
      </Field>

      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={draft.sameModelForAll}
          onChange={(e) => update(toggleSameModel(draft, e.target.checked))}
        />
        {tm.sameModelForAll}
      </label>

      {draft.sameModelForAll ? (
        <Field label={tm.modelName}>
          <input
            type="text"
            className={inputClass}
            value={draft.modelName}
            onChange={(e) => update({ modelName: e.target.value })}
          />
        </Field>
      ) : (
        <div className="grid grid-cols-3 gap-4">
          <Field label={tm.parserModel}>
            <input
              type="text"
              className={inputClass}
              value={draft.parserModel}
              onChange={(e) => update({ parserModel: e.target.value })}
            />
          </Field>
          <Field label={tm.plannerModel}>
            <input
              type="text"
              className={inputClass}
              value={draft.plannerModel}
              onChange={(e) => update({ plannerModel: e.target.value })}
            />
          </Field>
          <Field label={tm.conversationModel}>
            <input
              type="text"
              className={inputClass}
              value={draft.conversationModel}
              onChange={(e) => update({ conversationModel: e.target.value })}
            />
          </Field>
        </div>
      )}

      <div className="grid grid-cols-3 gap-3">
        {ROLES.map((role) => (
          <div key={role} className="flex items-center gap-2">
            <span className="w-10 shrink-0 text-xs text-neutral-500 dark:text-neutral-400">
              {tRole[role]}
            </span>
            <button
              type="button"
              className={`${secondaryButtonClass} !px-2 !py-1 text-xs`}
              disabled={testing !== null || !fillable}
              onClick={() => void handleTest(role)}
            >
              {testing === role ? tm.testing : ts.test}
            </button>
            <TestResultLabel result={results[role]} />
          </div>
        ))}
      </div>

      {!fillable ? (
        <p className="text-xs text-neutral-500 dark:text-neutral-400">{ts.incomplete}</p>
      ) : null}

      <div className="flex items-center gap-3">
        <button
          type="button"
          className={primaryButtonClass}
          disabled={saving || testing !== null || !fillable}
          onClick={() => void handleSave()}
        >
          {saving ? strings.common.saving : strings.common.save}
        </button>
        {status ? (
          <span
            className={
              status.ok
                ? "text-sm text-emerald-600 dark:text-emerald-400"
                : "text-sm text-red-600 dark:text-red-400"
            }
          >
            {status.message}
          </span>
        ) : null}
      </div>

      <div>
        <h3 className="mb-2 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
          {ts.usageTitle}
        </h3>
        {usage && usage.length > 0 ? (
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-neutral-500 dark:text-neutral-400">
                <th className="pb-1 font-normal">{ts.usageRole}</th>
                <th className="pb-1 text-right font-normal">{ts.usagePromptTokens}</th>
                <th className="pb-1 text-right font-normal">{ts.usageCompletionTokens}</th>
              </tr>
            </thead>
            <tbody>
              {usage.map((u) => (
                <tr key={u.role} className="border-t border-neutral-200 dark:border-neutral-700">
                  <td className="py-1 text-neutral-900 dark:text-neutral-50">{tRole[u.role]}</td>
                  <td className="py-1 text-right text-neutral-700 dark:text-neutral-300">
                    {u.prompt_tokens}
                  </td>
                  <td className="py-1 text-right text-neutral-700 dark:text-neutral-300">
                    {u.completion_tokens}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <p className="text-sm text-neutral-500 dark:text-neutral-400">{ts.usageEmpty}</p>
        )}
      </div>
    </div>
  );
}
