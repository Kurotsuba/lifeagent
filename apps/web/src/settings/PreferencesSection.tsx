import { useEffect, useState } from "react";
import { addPreference, deletePreference, getPreferences, getToday } from "../api/commands";
import { commandErrorMessage, type Preference, type PreferenceKind } from "../api/types";
import { primaryButtonClass } from "../components/buttonStyles";
import { ConfirmDeleteButton } from "../components/ConfirmDeleteButton";
import { Field, inputClass } from "../components/Field";
import { strings } from "../i18n";
import { LoadState } from "./LoadState";

const PREFERENCE_LIMIT = 100;

const t = strings.settings.preferences;

function validityText(p: Preference): string | null {
  if (p.valid_from && p.valid_until) return t.between(p.valid_from, p.valid_until);
  if (p.valid_from) return t.from(p.valid_from);
  if (p.valid_until) return t.until(p.valid_until);
  return null;
}

export function PreferencesSection() {
  const [items, setItems] = useState<Preference[] | null>(null);
  // The logical date, to mark expired preferences ("YYYY-MM-DD" compares as a string).
  const [today, setToday] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [kind, setKind] = useState<PreferenceKind>("general");
  const [text, setText] = useState("");
  const [validFrom, setValidFrom] = useState("");
  const [validUntil, setValidUntil] = useState("");
  const [permanent, setPermanent] = useState(false);
  const [adding, setAdding] = useState(false);

  async function refetch() {
    try {
      const [list, view] = await Promise.all([getPreferences(PREFERENCE_LIMIT), getToday()]);
      setItems(list);
      setToday(view.logical_date);
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  useEffect(() => {
    void refetch();
  }, []);

  async function handleAdd() {
    setAdding(true);
    setError(null);
    try {
      await addPreference({
        kind,
        text,
        valid_from: validFrom || null,
        valid_until: permanent ? null : validUntil || null,
        permanent,
      });
      setText("");
      setValidFrom("");
      setValidUntil("");
      setPermanent(false);
      await refetch();
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setAdding(false);
    }
  }

  async function handleDelete(id: string) {
    setError(null);
    try {
      await deletePreference(id);
      await refetch();
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  if (!items) {
    return <LoadState error={error} />;
  }

  return (
    <div className="space-y-4">
      <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.hint}</p>
      {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}

      <div className="space-y-2">
        {items.length === 0 ? (
          <p className="text-sm text-neutral-500 dark:text-neutral-400">{t.empty}</p>
        ) : (
          items.map((p) => {
            const expired = today !== null && p.valid_until !== null && p.valid_until < today;
            const validity = validityText(p);
            return (
              <div
                key={p.id}
                className={
                  "flex items-start justify-between gap-3 rounded-md border border-neutral-200 px-3 py-2 text-sm dark:border-neutral-700" +
                  (expired ? " opacity-50" : "")
                }
              >
                <div className="min-w-0">
                  <p className="text-neutral-900 dark:text-neutral-50">
                    <span className="mr-2 rounded bg-neutral-100 px-1.5 py-0.5 text-xs text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300">
                      {t.kinds[p.kind]}
                    </span>
                    {p.text}
                  </p>
                  {validity || expired ? (
                    <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400">
                      {validity}
                      {expired ? ` · ${t.expired}` : ""}
                    </p>
                  ) : null}
                </div>
                <ConfirmDeleteButton onConfirm={() => handleDelete(p.id)} />
              </div>
            );
          })
        )}
      </div>

      <div className="space-y-3 rounded-md border border-dashed border-neutral-300 p-3 dark:border-neutral-700">
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-[auto_1fr]">
          <Field label={t.kind}>
            <select
              className={inputClass}
              value={kind}
              onChange={(e) => setKind(e.target.value as PreferenceKind)}
            >
              <option value="general">{t.kinds.general}</option>
              <option value="dislike">{t.kinds.dislike}</option>
            </select>
          </Field>
          <Field label={t.text}>
            <input
              type="text"
              className={inputClass}
              value={text}
              placeholder={kind === "dislike" ? t.dislikePlaceholder : t.generalPlaceholder}
              onChange={(e) => setText(e.target.value)}
            />
          </Field>
        </div>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <Field label={t.validFrom}>
            <input
              type="date"
              className={inputClass}
              value={validFrom}
              onChange={(e) => setValidFrom(e.target.value)}
            />
          </Field>
          <Field label={t.validUntil}>
            <input
              type="date"
              className={inputClass}
              value={permanent ? "" : validUntil}
              disabled={permanent}
              onChange={(e) => setValidUntil(e.target.value)}
            />
          </Field>
        </div>
        <label className="flex items-center gap-2 text-sm text-neutral-700 dark:text-neutral-300">
          <input
            type="checkbox"
            checked={permanent}
            onChange={(e) => setPermanent(e.target.checked)}
          />
          {t.permanent}
        </label>
        <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.validityHint}</p>
        <button
          type="button"
          className={primaryButtonClass}
          disabled={adding || text.trim() === ""}
          onClick={() => void handleAdd()}
        >
          {adding ? t.adding : t.add}
        </button>
      </div>
    </div>
  );
}
