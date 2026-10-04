import { useState } from "react";
import type { ShoppingItem, ShoppingStatus, StockUnit, StockView } from "../api/types";
import { secondaryButtonClass } from "../components/buttonStyles";
import { strings } from "../i18n";

const t = strings.stock;

function amountText(n: number, unit: StockUnit): string {
  return t.amount(n, t.units[unit]);
}

interface ShoppingListProps {
  shopping: NonNullable<StockView["shopping"]>;
  /** Sets a line's status. Without it the list is read-only (the Today tab). */
  onSetStatus?: (item: ShoppingItem, status: ShoppingStatus) => Promise<void>;
}

/** The plan's shopping list. Each open line can be completed (bought: the server puts its
 * planned amount into stock) or cancelled; a cancelled line can be reopened. */
export function ShoppingList({ shopping, onSetStatus }: ShoppingListProps) {
  if (shopping.items.length === 0) {
    return <p className="text-sm text-neutral-500 dark:text-neutral-400">{t.nothingToBuy}</p>;
  }
  return (
    <ul className="space-y-2">
      {shopping.items.map((item) => (
        <ShoppingRow key={`${item.name}:${item.unit}`} item={item} onSetStatus={onSetStatus} />
      ))}
    </ul>
  );
}

function ShoppingRow({
  item,
  onSetStatus,
}: {
  item: ShoppingItem;
  onSetStatus?: ShoppingListProps["onSetStatus"];
}) {
  const [busy, setBusy] = useState(false);
  const closed = item.status !== "open";

  async function set(status: ShoppingStatus) {
    if (!onSetStatus) return;
    setBusy(true);
    try {
      await onSetStatus(item, status);
    } finally {
      setBusy(false);
    }
  }

  const smallButton = `${secondaryButtonClass} !px-2 !py-1 text-xs`;

  return (
    <li className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 text-sm">
      <span className="flex min-w-0 items-center gap-2">
        <span
          aria-hidden
          className={
            item.status === "done"
              ? "text-emerald-600 dark:text-emerald-400"
              : "text-neutral-300 dark:text-neutral-600"
          }
        >
          {item.status === "done" ? "✓" : item.status === "cancelled" ? "×" : "○"}
        </span>
        <span
          className={
            "break-words " +
            (closed
              ? "text-neutral-400 line-through dark:text-neutral-500"
              : "text-neutral-900 dark:text-neutral-50")
          }
        >
          {item.name}{" "}
          <span className="tabular-nums text-neutral-500 dark:text-neutral-400">
            {amountText(item.amount, item.unit)}
          </span>
        </span>
      </span>
      <span className="flex shrink-0 items-center gap-2">
        {item.status === "open" && item.in_stock > 0 ? (
          <span className="text-xs text-neutral-500 dark:text-neutral-400">
            {t.inStock(amountText(item.in_stock, item.unit))}
          </span>
        ) : null}
        {item.status === "done" ? (
          <span className="text-xs text-emerald-700 dark:text-emerald-400">{t.statusDone}</span>
        ) : null}
        {item.status === "cancelled" ? (
          <span className="text-xs text-neutral-500 dark:text-neutral-400">
            {t.statusCancelled}
          </span>
        ) : null}
        {onSetStatus && item.status === "open" ? (
          <>
            <button
              type="button"
              className={smallButton}
              disabled={busy}
              title={t.completeHint}
              onClick={() => void set("done")}
            >
              {t.complete}
            </button>
            <button
              type="button"
              className={smallButton}
              disabled={busy}
              onClick={() => void set("cancelled")}
            >
              {t.cancel}
            </button>
          </>
        ) : null}
        {onSetStatus && item.status === "cancelled" ? (
          <button
            type="button"
            className={smallButton}
            disabled={busy}
            onClick={() => void set("open")}
          >
            {t.reopen}
          </button>
        ) : null}
      </span>
    </li>
  );
}
