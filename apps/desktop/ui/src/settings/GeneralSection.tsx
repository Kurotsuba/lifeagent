import { useEffect, useState } from "react";
import { getSettings, saveSettings } from "../api/commands";
import { commandErrorMessage, type SettingsDto } from "../api/types";
import { Field, inputClass } from "../components/Field";
import { primaryButtonClass } from "../components/buttonStyles";
import { zhCN } from "../i18n/zh-CN";
import { LoadState } from "./LoadState";

const t = zhCN.settings;

/** §10.3 groups 计划 (plan time, day boundary) and 通用 (notifications, autostart) as separate
 * bullets, but `SettingsDto`/`save_settings` persist all four atomically in one call — so this
 * is one section with one 保存 button, not two, to match how the backend actually saves them. */
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

      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={draft.notifications_enabled}
          onChange={(e) => update({ notifications_enabled: e.target.checked })}
        />
        <span>
          {t.general.notifications}
          <span className="ml-1 text-xs text-neutral-500 dark:text-neutral-400">
            {t.general.notificationsHint}
          </span>
        </span>
      </label>

      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={draft.autostart}
          onChange={(e) => update({ autostart: e.target.checked })}
        />
        <span>
          {t.general.autostart}
          <span className="ml-1 text-xs text-neutral-500 dark:text-neutral-400">
            {t.general.autostartHint}
          </span>
        </span>
      </label>

      <div className="flex items-center gap-3">
        <button
          type="button"
          className={primaryButtonClass}
          disabled={saving}
          onClick={() => void handleSave()}
        >
          {saving ? zhCN.common.saving : zhCN.common.save}
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
