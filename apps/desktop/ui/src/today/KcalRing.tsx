import { zhCN } from "../i18n/zh-CN";

const t = zhCN.today.diet;

const SIZE = 128;
const STROKE = 10;
const RADIUS = (SIZE - STROKE) / 2;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

interface KcalRingProps {
  eatenKcal: number;
  targetKcal: number | null;
}

/** A single-hue progress ring (eaten / target kcal) — the accent color is fixed, never cycled,
 * and the number is always shown as text alongside the arc so identity/progress isn't
 * color-only. */
export function KcalRing({ eatenKcal, targetKcal }: KcalRingProps) {
  const ratio = targetKcal ? Math.max(0, Math.min(1, eatenKcal / targetKcal)) : 0;
  const dash = CIRCUMFERENCE * ratio;

  return (
    <div className="relative shrink-0" style={{ width: SIZE, height: SIZE }}>
      <svg width={SIZE} height={SIZE} className="-rotate-90">
        <circle
          cx={SIZE / 2}
          cy={SIZE / 2}
          r={RADIUS}
          fill="none"
          strokeWidth={STROKE}
          className="stroke-neutral-200 dark:stroke-neutral-700"
        />
        {targetKcal ? (
          <circle
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={RADIUS}
            fill="none"
            strokeWidth={STROKE}
            strokeLinecap="round"
            strokeDasharray={`${dash} ${CIRCUMFERENCE - dash}`}
            className="stroke-sky-500 dark:stroke-sky-400"
          />
        ) : null}
      </svg>
      <div className="absolute inset-0 flex flex-col items-center justify-center text-center">
        <span className="text-2xl font-semibold text-neutral-900 dark:text-neutral-50">
          {Math.round(eatenKcal)}
        </span>
        <span className="text-xs text-neutral-500 dark:text-neutral-400">
          {targetKcal !== null ? t.ofTargetKcal(Math.round(targetKcal)) : t.noTarget}
        </span>
      </div>
    </div>
  );
}
