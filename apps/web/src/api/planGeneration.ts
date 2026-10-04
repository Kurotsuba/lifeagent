import { useSyncExternalStore } from "react";
import { generatePlan, isPlanGenerating } from "./commands";
import { onPlanGenerating } from "./events";
import { commandErrorMessage } from "./types";

/**
 * Whether a plan generation is under way, for the Nutrition and Training generate buttons. Kept
 * outside any component so it survives switching pages, and mirrored from the server (which
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
  error: string | null;
}

/** `local`: this tab's own request is in flight (set before the server's event arrives). */
let local = false;
let server = false;
let error: string | null = null;
let state: PlanGenerationState = { pending: false, error: null };
const listeners = new Set<() => void>();

function update() {
  const pending = local || server;
  if (pending === state.pending && error === state.error) return;
  state = { pending, error };
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
export async function startPlanGeneration(
  date?: string,
  request?: string,
  before?: () => Promise<void>,
): Promise<boolean> {
  if (state.pending) return false;
  local = true;
  error = null;
  update();
  try {
    await before?.();
    await generatePlan(date, request);
    return true;
  } catch (e) {
    error = commandErrorMessage(e);
    return false;
  } finally {
    local = false;
    update();
  }
}
