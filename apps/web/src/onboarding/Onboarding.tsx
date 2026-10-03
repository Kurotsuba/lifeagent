import { useState } from "react";
import { completeOnboarding, logWeight, saveProfile, saveWeightGoal } from "../api/commands";
import { commandErrorMessage } from "../api/types";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { LanguageSelect } from "../components/LanguageSelect";
import { strings } from "../i18n";
import { buildGoal, buildProfile, defaultDraft, type OnboardingDraft } from "./draft";
import { isStepProfileValid, StepProfile } from "./StepProfile";
import { StepGoal } from "./StepGoal";

const t = strings.onboarding;

/** No 模型 step: the server's operator configures models for everyone. */
const STEPS: readonly string[] = t.steps;
const LAST_STEP = STEPS.length - 1;

interface OnboardingProps {
  onComplete: () => void;
}

export function Onboarding({ onComplete }: OnboardingProps) {
  const [step, setStep] = useState(0);
  const [draft, setDraft] = useState<OnboardingDraft>(defaultDraft);

  const [finishing, setFinishing] = useState(false);
  const [finishError, setFinishError] = useState<string | null>(null);
  // The weight already logged by an earlier, partly-failed 完成 attempt — so retrying (e.g.
  // after `save_weight_goal` rejected the goal) doesn't log the same weight a second time.
  const [loggedWeightKg, setLoggedWeightKg] = useState<number | null>(null);

  const update = (patch: Partial<OnboardingDraft>) => setDraft((d) => ({ ...d, ...patch }));

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
              disabled={finishing}
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
