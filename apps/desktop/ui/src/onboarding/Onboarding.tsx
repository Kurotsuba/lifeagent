import { useState } from "react";
import { completeOnboarding, logWeight, saveProfile, saveWeightGoal, saveModels, setApiKey, testModel } from "../api/commands";
import { commandErrorMessage, type Role } from "../api/types";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { zhCN } from "../i18n/zh-CN";
import {
  buildGoal,
  buildModelsConfig,
  buildProfile,
  defaultDraft,
  modelSignature,
  type OnboardingDraft,
} from "./draft";
import { isStepProfileValid, StepProfile } from "./StepProfile";
import { StepGoal } from "./StepGoal";
import { isStepModelFillable, StepModel } from "./StepModel";

const t = zhCN.onboarding;

const ROLES: Role[] = ["parser", "planner", "conversation"];

interface OnboardingProps {
  onComplete: () => void;
}

export function Onboarding({ onComplete }: OnboardingProps) {
  const [step, setStep] = useState(0);
  const [draft, setDraft] = useState<OnboardingDraft>(defaultDraft);

  const [testing, setTesting] = useState(false);
  const [testError, setTestError] = useState<string | null>(null);
  const [testedSignature, setTestedSignature] = useState<string | null>(null);
  const [testLatencies, setTestLatencies] = useState<Partial<Record<Role, number>>>({});

  const [finishing, setFinishing] = useState(false);
  const [finishError, setFinishError] = useState<string | null>(null);
  // The weight already logged by an earlier, partly-failed 完成 attempt — so retrying (e.g.
  // after `save_weight_goal` rejected the goal) doesn't log the same weight a second time.
  const [loggedWeightKg, setLoggedWeightKg] = useState<number | null>(null);

  const update = (patch: Partial<OnboardingDraft>) => setDraft((d) => ({ ...d, ...patch }));
  const testSucceeded = testedSignature !== null && testedSignature === modelSignature(draft);

  async function runTest() {
    setTesting(true);
    setTestError(null);
    try {
      await setApiKey(draft.providerId, draft.apiKey);
      await saveModels(buildModelsConfig(draft));
      const latencies: Partial<Record<Role, number>> = {};
      for (const role of ROLES) {
        const result = await testModel(role);
        if (!result.ok) {
          // A string, not an `Error`: `commandErrorMessage` would render it as "Error: …".
          throw `${zhCN.settings.role[role]}：${result.error ?? t.model.testFailed}`;
        }
        latencies[role] = result.latency_ms;
      }
      setTestLatencies(latencies);
      setTestedSignature(modelSignature(draft));
    } catch (error) {
      setTestedSignature(null);
      setTestError(commandErrorMessage(error));
    } finally {
      setTesting(false);
    }
  }

  async function handleFinish() {
    setFinishing(true);
    setFinishError(null);
    try {
      await saveProfile(buildProfile(draft));
      const weightKg = Number(draft.weightKg);
      if (weightKg !== loggedWeightKg) {
        await logWeight(weightKg);
        setLoggedWeightKg(weightKg);
      }
      await saveWeightGoal(buildGoal(draft));
      await completeOnboarding();
      onComplete();
    } catch (error) {
      setFinishError(commandErrorMessage(error));
      setFinishing(false);
    }
  }

  const canGoNext = step === 0 ? isStepProfileValid(draft) : true;

  return (
    <div className="flex h-full flex-col">
      <div className="flex-1 overflow-y-auto px-6 py-8">
        <div className="mx-auto max-w-2xl">
          <p className="mb-4 text-xs font-medium uppercase tracking-wide text-neutral-500 dark:text-neutral-400">
            {t.stepLabel(step + 1, t.steps.length)} · {t.steps[step]}
          </p>

          {step === 0 ? <StepProfile draft={draft} update={update} /> : null}
          {step === 1 ? <StepGoal draft={draft} update={update} /> : null}
          {step === 2 ? (
            <StepModel
              draft={draft}
              update={update}
              testing={testing}
              testError={testError}
              testSucceeded={testSucceeded}
              testLatencies={testLatencies}
              onTest={runTest}
            />
          ) : null}

          {finishError ? (
            <p className="mt-4 text-sm text-red-600 dark:text-red-400">{finishError}</p>
          ) : null}
        </div>
      </div>

      <div className="border-t border-neutral-200 px-6 py-4 dark:border-neutral-700">
        <div className="mx-auto flex max-w-2xl justify-between">
          <button
            type="button"
            className={secondaryButtonClass}
            disabled={step === 0}
            onClick={() => setStep((s) => Math.max(0, s - 1))}
          >
            {zhCN.common.back}
          </button>
          {step < 2 ? (
            <button
              type="button"
              className={primaryButtonClass}
              disabled={!canGoNext}
              onClick={() => setStep((s) => Math.min(2, s + 1))}
            >
              {zhCN.common.next}
            </button>
          ) : (
            <button
              type="button"
              className={primaryButtonClass}
              disabled={!testSucceeded || !isStepModelFillable(draft) || finishing}
              onClick={handleFinish}
            >
              {zhCN.common.finish}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
