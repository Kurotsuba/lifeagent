/**
 * Subscribes to the three `AppEvent`s the Rust side pushes (`crates/api/src/events.rs`,
 * `docs/PHASE1_SPEC.md` §10.1): `data-changed`, `plan-ready`, `plan-failed`, delivered as
 * Server-Sent Events from `/api/events`. Today and Chat both just want to know "something
 * changed, go refetch" — neither needs the payload, so this collapses all three into one
 * callback. `plan-generating` is the exception, with its own subscription: its payload is the
 * state the generate buttons show.
 *
 * All subscriptions share one `EventSource`, opened by the first and closed with the last: each
 * open stream holds one of the server's request slots (one Cloud Run instance, see
 * `deploy/deploy.sh`), so a stream per component used them up. The server ends each stream
 * after a few minutes and `EventSource` reconnects by itself, which fires `open` again.
 */

const EVENT_NAMES = ["data-changed", "plan-ready", "plan-failed"] as const;

type Handlers = Record<string, (e: MessageEvent<string>) => void>;

let shared: EventSource | null = null;
let subscribers = 0;

/** Adds `handlers` (by event name, `open` included) to the shared stream. A subscriber that
 * joins an already open stream gets its `open` handler called once, as it would on a stream of
 * its own. Returns an unsubscribe function. */
function subscribe(handlers: Handlers): () => void {
  if (!shared) shared = new EventSource("/api/events");
  const source = shared;
  subscribers++;
  const listeners = Object.entries(handlers).map(([name, handler]) => {
    const listener = (e: Event) => handler(e as MessageEvent<string>);
    source.addEventListener(name, listener);
    return [name, listener] as const;
  });
  const open = handlers.open;
  if (open && source.readyState === EventSource.OPEN) {
    queueMicrotask(() => open(new MessageEvent("open")));
  }
  return () => {
    for (const [name, listener] of listeners) source.removeEventListener(name, listener);
    subscribers--;
    if (subscribers === 0 && shared === source) {
      source.close();
      shared = null;
    }
  };
}

/** Calls `onEvent` whenever any of the three events arrives. Returns an unsubscribe function;
 * call it from a `useEffect` cleanup. */
export function onAppDataEvent(onEvent: () => void): () => void {
  return subscribe(Object.fromEntries(EVENT_NAMES.map((name) => [name, onEvent])));
}

/** Calls `onChange` with the `event_type` of every `data-changed` event (e.g.
 * `"diet.meal_logged"`), for the few views that care which data changed. Returns an
 * unsubscribe function. */
export function onDataChanged(onChange: (eventType: string) => void): () => void {
  return subscribe({
    "data-changed": (e) => {
      try {
        onChange((JSON.parse(e.data) as { event_type: string }).event_type);
      } catch {
        // A malformed payload is not worth surfacing; the next event carries on.
      }
    },
  });
}

/** Calls `onChange` with the server's `plan-generating` flag whenever it changes, and with
 * `null` whenever the stream (re)opens, since changes made while it was closed were missed.
 * Returns an unsubscribe function. */
export function onPlanGenerating(onChange: (generating: boolean | null) => void): () => void {
  return subscribe({
    open: () => onChange(null),
    "plan-generating": (e) => {
      try {
        onChange((JSON.parse(e.data) as { generating: boolean }).generating);
      } catch {
        onChange(null);
      }
    },
  });
}

/** Calls `onChange` whenever follow-ups may have changed (`follow-ups-changed`, or a
 * `data-changed` that may clear one), and whenever the stream (re)opens. Returns an unsubscribe
 * function. */
export function onFollowUpsChanged(onChange: () => void): () => void {
  return subscribe({
    open: onChange,
    "follow-ups-changed": onChange,
    "data-changed": onChange,
    "plan-ready": onChange,
  });
}

/** Calls `onRun` with the id of every agent run that ends (`activity-recorded`), and with `null`
 * whenever the stream (re)opens, since runs that ended meanwhile were missed. Returns an
 * unsubscribe function. */
export function onActivityRecorded(onRun: (runId: string | null) => void): () => void {
  return subscribe({
    open: () => onRun(null),
    "activity-recorded": (e) => {
      try {
        onRun((JSON.parse(e.data) as { run_id: string }).run_id);
      } catch {
        onRun(null);
      }
    },
  });
}
