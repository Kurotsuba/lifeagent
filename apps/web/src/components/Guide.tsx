import { useEffect, useState } from "react";
import { strings } from "../i18n";
import { primaryButtonClass, secondaryButtonClass } from "./buttonStyles";

/**
 * A short tour of the main tabs, shown once per account on this browser and again from the
 * sidebar's ? button. "Seen" lives in localStorage, like the locale: it is a view preference,
 * not account data, so a new browser shows the tour once more.
 */
const SEEN_KEY_PREFIX = "lifeagent.guideSeen.";

export function guideSeen(username: string | null): boolean {
  try {
    return localStorage.getItem(SEEN_KEY_PREFIX + (username ?? "")) === "1";
  } catch {
    // Storage unavailable: don't nag on every load.
    return true;
  }
}

function markGuideSeen(username: string | null) {
  try {
    localStorage.setItem(SEEN_KEY_PREFIX + (username ?? ""), "1");
  } catch {
    // Best effort.
  }
}

interface GuideProps {
  username: string | null;
  onClose: () => void;
}

export function Guide({ username, onClose }: GuideProps) {
  const steps = strings.guide.steps;
  const [index, setIndex] = useState(0);
  const step = steps[index];
  const last = index === steps.length - 1;

  function close() {
    markGuideSeen(username);
    onClose();
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 px-4"
      onClick={close}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="guide-title"
        className="w-full max-w-md rounded-lg border border-neutral-200 bg-white p-5 shadow-xl dark:border-neutral-700 dark:bg-neutral-800"
        onClick={(e) => e.stopPropagation()}
      >
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          {strings.guide.stepCounter(index + 1, steps.length)}
        </p>
        <h2
          id="guide-title"
          className="mt-1 text-lg font-semibold text-neutral-900 dark:text-neutral-50"
        >
          {step.title}
        </h2>
        <p className="mt-2 text-sm leading-relaxed whitespace-pre-line text-neutral-700 dark:text-neutral-300">
          {step.body}
        </p>
        <div className="mt-4 flex justify-center gap-1.5" aria-hidden="true">
          {steps.map((s, i) => (
            <span
              key={s.title}
              className={
                "h-1.5 w-1.5 rounded-full " +
                (i === index
                  ? "bg-neutral-900 dark:bg-neutral-100"
                  : "bg-neutral-300 dark:bg-neutral-600")
              }
            />
          ))}
        </div>
        <div className="mt-4 flex items-center gap-2">
          {!last ? (
            <button
              type="button"
              className="text-sm text-neutral-500 underline hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100"
              onClick={close}
            >
              {strings.guide.skip}
            </button>
          ) : null}
          <div className="ml-auto flex gap-2">
            {index > 0 ? (
              <button
                type="button"
                className={secondaryButtonClass}
                onClick={() => setIndex(index - 1)}
              >
                {strings.common.back}
              </button>
            ) : null}
            <button
              type="button"
              autoFocus
              className={primaryButtonClass}
              onClick={() => (last ? close() : setIndex(index + 1))}
            >
              {last ? strings.guide.done : strings.common.next}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
