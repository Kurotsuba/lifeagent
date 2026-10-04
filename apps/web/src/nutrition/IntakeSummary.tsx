import type { DietCard } from "../api/types";
import { KcalRing } from "./KcalRing";
import { MacroBars } from "./MacroBars";

/** Today's kcal ring and macro bars, eaten against target. Shown on both the Today and the
 * Nutrition tabs. */
export function IntakeSummary({ card }: { card: DietCard | null }) {
  return (
    <div className="flex flex-col items-center gap-4 sm:flex-row sm:gap-6">
      <KcalRing eatenKcal={card?.total?.kcal ?? 0} targetKcal={card?.target?.kcal ?? null} />
      <MacroBars
        eaten={card?.total ?? { kcal: 0, protein_g: 0, carbs_g: 0, fat_g: 0, fiber_g: 0 }}
        target={card?.target ?? null}
      />
    </div>
  );
}
