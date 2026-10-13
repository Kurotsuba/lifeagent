import { useEffect, useState } from "react";
import { getTrainingView } from "../api/commands";
import { onAppDataEvent } from "../api/events";
import { commandErrorMessage, type TrainingView } from "../api/types";
import { HelpButton } from "../components/Guide";
import { strings } from "../i18n";
import { PreferencesForm } from "./PreferencesForm";
import { TomorrowTraining } from "./TomorrowTraining";
import { TrainingPlanCard } from "./TrainingPlanCard";
import { WeekLog } from "./WeekLog";

const t = strings.training;

function Card({ children }: { children: React.ReactNode }) {
  return (
    <section className="rounded-lg border border-neutral-200 p-4 dark:border-neutral-700">
      {children}
    </section>
  );
}

/** The training plugin's page: the last 7 days' log, today's and tomorrow's planned training
 * with the optional request for tomorrow, and standing preferences. All data from
 * `get_training_view`; refetched on every app data event (a session logged in chat, a plan
 * generated). */
export function Training() {
  const [view, setView] = useState<TrainingView | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function refetch() {
    try {
      setView(await getTrainingView());
      setError(null);
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  useEffect(() => {
    void refetch();
    return onAppDataEvent(() => void refetch());
  }, []);

  if (!view) {
    return (
      <div className="p-6">
        <p
          className={
            "text-sm " +
            (error ? "text-red-600 dark:text-red-400" : "text-neutral-500 dark:text-neutral-400")
          }
        >
          {error ?? strings.common.loading}
        </p>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto p-6">
      <div className="mx-auto flex max-w-2xl flex-col gap-6">
        <div className="flex items-center gap-2">
          <h1 className="text-lg font-semibold text-neutral-900 dark:text-neutral-50">{t.title}</h1>
          <HelpButton topic="training" />
        </div>
        {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}

        <Card>
          <WeekLog view={view} onDeleted={() => void refetch()} />
        </Card>

        {view.today_plan ? (
          <Card>
            <h2 className="mb-3 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
              {t.todayPlan.title}
            </h2>
            <TrainingPlanCard section={view.today_plan} />
          </Card>
        ) : null}

        <Card>
          <TomorrowTraining view={view} onChanged={() => void refetch()} />
        </Card>

        <Card>
          <PreferencesForm preferences={view.preferences} onSaved={() => void refetch()} />
        </Card>
      </div>
    </div>
  );
}
