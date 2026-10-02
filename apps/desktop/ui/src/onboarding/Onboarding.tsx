import { useState } from "react";
import { completeOnboarding, logWeight, saveProfile, saveWeightGoal, saveModels, setApiKey, testModel } from "../api/commands";
import { isWeb } from "../api/platform";
import { commandErrorMessage, type Role } from "../api/types";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { LanguageSelect } from "../components/LanguageSelect";
import { describeError, strings } from "../i18n";
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

const t = strings.onboarding;

const ROLES: Role[] = ["parser", "planner", "conversation"];

/** The web build has no 模型 step: the server's operator configures models for everyone. */
const STEPS: readonly string[] = isWeb ? t.steps.slice(0, 2) : t.steps;
const LAST_STEP = STEPS.length - 1;

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
          throw strings.common.labelled(
            strings.settings.role[role],
            result.error ? describeError(result.error) : t.model.testFailed,
          );
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
          <div className="mb-4 flex items-center justify-between">
            <p className="text-xs font-medium uppercase tracking-wide text-neutral-500 dark:text-neutral-400">
              {t.stepLabel(step + 1, STEPS.length)} · {STEPS[step]}
            </p>
            {/* First step only: switching reloads the page, which drops the draft. */}
            {step === 0 ? <LanguageSelect /> : null}
          </div>

          {step === 0 ? <StepProfile draft={draft} update={update} /> : null}
          {step === 1 ? <StepGoal draft={draft} update={update} /> : null}
          {step === 2 && !isWeb ? (
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
            {strings.common.back}
          </button>
          {step < LAST_STEP ? (
            <button
              type="button"
              className={primaryButtonClass}
              disabled={!canGoNext}
              onClick={() => setStep((s) => Math.min(LAST_STEP, s + 1))}
            >
              {strings.common.next}
            </button>
          ) : (
            <button
              type="button"
              className={primaryButtonClass}
              disabled={
                finishing || (!isWeb && (!testSucceeded || !isStepModelFillable(draft)))
              }
              onClick={handleFinish}
            >
              {finishing ? strings.common.finishing : strings.common.finish}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
