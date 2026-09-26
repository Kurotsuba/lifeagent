import type { ReactNode } from "react";
import { zhCN } from "../i18n/zh-CN";
import { FoodDataSection } from "./FoodDataSection";
import { GeneralSection } from "./GeneralSection";
import { MemorySection } from "./MemorySection";
import { ModelSection } from "./ModelSection";
import { ProfileGoalSection } from "./ProfileGoalSection";

const t = zhCN.settings;

function SectionCard({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="rounded-lg border border-neutral-200 p-4 dark:border-neutral-700">
      <h2 className="mb-4 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
        {title}
      </h2>
      {children}
    </section>
  );
}

export function Settings() {
  return (
    <div className="h-full overflow-y-auto p-6">
      <div className="mx-auto flex max-w-2xl flex-col gap-6">
        <h1 className="text-lg font-semibold text-neutral-900 dark:text-neutral-50">
          {zhCN.nav.settings}
        </h1>

        <SectionCard title={t.sections.profileGoal}>
          <ProfileGoalSection />
        </SectionCard>

        <SectionCard title={t.sections.foodData}>
          <FoodDataSection />
        </SectionCard>

        <SectionCard title={t.sections.model}>
          <ModelSection />
        </SectionCard>

        <SectionCard title={`${t.sections.plan} · ${t.sections.general}`}>
          <GeneralSection />
        </SectionCard>

        <SectionCard title={t.sections.memory}>
          <MemorySection />
        </SectionCard>
      </div>
    </div>
  );
}
