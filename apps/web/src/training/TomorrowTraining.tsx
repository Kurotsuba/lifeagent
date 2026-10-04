import { useEffect, useState } from "react";
import { saveTrainingDayPreference } from "../api/commands";
import { startPlanGeneration, usePlanGeneration } from "../api/planGeneration";
import { commandErrorMessage, type TrainingView } from "../api/types";
import { inputClass } from "../components/Field";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { strings } from "../i18n";
import { TrainingPlanCard } from "./TrainingPlanCard";

const t = strings.training.tomorrow;

interface TomorrowTrainingProps {
  view: TrainingView;
  onChanged: () => void;
}

/** Tomorrow's planned training, the optional request for it, and (re)generating the plan with
 * that request. Generating makes the whole plan, so the diet follows the training. */
export function TomorrowTraining({ view, onChanged }: TomorrowTrainingProps) {
  const [draft, setDraft] = useState(view.tomorrow_preference ?? "");
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; message: string } | null>(null);
  // Shared with the Nutrition tab and kept across page switches, so a running generation can't
  // be started twice by leaving and coming back.
  const generation = usePlanGeneration();
  const busy = generation.pending ? "generate" : saving ? "save" : null;
  const shownStatus =
    status ?? (generation.error ? { ok: false, message: generation.error } : null);

  // A new day (or a request saved from chat) replaces the draft.
  useEffect(() => {
    setDraft(view.tomorrow_preference ?? "");
  }, [view.tomorrow_date, view.tomorrow_preference]);

  const dirty = draft.trim() !== (view.tomorrow_preference ?? "");

  async function handleSave() {
    setSaving(true);
    setStatus(null);
    try {
      await saveTrainingDayPreference(draft, view.tomorrow_date);
      setStatus({ ok: true, message: t.preferenceSaved });
      onChanged();
    } catch (e) {
      setStatus({ ok: false, message: commandErrorMessage(e) });
    } finally {
      setSaving(false);
    }
  }

  async function handleGenerate() {
    setStatus(null);
    // The unsaved request is saved inside the generation, so the button stays disabled
    // throughout and a failed save is reported like a failed generation.
    const pending = dirty ? draft : null;
    const saveFirst =
      pending === null
        ? undefined
        : () => saveTrainingDayPreference(pending, view.tomorrow_date);
    if (await startPlanGeneration(view.tomorrow_date, undefined, saveFirst)) {
      onChanged();
    }
  }

  return (
    <div>
      <h2 className="mb-3 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
        {t.title(view.tomorrow_date)}
      </h2>

      {view.tomorrow_plan ? (
        <>
          <TrainingPlanCard section={view.tomorrow_plan} />
          <p className="mt-2 text-xs text-neutral-500 dark:text-neutral-400">{t.dietNote}</p>
        </>
      ) : (
        <p className="text-sm text-neutral-500 dark:text-neutral-400">
          {view.tomorrow_has_plan ? t.noTrainingSection : t.noPlan}
        </p>
      )}

      <label className="mt-4 block text-sm">
        <span className="block font-medium text-neutral-700 dark:text-neutral-300">
          {t.preferenceLabel}
        </span>
        <textarea
          className={inputClass}
          rows={2}
          maxLength={500}
          placeholder={t.preferencePlaceholder}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
      </label>

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <button
          type="button"
          className={primaryButtonClass}
          disabled={busy !== null}
          onClick={() => void handleGenerate()}
        >
          {busy === "generate" ? t.generating : view.tomorrow_has_plan ? t.regenerate : t.generate}
        </button>
        <button
          type="button"
          className={secondaryButtonClass}
          disabled={busy !== null || !dirty}
          onClick={() => void handleSave()}
        >
          {t.savePreference}
        </button>
        {shownStatus ? (
          <span
            className={
              "text-sm " +
              (shownStatus.ok
                ? "text-green-700 dark:text-green-400"
                : "text-red-600 dark:text-red-400")
            }
          >
            {shownStatus.message}
          </span>
        ) : null}
      </div>
    </div>
  );
}
