import type { DietMacros } from "../api/types";
import { strings } from "../i18n";

const t = strings.today.diet;

/** Fixed hue order, never cycled: protein/carbs/fat always map to the same color so a repeat
 * viewer builds a stable association instead of re-reading the legend every time. */
const ROWS: { key: "protein_g" | "carbs_g" | "fat_g"; label: string; barClass: string }[] = [
  { key: "protein_g", label: t.protein, barClass: "bg-indigo-500 dark:bg-indigo-400" },
  { key: "carbs_g", label: t.carbs, barClass: "bg-amber-500 dark:bg-amber-400" },
  { key: "fat_g", label: t.fat, barClass: "bg-violet-500 dark:bg-violet-400" },
];

interface MacroBarsProps {
  eaten: DietMacros;
  target: DietMacros | null;
}

export function MacroBars({ eaten, target }: MacroBarsProps) {
  return (
    <div className="flex w-full flex-1 flex-col justify-center gap-2">
      {ROWS.map((row) => {
        const value = eaten[row.key];
        const goal = target ? target[row.key] : null;
        const ratio = goal ? Math.max(0, Math.min(1, value / goal)) : 0;
        return (
          <div key={row.key} className="flex items-center gap-2 text-xs">
            <span className="w-16 shrink-0 text-neutral-500 dark:text-neutral-400">
              {row.label}
            </span>
            <div className="h-2 flex-1 overflow-hidden rounded-full bg-neutral-200 dark:bg-neutral-700">
              {goal ? (
                <div
                  className={`h-full rounded-full ${row.barClass}`}
                  style={{ width: `${ratio * 100}%` }}
                />
              ) : null}
            </div>
            <span className="w-24 shrink-0 text-right text-neutral-700 dark:text-neutral-300">
              {t.gramsShort(Math.round(value))}
              {goal !== null ? ` / ${t.gramsShort(Math.round(goal))}` : ""}
            </span>
          </div>
        );
      })}
    </div>
  );
}
