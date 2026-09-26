/**
 * Subscribes to the three `AppEvent`s the Rust side forwards to the webview
 * (`apps/desktop/src-tauri/src/events.rs`, `docs/PHASE1_SPEC.md` §10.1): `data-changed`,
 * `plan-ready`, `plan-failed`. Today and Chat both just want to know "something changed, go
 * refetch" — neither needs the payload, so this collapses all three into one callback.
 */

import { listen } from "@tauri-apps/api/event";

const EVENT_NAMES = ["data-changed", "plan-ready", "plan-failed"] as const;

/** Calls `onEvent` whenever any of the three events arrives. Returns an unsubscribe function;
 * call it from a `useEffect` cleanup. */
export function onAppDataEvent(onEvent: () => void): () => void {
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
