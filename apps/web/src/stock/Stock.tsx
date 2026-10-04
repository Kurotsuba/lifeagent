import { useEffect, useState, type ReactNode } from "react";
import {
  addStockItem,
  deleteStockItem,
  getStock,
  saveStockSettings,
  setShoppingStatus,
  setStockQuantity,
} from "../api/commands";
import { onAppDataEvent } from "../api/events";
import {
  commandErrorMessage,
  type NewStockItem,
  type StockItem,
  type StockSettings,
  type StockUnit,
  type StockView,
} from "../api/types";
import { primaryButtonClass } from "../components/buttonStyles";
import { ConfirmDeleteButton } from "../components/ConfirmDeleteButton";
import { Field, inputClass } from "../components/Field";
import { strings } from "../i18n";
import { ShoppingList } from "./ShoppingList";
import { STOCK_CHANGED_EVENT } from "./StockUseToast";

const t = strings.stock;
const UNITS: StockUnit[] = ["g", "ml", "piece"];

function amountText(n: number, unit: StockUnit): string {
  return t.amount(n, t.units[unit]);
}

function Card({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="rounded-lg border border-neutral-200 p-4 dark:border-neutral-700">
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-50">{title}</h2>
        {aside}
      </div>
      {children}
    </section>
  );
}

/** The Stock tab: what's at home, the plan's shopping list, and what logged meals used. All of
 * it comes from `get_stock`; this page only renders it and forwards edits. */
export function Stock() {
  const [view, setView] = useState<StockView | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function refetch() {
    try {
      setView(await getStock());
      setError(null);
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  useEffect(() => {
    void refetch();
    // A meal logged in chat takes stock out on the server, and the notice's undo puts it
    // back; follow along.
    const onStockChanged = () => void refetch();
    window.addEventListener(STOCK_CHANGED_EVENT, onStockChanged);
    const unsubscribe = onAppDataEvent(() => void refetch());
    return () => {
      window.removeEventListener(STOCK_CHANGED_EVENT, onStockChanged);
      unsubscribe();
    };
  }, []);

  /** Runs an edit, then refetches; a failure is shown above the page. */
  async function act(action: () => Promise<unknown>) {
    setError(null);
    try {
      await action();
      await refetch();
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  if (!view) {
    return (
      <div className="p-4 md:p-6">
        <p
          className={
            error ? "text-sm text-red-600 dark:text-red-400" : "text-sm text-neutral-500 dark:text-neutral-400"
          }
        >
          {error ?? strings.common.loading}
        </p>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto p-4 md:p-6">
      <div className="mx-auto flex max-w-2xl flex-col gap-6">
        <header className="space-y-3">
          <h1 className="text-lg font-semibold text-neutral-900 dark:text-neutral-50">
            {strings.nav.stock}
          </h1>
          <SettingsToggles
            settings={view.settings}
            onChange={(next) => act(() => saveStockSettings(next))}
          />
          <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.hint}</p>
          {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}
        </header>

        <Card title={t.itemsTitle}>
          <div className="space-y-2">
            {view.items.length === 0 ? (
              <p className="text-sm text-neutral-500 dark:text-neutral-400">{t.empty}</p>
            ) : (
              view.items.map((item) => (
                <StockRow
                  // Remount on change, so the quantity box shows the saved value.
                  key={`${item.id}:${item.updated_at}`}
                  item={item}
                  today={view.logical_date}
                  onSetQuantity={(q) => act(() => setStockQuantity(item.id, q))}
                  onDelete={() => act(() => deleteStockItem(item.id))}
                />
              ))
            )}
          </div>
          <AddForm onAdd={(item) => act(() => addStockItem(item))} />
        </Card>

        <Card
          title={t.shoppingTitle}
          aside={
            view.shopping ? (
              <span className="text-xs text-neutral-500 dark:text-neutral-400">
                {t.shoppingFor(view.shopping.plan_date)}
              </span>
            ) : null
          }
        >
          {!view.shopping ? (
            <p className="text-sm text-neutral-500 dark:text-neutral-400">{t.noShopping}</p>
          ) : (
            <ShoppingList
              shopping={view.shopping}
              onSetStatus={(item, status) => {
                const shopping = view.shopping;
                if (!shopping) return Promise.resolve();
                return act(() =>
                  setShoppingStatus(
                    {
                      plan_date: shopping.plan_date,
                      plan_version: shopping.plan_version,
                      name: item.name,
                      unit: item.unit,
                    },
                    status,
                  ),
                );
              }}
            />
          )}
        </Card>

        <Card title={t.recentTitle}>
          {view.recent_uses.length === 0 ? (
            <p className="text-sm text-neutral-500 dark:text-neutral-400">{t.recentEmpty}</p>
          ) : (
            <ul className="space-y-1 text-sm text-neutral-700 dark:text-neutral-300">
              {view.recent_uses.map((u, i) => (
                <li key={`${u.event_id}:${i}`} className="flex justify-between gap-3">
                  <span>
                    <span className="mr-2 text-xs text-neutral-500 dark:text-neutral-400">
                      {u.logical_date}
                    </span>
                    {u.item_name}
                  </span>
                  <span className="tabular-nums">−{amountText(u.amount, u.unit)}</span>
                </li>
              ))}
            </ul>
          )}
        </Card>
      </div>
    </div>
  );
}

function SettingsToggles({
  settings,
  onChange,
}: {
  settings: StockSettings;
  onChange: (next: StockSettings) => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);

  async function toggle(patch: Partial<StockSettings>) {
    setBusy(true);
    try {
      await onChange({ ...settings, ...patch });
    } finally {
      setBusy(false);
    }
  }

  const options: { key: keyof StockSettings; label: string }[] = [
    { key: "auto_consume", label: t.autoConsume },
    { key: "plan_from_stock", label: t.planFromStock },
  ];
  return (
    <div className="flex flex-col gap-1">
      {options.map(({ key, label }) => (
        <label key={key} className="flex items-center gap-2 text-sm text-neutral-700 dark:text-neutral-300">
          <input
            type="checkbox"
            checked={settings[key]}
            disabled={busy}
            onChange={(e) => void toggle({ [key]: e.target.checked })}
          />
          {label}
        </label>
      ))}
    </div>
  );
}

function StockRow({
  item,
  today,
  onSetQuantity,
  onDelete,
}: {
  item: StockItem;
  /** Logical date, "YYYY-MM-DD"; compares as a string. */
  today: string;
  onSetQuantity: (quantity: number) => Promise<void>;
  onDelete: () => Promise<void>;
}) {
  const [draft, setDraft] = useState(String(item.quantity));
  const expired = item.expires_on !== null && item.expires_on < today;

  function commit() {
    const quantity = Number(draft);
    if (draft.trim() === "" || !Number.isFinite(quantity) || quantity < 0) {
      setDraft(String(item.quantity));
      return;
    }
    if (quantity !== item.quantity) void onSetQuantity(quantity);
  }

  return (
    <div className="flex items-center justify-between gap-3 rounded-md border border-neutral-200 px-3 py-2 text-sm dark:border-neutral-700">
      <div className="min-w-0">
        <p className="break-words text-neutral-900 dark:text-neutral-50">{item.name}</p>
        {item.expires_on ? (
          <p
            className={
              "text-xs " +
              (expired ? "text-red-600 dark:text-red-400" : "text-neutral-500 dark:text-neutral-400")
            }
          >
            {t.bestBefore(item.expires_on)}
            {expired ? ` · ${t.expired}` : ""}
          </p>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <span className="flex items-center gap-1">
          <input
            type="number"
            min={0}
            step="any"
            inputMode="decimal"
            aria-label={t.quantityLabel(item.name)}
            className="w-20 rounded-md border border-neutral-300 bg-white px-2 py-1 text-right text-base tabular-nums md:text-sm dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
            }}
          />
          <span className="w-8 text-xs text-neutral-500 dark:text-neutral-400">{t.units[item.unit]}</span>
        </span>
        <ConfirmDeleteButton onConfirm={onDelete} />
      </div>
    </div>
  );
}

function AddForm({ onAdd }: { onAdd: (item: NewStockItem) => Promise<void> }) {
  const [name, setName] = useState("");
  const [quantity, setQuantity] = useState("");
  const [unit, setUnit] = useState<StockUnit>("piece");
  const [expiresOn, setExpiresOn] = useState("");
  const [adding, setAdding] = useState(false);

  const quantityNumber = Number(quantity);
  const valid = name.trim() !== "" && quantity.trim() !== "" && quantityNumber > 0;

  async function handleAdd() {
    setAdding(true);
    try {
      await onAdd({ name, quantity: quantityNumber, unit, expires_on: expiresOn || null });
      setName("");
      setQuantity("");
      setExpiresOn("");
    } finally {
      setAdding(false);
    }
  }

  return (
    <form
      className="mt-4 space-y-3 rounded-md border border-dashed border-neutral-300 p-3 dark:border-neutral-700"
      onSubmit={(e) => {
        e.preventDefault();
        if (valid && !adding) void handleAdd();
      }}
    >
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-[1fr_6rem_6rem]">
        <div className="col-span-2 sm:col-span-1">
          <Field label={t.name}>
            <input
              type="text"
              className={inputClass}
              value={name}
              placeholder={t.namePlaceholder}
              onChange={(e) => setName(e.target.value)}
            />
          </Field>
        </div>
        <Field label={t.quantity}>
          <input
            type="number"
            min={0}
            step="any"
            inputMode="decimal"
            className={inputClass}
            value={quantity}
            onChange={(e) => setQuantity(e.target.value)}
          />
        </Field>
        <Field label={t.unit}>
          <select
            className={inputClass}
            value={unit}
            onChange={(e) => setUnit(e.target.value as StockUnit)}
          >
            {UNITS.map((u) => (
              <option key={u} value={u}>
                {t.units[u]}
              </option>
            ))}
          </select>
        </Field>
      </div>
      <div className="flex flex-wrap items-end gap-3">
        <Field label={strings.common.withOptional(t.expiresOn)}>
          <input
            type="date"
            className={inputClass}
            value={expiresOn}
            onChange={(e) => setExpiresOn(e.target.value)}
          />
        </Field>
        <button type="submit" className={primaryButtonClass} disabled={adding || !valid}>
          {adding ? t.adding : t.add}
        </button>
      </div>
    </form>
  );
}
