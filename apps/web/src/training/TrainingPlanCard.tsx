import type { TrainingPlanSection } from "../api/types";
import { strings } from "../i18n";
import { exerciseLabel } from "./format";

const t = strings.training.plan;

/** One day's planned training, as finalized by the training plugin (duration and energy are
 * the app's own figures). */
export function TrainingPlanCard({ section }: { section: TrainingPlanSection }) {
  if (section.rest_day) {
    return (
      <div className="text-sm">
        <p className="font-medium text-neutral-900 dark:text-neutral-50">{t.restDay}</p>
        <ExerciseList section={section} />
        <Tips tips={section.tips} />
      </div>
    );
  }

  const meta = [
    section.time_of_day ? t.timeOfDay[section.time_of_day] : null,
    t.minutes(Math.round(section.duration_min)),
    t.intensity[section.intensity],
    section.exercise_kcal != null ? t.kcal(Math.round(section.exercise_kcal)) : null,
  ].filter((part): part is string => part !== null);

  return (
    <div className="text-sm">
      <p className="font-medium text-neutral-900 dark:text-neutral-50">
        {section.title || t.untitled}
      </p>
      <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
        {meta.join(" · ")}
      </p>
      {section.focus ? (
        <p className="mt-1 text-neutral-600 dark:text-neutral-400">{section.focus}</p>
      ) : null}
      <ExerciseList section={section} />
      <Tips tips={section.tips} />
    </div>
  );
}

function ExerciseList({ section }: { section: TrainingPlanSection }) {
  if (section.exercises.length === 0) return null;
  return (
    <ul className="mt-2 flex flex-col gap-1">
      {section.exercises.map((e, i) => (
        <li
          key={i}
          className="rounded-md border border-neutral-200 px-3 py-1.5 dark:border-neutral-700"
        >
          <span className="text-neutral-900 dark:text-neutral-50">{exerciseLabel(e)}</span>
          {e.note ? (
            <span className="ml-2 text-xs italic text-neutral-500 dark:text-neutral-400">
              {e.note}
            </span>
          ) : null}
        </li>
      ))}
    </ul>
  );
}

function Tips({ tips }: { tips: string[] }) {
  if (tips.length === 0) return null;
  return (
    <ul className="mt-2 list-inside list-disc text-xs text-neutral-500 dark:text-neutral-400">
      {tips.map((tip, i) => (
        <li key={i}>{tip}</li>
      ))}
    </ul>
  );
}
