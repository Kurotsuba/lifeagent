/**
 * Subscribes to the three `AppEvent`s the Rust side pushes (`crates/api/src/events.rs`,
 * `docs/PHASE1_SPEC.md` §10.1): `data-changed`, `plan-ready`, `plan-failed`, delivered as
 * Server-Sent Events from `/api/events`. Today and Chat both just want to know "something
 * changed, go refetch" — neither needs the payload, so this collapses all three into one
 * callback.
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
