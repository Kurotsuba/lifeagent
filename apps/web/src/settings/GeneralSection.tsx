import { useEffect, useState } from "react";
import { getSettings, saveSettings } from "../api/commands";
import { commandErrorMessage, type SettingsDto } from "../api/types";
import { Field, inputClass } from "../components/Field";
import { primaryButtonClass } from "../components/buttonStyles";
import { strings } from "../i18n";
import { LoadState } from "./LoadState";

const t = strings.settings;

/** §10.3's 计划 settings: plan time and day boundary, saved together by `save_settings`. */
export function GeneralSection() {
  const [draft, setDraft] = useState<SettingsDto | null>(null);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; message: string } | null>(null);

  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((s) => !cancelled && setDraft(s))
      .catch((e: unknown) => {
        if (!cancelled) setStatus({ ok: false, message: commandErrorMessage(e) });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  function update(patch: Partial<SettingsDto>) {
    setDraft((d) => (d ? { ...d, ...patch } : d));
  }

  async function handleSave() {
    if (!draft) return;
    setSaving(true);
    setStatus(null);
    try {
      await saveSettings(draft);
      setStatus({ ok: true, message: t.saveSucceeded });
    } catch (e) {
      setStatus({ ok: false, message: commandErrorMessage(e) });
    } finally {
      setSaving(false);
    }
  }

  if (!draft) {
    return <LoadState error={status && !status.ok ? status.message : null} />;
  }

  return (
    <div className="space-y-5">
      <div className="grid grid-cols-2 gap-4">
        <Field label={t.plan.planTime}>
          <input
            type="time"
            className={inputClass}
            value={draft.plan_time}
            onChange={(e) => update({ plan_time: e.target.value })}
          />
        </Field>
        <Field label={t.plan.dayBoundary}>
          <input
            type="time"
            className={inputClass}
            value={draft.day_boundary}
            onChange={(e) => update({ day_boundary: e.target.value })}
          />
        </Field>
      </div>

      <div className="flex items-center gap-3">
        <button
          type="button"
          className={primaryButtonClass}
          disabled={saving}
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
    </div>
  );
}
