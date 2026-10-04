import { useEffect, useRef, useState } from "react";
import { getStock, undoStockUse } from "../api/commands";
import { onDataChanged } from "../api/events";
import { commandErrorMessage, type StockUse } from "../api/types";
import { strings } from "../i18n";

const t = strings.stock;
const MEAL_EVENT = "diet.meal_logged";
/** How long the notice stays up unless the pointer is on it. */
const SHOW_MS = 15_000;

/** Fired on `window` after an undo here, so an open Stock tab refetches (an undo is not an
 * event-log change, so no `data-changed` follows it). */
export const STOCK_CHANGED_EVENT = "lifeagent:stock-changed";

/**
 * A notice after a logged meal takes food out of stock, with an undo that puts it back and keeps
 * the meal. Consumption happens on the server after the chat reply, so this follows
 * `data-changed` for meals and shows the newest meal's uses the first time they appear; uses
 * already there when the page opened are never announced.
 */
export function StockUseToast() {
  const [shown, setShown] = useState<{ eventId: string; uses: StockUse[] } | null>(null);
  const [undoing, setUndoing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hovered, setHovered] = useState(false);
  // Meal events whose uses were already there or already announced. `null` until the first
  // successful read, which only records them.
  const seen = useRef<Set<string> | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function check() {
      let uses: StockUse[];
      try {
        uses = (await getStock()).recent_uses;
      } catch {
        return; // Best effort: the Stock tab still lists every use.
      }
      if (cancelled) return;
      const known = seen.current;
      seen.current = new Set([...(known ?? []), ...uses.map((u) => u.event_id)]);
      if (!known) return;
      // Newest first, so the first unseen use belongs to the meal just logged.
      const fresh = uses.find((u) => !known.has(u.event_id));
      if (!fresh) return;
      setShown({ eventId: fresh.event_id, uses: uses.filter((u) => u.event_id === fresh.event_id) });
      setError(null);
    }
    void check();
    const unsubscribe = onDataChanged((eventType) => {
      if (eventType === MEAL_EVENT) void check();
    });
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, []);

  useEffect(() => {
    if (!shown || hovered || undoing || error) return;
    const timer = window.setTimeout(() => setShown(null), SHOW_MS);
    return () => window.clearTimeout(timer);
  }, [shown, hovered, undoing, error]);

  if (!shown) return null;

  async function undo(eventId: string) {
    setUndoing(true);
    setError(null);
    try {
      await undoStockUse(eventId);
      window.dispatchEvent(new Event(STOCK_CHANGED_EVENT));
      setShown(null);
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setUndoing(false);
    }
  }

  return (
    <div
      role="status"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      className="fixed inset-x-4 bottom-4 z-40 mx-auto max-w-sm rounded-lg border border-neutral-200 bg-white p-4 text-sm shadow-lg dark:border-neutral-700 dark:bg-neutral-800 md:inset-x-auto md:right-6 md:bottom-6"
    >
      <p className="text-neutral-700 dark:text-neutral-200">{t.usedToast}</p>
      <ul className="mt-2 space-y-0.5 text-neutral-900 dark:text-neutral-50">
        {shown.uses.map((u, i) => (
          <li key={`${u.item_name}:${i}`} className="flex justify-between gap-3">
            <span>{u.item_name}</span>
            <span className="tabular-nums">−{t.amount(u.amount, t.units[u.unit])}</span>
          </li>
        ))}
      </ul>
      <p className="mt-2 text-xs text-neutral-500 dark:text-neutral-400">{t.undoHint}</p>
      {error ? <p className="mt-2 text-red-600 dark:text-red-400">{error}</p> : null}
      <div className="mt-3 flex justify-end gap-2">
        <button
          type="button"
          onClick={() => setShown(null)}
          className="rounded-md px-3 py-1.5 text-neutral-600 hover:bg-neutral-100 dark:text-neutral-300 dark:hover:bg-neutral-700"
        >
          {t.dismiss}
        </button>
        <button
          type="button"
          disabled={undoing}
          onClick={() => void undo(shown.eventId)}
          className="rounded-md bg-neutral-900 px-3 py-1.5 font-medium text-white hover:bg-neutral-700 disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-300"
        >
          {undoing ? t.undoing : t.undo}
        </button>
      </div>
    </div>
  );
}
