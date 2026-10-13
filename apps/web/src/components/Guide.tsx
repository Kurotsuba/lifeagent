import { useEffect, useState } from "react";
import { strings } from "../i18n";
import { primaryButtonClass, secondaryButtonClass } from "./buttonStyles";

/**
 * A short step-by-step guide in a pop-up. The core tour of the main tabs is shown once per
 * account on this browser and again from the sidebar's ? button; each plugin page has its own
 * guide behind a small ? next to its title (`HelpButton`). "Seen" (core tour only) lives in
 * localStorage, like the locale: it is a view preference, not account data, so a new browser
 * shows the tour once more.
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

export function markGuideSeen(username: string | null) {
  try {
    localStorage.setItem(SEEN_KEY_PREFIX + (username ?? ""), "1");
  } catch {
    // Best effort.
  }
}

export interface GuideStep {
  readonly title: string;
  readonly body: string;
}

interface GuideProps {
  steps: readonly GuideStep[];
  /** Label of the button on the last step. */
  doneLabel: string;
  onClose: () => void;
}

export function Guide({ steps, doneLabel, onClose }: GuideProps) {
  const [index, setIndex] = useState(0);
  const step = steps[index];
  const last = index === steps.length - 1;
  const close = onClose;

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
              {last ? doneLabel : strings.common.next}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

interface HelpButtonProps {
  /** Which page's guide to open. */
  topic: keyof typeof strings.guide.pages;
}

/** A small ? beside a page title that opens that page's guide. */
export function HelpButton({ topic }: HelpButtonProps) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        aria-label={strings.guide.pageHelp}
        title={strings.guide.pageHelp}
        onClick={() => setOpen(true)}
        className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-neutral-400 align-middle text-xs font-medium text-neutral-500 transition-colors hover:border-neutral-700 hover:text-neutral-800 dark:border-neutral-500 dark:text-neutral-400 dark:hover:border-neutral-200 dark:hover:text-neutral-100"
      >
        ?
      </button>
      {open ? (
        <Guide
          steps={strings.guide.pages[topic]}
          doneLabel={strings.guide.gotIt}
          onClose={() => setOpen(false)}
        />
      ) : null}
    </>
  );
}
