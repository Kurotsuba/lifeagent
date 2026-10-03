import type { ReactNode } from "react";
import { LanguageSelect } from "../components/LanguageSelect";
import { strings } from "../i18n";
import { AccountSection } from "./AccountSection";
import { FeaturesSection } from "./FeaturesSection";
import { FoodDataSection } from "./FoodDataSection";
import { GeneralSection } from "./GeneralSection";
import { PreferencesSection } from "./PreferencesSection";
import { ProfileGoalSection } from "./ProfileGoalSection";

const t = strings.settings;

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

interface SettingsProps {
  /** An optional plugin was switched on or off. */
  onPluginsChanged: () => void;
}

export function Settings({ onPluginsChanged }: SettingsProps) {
  return (
    <div className="h-full overflow-y-auto p-4 md:p-6">
      <div className="mx-auto flex max-w-2xl flex-col gap-6">
        <h1 className="text-lg font-semibold text-neutral-900 dark:text-neutral-50">
          {strings.nav.settings}
        </h1>

        <SectionCard title={t.sections.profileGoal}>
          <ProfileGoalSection />
        </SectionCard>

        <SectionCard title={t.sections.features}>
          <FeaturesSection onChanged={onPluginsChanged} />
        </SectionCard>

        <SectionCard title={t.sections.foodData}>
          <FoodDataSection />
        </SectionCard>

        <SectionCard title={t.sections.plan}>
          <GeneralSection />
        </SectionCard>

        <SectionCard title={t.sections.language}>
          <div className="flex items-center gap-3">
            <LanguageSelect />
            <span className="text-xs text-neutral-500 dark:text-neutral-400">{t.language.hint}</span>
          </div>
        </SectionCard>

        <SectionCard title={t.sections.preferences}>
          <PreferencesSection />
        </SectionCard>

        <SectionCard title={t.sections.account}>
          <AccountSection />
        </SectionCard>
      </div>
    </div>
  );
}
