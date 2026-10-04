/**
 * Subscribes to the three `AppEvent`s the Rust side pushes (`crates/api/src/events.rs`,
 * `docs/PHASE1_SPEC.md` §10.1): `data-changed`, `plan-ready`, `plan-failed`, delivered as
 * Server-Sent Events from `/api/events`. Today and Chat both just want to know "something
 * changed, go refetch" — neither needs the payload, so this collapses all three into one
 * callback. `plan-generating` is the exception, with its own subscription: its payload is the
 * state the generate buttons show.
 */

const EVENT_NAMES = ["data-changed", "plan-ready", "plan-failed"] as const;

/** Calls `onEvent` whenever any of the three events arrives. Returns an unsubscribe function;
 * call it from a `useEffect` cleanup. `EventSource` reconnects by itself after a dropped
 * connection (e.g. a server restart). */
export function onAppDataEvent(onEvent: () => void): () => void {
  const source = new EventSource("/api/events");
  for (const name of EVENT_NAMES) {
    source.addEventListener(name, onEvent);
  }
  return () => source.close();
}

/** Calls `onChange` with the `event_type` of every `data-changed` event (e.g.
 * `"diet.meal_logged"`), for the few views that care which data changed. Returns an
 * unsubscribe function. */
export function onDataChanged(onChange: (eventType: string) => void): () => void {
  const source = new EventSource("/api/events");
  source.addEventListener("data-changed", (e) => {
    try {
      onChange((JSON.parse(e.data) as { event_type: string }).event_type);
    } catch {
      // A malformed payload is not worth surfacing; the next event carries on.
    }
  });
  return () => source.close();
}

/** Calls `onChange` with the server's `plan-generating` flag whenever it changes, and with
 * `null` whenever the stream (re)opens, since changes made while it was closed were missed.
 * Returns an unsubscribe function. */
export function onPlanGenerating(onChange: (generating: boolean | null) => void): () => void {
  const source = new EventSource("/api/events");
  source.addEventListener("open", () => onChange(null));
  source.addEventListener("plan-generating", (e) => {
    try {
      onChange((JSON.parse(e.data) as { generating: boolean }).generating);
    } catch {
      onChange(null);
    }
  });
  return () => source.close();
}
