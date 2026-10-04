import { useSyncExternalStore } from "react";
import { generatePlan, isPlanGenerating, revisePlan } from "./commands";
import { onPlanGenerating } from "./events";
import { commandErrorMessage } from "./types";

/**
 * Whether a plan generation or revision is under way, for the Nutrition and Training plan
 * buttons. Kept outside any component so it survives switching pages, and mirrored from the server (which
 * turns away a second manual generation, `plan_busy`) so it also holds after a reload, in a
 * second tab, and while the chat or the scheduler generates. Both pages generate the same
 * (whole) plan, so they share one flag.
 *
 * A manual generation's failure only reaches its caller (the server publishes `plan-failed`
 * for scheduled runs only), so the last error is kept here too, for a page that remounts after
 * its request failed. Success needs nothing extra: `plan-ready` makes the pages refetch.
 */
export interface PlanGenerationState {
  pending: boolean;
  /** This tab's own request is a revision (Adjust), so that button shows the progress. */
  revising: boolean;
  error: string | null;
}

/** `local`: this tab's own request is in flight (set before the server's event arrives). */
let local: "generate" | "revise" | null = null;
let server = false;
let error: string | null = null;
let state: PlanGenerationState = {
  pending: false,
  revising: false,
  error: null,
};
const listeners = new Set<() => void>();

function update() {
  const pending = local !== null || server;
  const revising = local === "revise";
  if (
    pending === state.pending &&
    revising === state.revising &&
    error === state.error
  )
    return;
  state = { pending, revising, error };
  for (const listener of listeners) listener();
}

// The server stream is open only while some button is on screen, and the flag is refetched
// each time it (re)opens. `seq` drops a fetched value an event has overtaken.
let unsubscribeServer: (() => void) | null = null;
let seq = 0;

function refreshServer() {
  const asked = ++seq;
  isPlanGenerating()
    .then((generating) => {
      if (asked !== seq) return;
      server = generating;
      update();
    })
    .catch(() => {
      // Leave the last known value; the next (re)connect asks again.
    });
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (!unsubscribeServer) {
    unsubscribeServer = onPlanGenerating((generating) => {
      if (generating === null) {
        refreshServer();
        return;
      }
      seq++;
      server = generating;
      update();
    });
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && unsubscribeServer) {
      unsubscribeServer();
      unsubscribeServer = null;
    }
  };
}

export function usePlanGeneration(): PlanGenerationState {
  return useSyncExternalStore(subscribe, () => state);
}

/** Runs `before` (e.g. saving a pending request) and then `generate_plan`, unless a generation
 * is already under way. Resolves to whether the plan was generated; a failure is recorded in
 * the shared state rather than thrown. */
export function startPlanGeneration(
  date?: string,
  request?: string,
  before?: () => Promise<void>,
): Promise<boolean> {
  return runExclusive("generate", async () => {
    await before?.();
    await generatePlan(date, request);
  });
}

/** `revise_plan` under the same flag as a generation: the server runs both one at a time, and
 * either one replaces the plan both pages show. */
export function startPlanRevision(
  date: string,
  instruction: string,
): Promise<boolean> {
  return runExclusive("revise", () =>
    revisePlan(date, instruction).then(() => undefined),
  );
}

async function runExclusive(
  kind: "generate" | "revise",
  work: () => Promise<void>,
): Promise<boolean> {
  if (state.pending) return false;
  local = kind;
  error = null;
  update();
  try {
    await work();
    return true;
  } catch (e) {
    error = commandErrorMessage(e);
    return false;
  } finally {
    local = null;
    update();
  }
}
