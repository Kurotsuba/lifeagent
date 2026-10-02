import { useState } from "react";
import { strings } from "../i18n";
import { primaryButtonClass, secondaryButtonClass } from "./buttonStyles";

interface ConfirmDeleteButtonProps {
  onConfirm: () => Promise<void>;
}

/** A two-step delete (tap once to arm, again to confirm) — used wherever the app needs a
 * delete-with-confirm without a browser `confirm()` popup breaking the app's own styling. */
export function ConfirmDeleteButton({ onConfirm }: ConfirmDeleteButtonProps) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);

  async function handleConfirm() {
    setBusy(true);
    try {
      await onConfirm();
    } finally {
      setBusy(false);
      setConfirming(false);
    }
  }

  if (confirming) {
    return (
      <div className="flex shrink-0 items-center gap-2">
        <button
          type="button"
          className={`${primaryButtonClass} !px-2 !py-1 text-xs`}
          disabled={busy}
          onClick={() => void handleConfirm()}
        >
          {strings.common.confirmDelete}
        </button>
        <button
          type="button"
          className={`${secondaryButtonClass} !px-2 !py-1 text-xs`}
          disabled={busy}
          onClick={() => setConfirming(false)}
        >
          {strings.common.cancel}
        </button>
      </div>
    );
  }

  return (
    <button
      type="button"
      className="shrink-0 text-xs text-neutral-400 hover:text-red-600 dark:hover:text-red-400"
      onClick={() => setConfirming(true)}
    >
      {strings.common.delete}
    </button>
  );
}
