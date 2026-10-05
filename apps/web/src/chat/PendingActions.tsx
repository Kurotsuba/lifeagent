import { useState } from "react";
import { approveAction, rejectAction } from "../api/commands";
import { commandErrorMessage, type PendingAction } from "../api/types";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { describeActionPreview, strings } from "../i18n";

interface PendingActionsProps {
  /** Pending ones first, then those resolved on this page since the last message. */
  actions: PendingAction[];
  /** Called with the action as the server returns it after approving or rejecting. */
  onResolved: (action: PendingAction) => void;
}

/** Changes the assistant asked for that only run once the user approves them here
 * (`crates/core/src/approvals.rs`). */
export function PendingActions({ actions, onResolved }: PendingActionsProps) {
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  if (actions.length === 0) return null;

  async function resolve(id: string, approve: boolean) {
    setBusyId(id);
    setError(null);
    try {
      onResolved(await (approve ? approveAction(id) : rejectAction(id)));
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setBusyId(null);
    }
  }

  const hasPending = actions.some((a) => a.status === "pending");

  return (
    <div className="mx-auto w-full max-w-2xl px-4 pb-2 md:px-6">
      <div className="rounded-lg border border-amber-300 bg-amber-50 p-3 text-sm dark:border-amber-800 dark:bg-amber-950/40">
        {hasPending ? (
          <>
            <p className="font-medium text-amber-900 dark:text-amber-200">
              {strings.approvals.title}
            </p>
            <p className="mt-0.5 text-xs text-amber-800 dark:text-amber-300">
              {strings.approvals.hint}
            </p>
          </>
        ) : null}
        <ul className="mt-2 flex flex-col gap-2">
          {actions.map((a) => (
            <li
              key={a.id}
              className="flex flex-wrap items-center justify-between gap-2 rounded-md bg-white px-3 py-2 dark:bg-neutral-900"
            >
              <span className="min-w-0 flex-1 [overflow-wrap:anywhere] text-neutral-900 dark:text-neutral-100">
                {describeActionPreview(a.preview, a.tool_name)}
              </span>
              {a.status === "pending" ? (
                <span className="flex shrink-0 gap-2">
                  <button
                    type="button"
                    className={`${secondaryButtonClass} px-3 py-1`}
                    disabled={busyId !== null}
                    onClick={() => void resolve(a.id, false)}
                  >
                    {strings.approvals.reject}
                  </button>
                  <button
                    type="button"
                    className={`${primaryButtonClass} px-3 py-1`}
                    disabled={busyId !== null}
                    onClick={() => void resolve(a.id, true)}
                  >
                    {strings.approvals.approve}
                  </button>
                </span>
              ) : (
                <span
                  className={
                    "shrink-0 text-xs " +
                    (a.status === "approved"
                      ? "text-emerald-700 dark:text-emerald-400"
                      : a.status === "failed"
                        ? "text-red-600 dark:text-red-400"
                        : "text-neutral-500 dark:text-neutral-400")
                  }
                >
                  {statusLabel(a)}
                </span>
              )}
            </li>
          ))}
        </ul>
        {error ? (
          <p className="mt-2 text-red-600 dark:text-red-400">
            {strings.common.labelled(strings.approvals.failedToResolve, error)}
          </p>
        ) : null}
      </div>
    </div>
  );
}

function statusLabel(a: PendingAction): string {
  switch (a.status) {
    case "approved":
      return strings.approvals.approved;
    case "rejected":
      return strings.approvals.rejected;
    case "expired":
      return strings.approvals.expired;
    case "failed":
      return strings.approvals.failed(a.result_summary ?? "");
    case "pending":
      return "";
  }
}
