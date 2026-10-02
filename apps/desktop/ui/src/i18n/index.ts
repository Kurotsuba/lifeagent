import type { CommandError, ErrorDetail, ImportMessage, MacroViolation } from "../api/types";
import { en } from "./en";
import { ja } from "./ja";
import { zhCN } from "./zh-CN";

/**
 * UI locale selection. Every component reads its copy from `strings`, the table for the
 * locale picked once at startup; switching language stores the choice and reloads the page,
 * so components can keep reading strings at module level instead of through a hook.
 *
 * The choice is per browser / per webview (localStorage), not per account: it must already
 * apply on the login screen, before there is an account to read it from.
 */

/** The zh-CN table's shape with every literal widened to `string`, so other locales can say
 * different things while the compiler still checks that each one has every key. */
type Widen<T> = T extends string
  ? string
  : T extends (...args: infer A) => infer R
    ? (...args: A) => Widen<R>
    : T extends readonly (infer U)[]
      ? readonly Widen<U>[]
      : { readonly [K in keyof T]: Widen<T[K]> };

export type Strings = Widen<typeof zhCN>;

/** One entry per variant of a tagged union `U` (tag field `Tag`): a fixed string, or a function
 * of the variant's fields. */
type MessagesFor<U extends Record<Tag, string>, Tag extends string> = {
  readonly [K in U[Tag]]: string | ((d: Extract<U, Record<Tag, K>>) => string);
};

// Compile-time checks that the reference table covers every backend variant (the other locales
// are checked against it through `Strings`). `food_data` is rendered via `importMessages`, and
// `invalid_macros` via `macroViolations`.
const _errorsComplete: MessagesFor<Exclude<ErrorDetail, { kind: "food_data" }>, "kind"> =
  zhCN.errors;
const _importComplete: MessagesFor<
  Exclude<ImportMessage, { kind: "invalid_macros" }>,
  "kind"
> = zhCN.importMessages;
const _macrosComplete: MessagesFor<MacroViolation, "rule"> = zhCN.macroViolations;
void _errorsComplete;
void _importComplete;
void _macrosComplete;

export const LOCALES = ["ja", "en", "zh-CN"] as const;
export type Locale = (typeof LOCALES)[number];

export const DEFAULT_LOCALE: Locale = "ja";

const TABLES: Record<Locale, Strings> = { ja, en, "zh-CN": zhCN };

/** Each language's name written in itself, for the language picker. */
export const LOCALE_NAMES: Record<Locale, string> = {
  ja: "日本語",
  en: "English",
  "zh-CN": "简体中文",
};

const STORAGE_KEY = "lifeagent.locale";

function isLocale(value: unknown): value is Locale {
  return typeof value === "string" && (LOCALES as readonly string[]).includes(value);
}

function readStoredLocale(): Locale {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return isLocale(stored) ? stored : DEFAULT_LOCALE;
  } catch {
    // Storage can be unavailable (private mode, blocked site data): use the default.
    return DEFAULT_LOCALE;
  }
}

export const locale: Locale = readStoredLocale();

export const strings: Strings = TABLES[locale];

if (typeof document !== "undefined") {
  document.documentElement.lang = locale;
}

/** Stores `next` and reloads so every module picks up the new table. */
export function setLocale(next: Locale) {
  if (next === locale) return;
  try {
    localStorage.setItem(STORAGE_KEY, next);
  } catch {
    return;
  }
  window.location.reload();
}

type Entry<T> = string | ((d: T) => string) | undefined;

function render<T>(entry: Entry<T>, d: T): string | undefined {
  return typeof entry === "function" ? entry(d) : entry;
}

/** An import error or warning in the current language. */
export function describeImportMessage(m: ImportMessage): string {
  if (m.kind === "invalid_macros") {
    const entry = strings.macroViolations[m.violation.rule] as Entry<MacroViolation>;
    return render(entry, m.violation) ?? m.violation.rule;
  }
  const entry = strings.importMessages[m.kind] as Entry<ImportMessage>;
  return render(entry, m) ?? m.kind;
}

/** A `CommandError` in the current language: from its `detail` when this build knows the kind,
 * otherwise the server's own (zh-CN) `message`. */
export function describeError(e: CommandError): string {
  const d: ErrorDetail | undefined = e.detail;
  if (!d) return e.message;
  if (d.kind === "food_data") return describeImportMessage(d.detail);
  const entry = strings.errors[d.kind] as Entry<ErrorDetail>;
  return render(entry, d) ?? e.message;
}
