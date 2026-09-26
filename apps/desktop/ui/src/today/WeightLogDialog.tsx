import { useState } from "react";
import { logWeight } from "../api/commands";
import { commandErrorMessage } from "../api/types";
import { inputClass } from "../components/Field";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { zhCN } from "../i18n/zh-CN";

interface WeightLogDialogProps {
  onClose: () => void;
  onSaved: () => void;
}

/** Small popover anchored under the header's 记录体重 button — a full modal would be overkill
 * for one number field. */
export function WeightLogDialog({ onClose, onSaved }: WeightLogDialogProps) {
  const [kg, setKg] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSave() {
    const value = Number(kg);
    if (!Number.isFinite(value) || value <= 0) {
      setError(zhCN.today.logWeightInvalid);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await logWeight(value);
      onSaved();
      onClose();
    } catch (e) {
      setError(commandErrorMessage(e));
      setSaving(false);
    }
  }

  return (
    <div className="absolute right-0 top-full z-10 mt-2 w-56 rounded-md border border-neutral-200 bg-white p-3 shadow-lg dark:border-neutral-700 dark:bg-neutral-800">
      <p className="mb-2 text-sm font-medium text-neutral-900 dark:text-neutral-50">
        {zhCN.today.logWeightTitle}
      </p>
      <input
        type="number"
        step="0.1"
        min="0"
        autoFocus
        className={inputClass}
        placeholder={zhCN.today.logWeightPlaceholder}
        value={kg}
        onChange={(e) => setKg(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void handleSave();
        }}
      />
      {error ? <p className="mt-1 text-xs text-red-600 dark:text-red-400">{error}</p> : null}
      <div className="mt-3 flex justify-end gap-2">
        <button
          type="button"
          className={`${secondaryButtonClass} !px-3 !py-1.5 text-xs`}
          disabled={saving}
          onClick={onClose}
        >
          {zhCN.common.cancel}
        </button>
        <button
          type="button"
          className={`${primaryButtonClass} !px-3 !py-1.5 text-xs`}
          disabled={saving}
          onClick={handleSave}
        >
          {zhCN.common.save}
        </button>
      </div>
    </div>
  );
}
