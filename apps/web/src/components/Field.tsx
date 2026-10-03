import type { ReactNode } from "react";

/** Tailwind classes shared by every text/number/select/textarea input in the app, so form
 * controls look consistent without a component wrapper around every native element. */
export const inputClass =
  "mt-1 block w-full rounded-md border border-neutral-300 bg-white px-3 py-2 text-sm " +
  "text-neutral-900 shadow-sm focus:border-neutral-500 focus:outline-none focus:ring-1 " +
  "focus:ring-neutral-500 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100";

interface FieldProps {
  label: string;
  hint?: string;
  children: ReactNode;
}

/** A labeled form field: label on top, the control (passed as children), an optional hint
 * underneath. The control is nested inside the `<label>`, so no `id`/`htmlFor` bookkeeping. */
export function Field({ label, hint, children }: FieldProps) {
  return (
    <label className="block text-sm">
      <span className="block font-medium text-neutral-700 dark:text-neutral-300">{label}</span>
      {children}
      {hint ? (
        <span className="mt-1 block text-xs text-neutral-500 dark:text-neutral-400">{hint}</span>
      ) : null}
    </label>
  );
}
