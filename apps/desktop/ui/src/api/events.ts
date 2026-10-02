/**
 * Subscribes to the three `AppEvent`s the Rust side pushes (`crates/api/src/events.rs`,
 * `docs/PHASE1_SPEC.md` §10.1): `data-changed`, `plan-ready`, `plan-failed`. The desktop gets
 * them as Tauri events, the web build as Server-Sent Events from `/api/events` (same names).
 * Today and Chat both just want to know "something changed, go refetch" — neither needs the
 * payload, so this collapses all three into one callback.
 */

import { listen } from "@tauri-apps/api/event";
import { isDesktop } from "./platform";

const EVENT_NAMES = ["data-changed", "plan-ready", "plan-failed"] as const;

/** Calls `onEvent` whenever any of the three events arrives. Returns an unsubscribe function;
 * call it from a `useEffect` cleanup. */
export function onAppDataEvent(onEvent: () => void): () => void {
  return isDesktop ? listenTauri(onEvent) : listenServer(onEvent);
}

function listenTauri(onEvent: () => void): () => void {
  let cancelled = false;
  const unlistenPromises = EVENT_NAMES.map((name) =>
    listen(name, () => {
      if (!cancelled) onEvent();
    }),
  );
  return () => {
    cancelled = true;
    for (const p of unlistenPromises) {
      p.then((unlisten) => unlisten()).catch(() => {
        // Nothing to clean up if the listener never registered.
      });
    }
  };
}

/** `EventSource` reconnects by itself after a dropped connection (e.g. a server restart). */
function listenServer(onEvent: () => void): () => void {
  const source = new EventSource("/api/events");
  for (const name of EVENT_NAMES) {
    source.addEventListener(name, onEvent);
  }
  return () => source.close();
}
