import { useEffect, useState } from "react";
import { saveTrainingPreferences } from "../api/commands";
import {
  commandErrorMessage,
  type TrainingGoal,
  type TrainingPreferences,
} from "../api/types";
import { Field, inputClass } from "../components/Field";
import { primaryButtonClass } from "../components/buttonStyles";
import { strings } from "../i18n";

const t = strings.training.preferences;

const GOALS: TrainingGoal[] = ["general_fitness", "strength", "muscle", "endurance", "fat_loss"];

interface PreferencesFormProps {
  preferences: TrainingPreferences;
  onSaved: () => void;
}

/** Standing preferences every training plan follows. */
export function PreferencesForm({ preferences, onSaved }: PreferencesFormProps) {
  const [draft, setDraft] = useState(preferences);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; message: string } | null>(null);

  useEffect(() => setDraft(preferences), [preferences]);

  function update(patch: Partial<TrainingPreferences>) {
    setDraft((d) => ({ ...d, ...patch }));
  }

  // Input bounds only, so the form can say what's wrong before the round trip; the backend
  // enforces the same ranges.
  const valid =
    Number.isInteger(draft.days_per_week) &&
    draft.days_per_week >= 1 &&
    draft.days_per_week <= 7 &&
    Number.isInteger(draft.session_minutes) &&
    draft.session_minutes >= 10 &&
    draft.session_minutes <= 180;

  async function handleSave() {
    setSaving(true);
    setStatus(null);
    try {
      await saveTrainingPreferences(draft);
      setStatus({ ok: true, message: strings.settings.saveSucceeded });
      onSaved();
    } catch (e) {
      setStatus({ ok: false, message: commandErrorMessage(e) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="space-y-4">
      <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-50">{t.title}</h2>
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <Field label={t.goal}>
          <select
            className={inputClass}
            value={draft.goal}
            onChange={(e) => update({ goal: e.target.value as TrainingGoal })}
          >
            {GOALS.map((g) => (
              <option key={g} value={g}>
                {t.goalOptions[g]}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t.daysPerWeek}>
          <input
            type="number"
            className={inputClass}
            min={1}
            max={7}
            value={draft.days_per_week}
            onChange={(e) => update({ days_per_week: Number(e.target.value) })}
          />
        </Field>
        <Field label={t.sessionMinutes}>
          <input
            type="number"
            className={inputClass}
            min={10}
            max={180}
            step={5}
            value={draft.session_minutes}
            onChange={(e) => update({ session_minutes: Number(e.target.value) })}
          />
        </Field>
      </div>
      <Field label={strings.common.withOptional(t.equipment)}>
        <input
          type="text"
          className={inputClass}
          maxLength={500}
          placeholder={t.equipmentPlaceholder}
          value={draft.equipment}
          onChange={(e) => update({ equipment: e.target.value })}
        />
      </Field>
      <Field label={strings.common.withOptional(t.notes)}>
        <input
          type="text"
          className={inputClass}
          maxLength={500}
          placeholder={t.notesPlaceholder}
          value={draft.notes}
          onChange={(e) => update({ notes: e.target.value })}
        />
      </Field>
      {!valid ? <p className="text-sm text-red-600 dark:text-red-400">{t.invalid}</p> : null}
      <div className="flex items-center gap-3">
        <button
          type="button"
          className={primaryButtonClass}
          disabled={saving || !valid}
          onClick={() => void handleSave()}
        >
          {saving ? strings.common.saving : strings.common.save}
        </button>
        {status ? (
          <span
            className={
              "text-sm " +
              (status.ok
                ? "text-green-700 dark:text-green-400"
                : "text-red-600 dark:text-red-400")
            }
          >
            {status.message}
          </span>
        ) : null}
      </div>
    </div>
  );
}
