import { useEffect, useRef, useState } from "react";
import {
  approveAction,
  getActivity,
  getActivityRun,
  getActivitySummary,
  rejectAction,
} from "../api/commands";
import { onActivityRecorded } from "../api/events";
import {
  commandErrorMessage,
  type ActivityActor,
  type ActivityRun,
  type ActivityRunInfo,
  type ActivityStep,
  type ActivitySummary,
  type RunStatus,
} from "../api/types";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { describeActionPreview, describeToolSummary, locale, strings, toolLabel } from "../i18n";

const t = strings.activity;
const PAGE = 30;

const STATUS_STYLES: Record<RunStatus, string> = {
  running: "bg-sky-100 text-sky-800 dark:bg-sky-950 dark:text-sky-300",
  succeeded: "bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300",
  needs_approval: "bg-amber-100 text-amber-900 dark:bg-amber-950 dark:text-amber-300",
  failed: "bg-red-100 text-red-800 dark:bg-red-950 dark:text-red-300",
  interrupted: "bg-neutral-200 text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300",
};

/**
 * The Activity tab (`docs/AUDIT_LOG_SPEC.md` §6.2): what the agent did, run by run, with each
 * run's steps on demand. Everything comes from `get_activity*`; counts and totals are computed
 * in Rust. Refreshes when a run ends (`activity-recorded`).
 */
export function Activity() {
  const [summary, setSummary] = useState<ActivitySummary | null>(null);
  const [runs, setRuns] = useState<ActivityRunInfo[] | null>(null);
  const [nextBefore, setNextBefore] = useState<string | null>(null);
  const [actor, setActor] = useState<ActivityActor | null>(null);
  const [status, setStatus] = useState<RunStatus | null>(null);
  const [open, setOpen] = useState<Record<string, ActivityRun | "loading">>({});
  const [error, setError] = useState<string | null>(null);
  // Reads the latest filters and open runs from the event callback without resubscribing.
  const latest = useRef({ actor, status, open });
  latest.current = { actor, status, open };

  async function refresh() {
    const { actor, status, open } = latest.current;
    try {
      const [page, counts] = await Promise.all([
        getActivity({ limit: PAGE, actor: actor ?? undefined, status: status ?? undefined }),
        getActivitySummary(7),
      ]);
      setRuns(page.runs);
      setNextBefore(page.next_before);
      setSummary(counts);
      setError(null);
      // Open runs may have moved on (an approval resolved, a running run ended).
      for (const id of Object.keys(open)) {
        void loadRun(id);
      }
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  async function loadMore() {
    if (!nextBefore) return;
    try {
      const page = await getActivity({
        before: nextBefore,
        limit: PAGE,
        actor: actor ?? undefined,
        status: status ?? undefined,
      });
      setRuns((current) => [...(current ?? []), ...page.runs]);
      setNextBefore(page.next_before);
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  async function loadRun(id: string) {
    try {
      const run = await getActivityRun(id);
      setOpen((current) => (id in current ? { ...current, [id]: run } : current));
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  function toggle(id: string) {
    if (open[id]) {
      setOpen(({ [id]: _closed, ...rest }) => rest);
      return;
    }
    setOpen((current) => ({ ...current, [id]: "loading" }));
    void loadRun(id);
  }

  useEffect(() => {
    void refresh();
    // Filters changed: refetch from the top.
  }, [actor, status]);

  useEffect(() => onActivityRecorded(() => void refresh()), []);

  return (
    <div className="h-full overflow-y-auto p-4 md:p-6">
      <div className="mx-auto flex max-w-2xl flex-col gap-5">
        <header className="space-y-2">
          <h1 className="text-lg font-semibold text-neutral-900 dark:text-neutral-50">
            {strings.nav.activity}
          </h1>
          <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.hint}</p>
          {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}
        </header>

        {summary ? <SummaryStrip summary={summary} /> : null}

        <div className="flex flex-wrap gap-2 text-sm">
          <FilterSelect
            value={actor}
            options={["user", "scheduler"] as const}
            label={(a) => t.actors[a]}
            onChange={setActor}
          />
          <FilterSelect
            value={status}
            options={["succeeded", "needs_approval", "failed", "running", "interrupted"] as const}
            label={(s) => t.statuses[s]}
            onChange={setStatus}
          />
        </div>

        {runs === null ? (
          <p className="text-sm text-neutral-500 dark:text-neutral-400">{strings.common.loading}</p>
        ) : runs.length === 0 ? (
          <p className="text-sm text-neutral-500 dark:text-neutral-400">{t.empty}</p>
        ) : (
          <RunList runs={runs} open={open} onToggle={toggle} onChanged={() => void refresh()} />
        )}

        {nextBefore ? (
          <button type="button" className={secondaryButtonClass} onClick={() => void loadMore()}>
            {t.loadMore}
          </button>
        ) : null}
      </div>
    </div>
  );
}

function SummaryStrip({ summary }: { summary: ActivitySummary }) {
  const tiles: { label: string; value: number; detail?: string }[] = [
    { label: t.stats.byUser, value: summary.runs_by_user },
    { label: t.stats.byScheduler, value: summary.runs_by_scheduler },
    {
      label: t.stats.approvals,
      value: summary.approvals_requested,
      detail: t.stats.approvalsDetail(summary.approvals_approved, summary.approvals_rejected),
    },
    { label: t.stats.guardrails, value: summary.guardrails },
    {
      label: t.stats.tokens,
      value: summary.tokens,
      detail: t.stats.last24h(summary.tokens_last_24h, summary.daily_token_budget),
    },
  ];
  return (
    <section className="space-y-2">
      <h2 className="text-xs font-medium text-neutral-500 dark:text-neutral-400">
        {t.period(summary.days)}
      </h2>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
        {tiles.map((tile) => (
          <div
            key={tile.label}
            className="rounded-lg border border-neutral-200 px-3 py-2 dark:border-neutral-700"
          >
            <p className="text-xs text-neutral-500 dark:text-neutral-400">{tile.label}</p>
            <p className="text-lg font-semibold tabular-nums text-neutral-900 dark:text-neutral-50">
              {tile.value.toLocaleString()}
            </p>
            {tile.detail ? (
              <p className="text-xs text-neutral-500 dark:text-neutral-400">{tile.detail}</p>
            ) : null}
          </div>
        ))}
      </div>
    </section>
  );
}

function FilterSelect<T extends string>({
  value,
  options,
  label,
  onChange,
}: {
  value: T | null;
  options: readonly T[];
  label: (option: T) => string;
  onChange: (value: T | null) => void;
}) {
  return (
    <select
      className="rounded-md border border-neutral-300 bg-white px-2 py-1 dark:border-neutral-600 dark:bg-neutral-800"
      value={value ?? ""}
      onChange={(e) => onChange(e.target.value === "" ? null : (e.target.value as T))}
    >
      <option value="">{t.filterAll}</option>
      {options.map((option) => (
        <option key={option} value={option}>
          {label(option)}
        </option>
      ))}
    </select>
  );
}

/** Runs grouped by logical date, newest first. */
function RunList({
  runs,
  open,
  onToggle,
  onChanged,
}: {
  runs: ActivityRunInfo[];
  open: Record<string, ActivityRun | "loading">;
  onToggle: (id: string) => void;
  onChanged: () => void;
}) {
  const days: { date: string; runs: ActivityRunInfo[] }[] = [];
  for (const run of runs) {
    const last = days[days.length - 1];
    if (last && last.date === run.logical_date) last.runs.push(run);
    else days.push({ date: run.logical_date, runs: [run] });
  }
  return (
    <div className="flex flex-col gap-4">
      {days.map((day) => (
        <section key={day.date} className="space-y-2">
          <h2 className="text-xs font-medium text-neutral-500 dark:text-neutral-400">{day.date}</h2>
          <ul className="flex flex-col gap-2">
            {day.runs.map((run) => (
              <RunItem
                key={run.id}
                run={run}
                detail={open[run.id]}
                onToggle={() => onToggle(run.id)}
                onChanged={onChanged}
              />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

function RunItem({
  run,
  detail,
  onToggle,
  onChanged,
}: {
  run: ActivityRunInfo;
  detail: ActivityRun | "loading" | undefined;
  onToggle: () => void;
  onChanged: () => void;
}) {
  const time = new Date(run.started_at).toLocaleTimeString(locale, {
    hour: "2-digit",
    minute: "2-digit",
  });
  return (
    <li className="rounded-lg border border-neutral-200 p-3 text-sm dark:border-neutral-700">
      <div className="flex flex-wrap items-center gap-2">
        <span
          className={
            "rounded px-1.5 py-0.5 text-xs font-medium " +
            (run.actor === "scheduler"
              ? "bg-violet-100 text-violet-800 dark:bg-violet-950 dark:text-violet-300"
              : "bg-neutral-200 text-neutral-700 dark:bg-neutral-700 dark:text-neutral-200")
          }
        >
          {t.actors[run.actor]}
        </span>
        <span className="font-medium text-neutral-900 dark:text-neutral-50">
          {t.triggers[run.trigger]}
        </span>
        <span className="text-xs tabular-nums text-neutral-500 dark:text-neutral-400">{time}</span>
        <span className={"rounded-full px-2 py-0.5 text-xs " + STATUS_STYLES[run.status]}>
          {t.statuses[run.status]}
        </span>
      </div>
      {run.input_excerpt ? (
        <p className="mt-1 [overflow-wrap:anywhere] text-neutral-700 dark:text-neutral-300">
          「{run.input_excerpt}」
        </p>
      ) : null}
      <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400">
        {[
          // A run that never reached the model (e.g. nothing to plan) has no counts worth showing.
          run.outcome && run.outcome.model_calls + run.outcome.tool_calls > 0
            ? t.counts(run.outcome)
            : "",
          run.outcome?.plan ? t.planSaved(run.outcome.plan.date, run.outcome.plan.version) : "",
          run.error_code ? (t.errorCodes[run.error_code] ?? t.errorCode(run.error_code)) : "",
        ]
          .filter((s) => s !== "")
          .join(" · ")}
      </p>
      <button
        type="button"
        className="mt-2 text-xs text-sky-700 hover:underline dark:text-sky-400"
        onClick={onToggle}
      >
        {detail ? t.hideSteps : t.showSteps}
      </button>
      {detail === "loading" ? (
        <p className="mt-2 text-xs text-neutral-500 dark:text-neutral-400">
          {strings.common.loading}
        </p>
      ) : detail ? (
        <ol className="mt-2 space-y-1.5 border-l-2 border-neutral-200 pl-3 dark:border-neutral-700">
          {detail.steps.map((step) => (
            <StepItem key={step.id} step={step} onChanged={onChanged} />
          ))}
        </ol>
      ) : null}
    </li>
  );
}

function StepItem({ step, onChanged }: { step: ActivityStep; onChanged: () => void }) {
  const muted = "text-xs text-neutral-500 dark:text-neutral-400";
  switch (step.kind) {
    case "model_call": {
      const d = step.detail;
      return (
        <li className={muted}>
          {t.modelCall(
            t.roles[d.role] ?? d.role,
            d.model,
            d.prompt_tokens + d.completion_tokens,
            (d.latency_ms / 1000).toFixed(1),
            d.ok,
          )}
        </li>
      );
    }
    case "tool_call": {
      const d = step.detail;
      const ok = d.outcome === "ran";
      return (
        <li className="text-neutral-800 dark:text-neutral-200">
          <span className="font-medium">{toolLabel(d.tool)}</span>
          <span className={ok ? " text-emerald-700 dark:text-emerald-400" : " text-red-700 dark:text-red-400"}>
            {" · "}
            {t.toolOutcomes[d.outcome]}
          </span>
          {d.summary ? (
            <span className="block [overflow-wrap:anywhere]">
              {describeToolSummary(d.summary, d.tool)}
            </span>
          ) : null}
          {d.error ? (
            <span className={"block [overflow-wrap:anywhere] " + muted}>{d.error}</span>
          ) : null}
          <Arguments value={d.arguments} />
        </li>
      );
    }
    case "approval_requested": {
      const d = step.detail;
      return (
        <li className="rounded-md bg-amber-50 px-2 py-1.5 text-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
          <span className="font-medium">{t.approvalRequested}</span>
          <span className="block [overflow-wrap:anywhere]">
            {describeActionPreview(d.preview, d.tool)}
          </span>
          {step.action_status === "pending" ? (
            <ApprovalButtons actionId={d.action_id} onChanged={onChanged} />
          ) : step.action_status ? (
            <span className="block text-xs">{t.actionStatus[step.action_status]}</span>
          ) : null}
          <Arguments value={d.arguments} />
        </li>
      );
    }
    case "approval_resolved": {
      const d = step.detail;
      return (
        <li className="text-neutral-800 dark:text-neutral-200">
          <span className="font-medium">{t.approvalResolved[d.decision]}</span>
          {" · "}
          {toolLabel(d.tool)}
          {d.result ? (
            <span className="block [overflow-wrap:anywhere]">
              {describeToolSummary(d.result, d.tool)}
            </span>
          ) : null}
          {d.requested_in ? <span className={"block " + muted}>{t.fromEarlierRun}</span> : null}
        </li>
      );
    }
    case "guardrail": {
      const d = step.detail;
      return (
        <li className="rounded-md bg-red-50 px-2 py-1.5 font-medium text-red-800 dark:bg-red-950/40 dark:text-red-300">
          {t.guardrails[d.rule] ?? d.rule}
          {d.tool ? <span className="font-normal"> · {toolLabel(d.tool)}</span> : null}
        </li>
      );
    }
    case "plan_validation": {
      const d = step.detail;
      return (
        <li className="text-neutral-800 dark:text-neutral-200">
          {t.planValidation(d)}
          {d.violations.length > 0 ? (
            <ul className={"mt-0.5 list-disc pl-4 " + muted}>
              {d.violations.map((v, i) => (
                <li key={i} className="[overflow-wrap:anywhere]">
                  {v}
                </li>
              ))}
            </ul>
          ) : null}
        </li>
      );
    }
    case "plan_saved":
      return (
        <li className="text-neutral-800 dark:text-neutral-200">
          {t.planSavedStep(step.detail.date, step.detail.version)}
        </li>
      );
  }
}

/** The tool's arguments as the model sent them, folded away by default. */
function Arguments({ value }: { value: unknown }) {
  return (
    <details className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
      <summary className="cursor-pointer select-none">{t.arguments}</summary>
      <pre className="mt-1 overflow-x-auto whitespace-pre-wrap [overflow-wrap:anywhere] rounded bg-neutral-100 p-2 dark:bg-neutral-800">
        {JSON.stringify(value, null, 2)}
      </pre>
    </details>
  );
}

function ApprovalButtons({ actionId, onChanged }: { actionId: string; onChanged: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function resolve(approve: boolean) {
    setBusy(true);
    setError(null);
    try {
      await (approve ? approveAction(actionId) : rejectAction(actionId));
      onChanged();
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <span className="mt-1 flex flex-wrap items-center gap-2">
      <button
        type="button"
        className={primaryButtonClass}
        disabled={busy}
        onClick={() => void resolve(true)}
      >
        {strings.approvals.approve}
      </button>
      <button
        type="button"
        className={secondaryButtonClass}
        disabled={busy}
        onClick={() => void resolve(false)}
      >
        {strings.approvals.reject}
      </button>
      {error ? <span className="text-xs text-red-600 dark:text-red-400">{error}</span> : null}
    </span>
  );
}
