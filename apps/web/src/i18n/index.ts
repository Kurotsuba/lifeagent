import type {
  ActionPreview,
  CommandError,
  DietMealItem,
  ErrorDetail,
  FollowUpData,
  ImportMessage,
  MacroViolation,
  PlanNoteData,
  SourceKind,
  StoredPlanNote,
  ToolSummary,
} from "../api/types";
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
const _previewsComplete: MessagesFor<ActionPreview, "kind"> = zhCN.approvals.previews;
const _toolSummariesComplete: MessagesFor<ToolSummary, "kind"> = zhCN.toolSummaries;
const _planNotesComplete: MessagesFor<PlanNoteData, "kind"> = zhCN.planNotes;
void _planNotesComplete;
void _toolSummariesComplete;
void _previewsComplete;
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

/** A tool's name as a label in the current language; the raw name if this build doesn't
 * know it. */
export function toolLabel(tool: string): string {
  return strings.tools[tool] ?? tool;
}

/** A plan warning or violation in the current language. A kind this build doesn't know falls
 * back to the server's English `text`; a note saved before notes were data is shown as stored. */
export function describePlanNote(note: StoredPlanNote): string {
  if (typeof note === "string") return note;
  const entry = (strings.planNotes as Record<string, Entry<never>>)[note.kind];
  const text = render(entry, note as never) ?? note.text;
  return note.unmet ? strings.planNotes.unmet(text) : text;
}

/** A food source's display name. The two built-in sources are seeded with a fixed zh-CN name
 * in the database, so they are named from the current locale; datasets keep the name the user
 * gave them. */
export function sourceDisplayName(kind: SourceKind, name: string): string {
  if (kind === "user") return strings.settings.foodData.kindManual;
  if (kind === "llm_estimate") return strings.settings.foodData.kindEstimate;
  return name;
}

/** A meal or plan item's tooltip: where its nutrition comes from, as `source:row`, or just the
 * row for meals logged before the source was recorded. */
export function mealItemSource(item: DietMealItem): string {
  return item.source_name === null
    ? item.name
    : `${sourceDisplayName(item.source_kind, item.source_name)}:${item.name}`;
}

/** What a tool call did, in the current language. A kind this build doesn't know (a newer
 * server, a plugin without translations) falls back to the tool's label, then the raw kind. */
export function describeToolSummary(summary: ToolSummary | { kind: string }, tool?: string): string {
  const entry = (strings.toolSummaries as Record<string, Entry<never>>)[summary.kind];
  return render(entry, summary as never) ?? (tool ? toolLabel(tool) : summary.kind);
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

/** What a pending action will do, in the current language; a preview kind this build doesn't
 * know falls back to naming the tool. */
export function describeActionPreview(preview: ActionPreview, toolName: string): string {
  const entry = strings.approvals.previews[preview.kind] as Entry<ActionPreview>;
  return render(entry, preview) ?? strings.approvals.unknown(toolName);
}

/** What a follow-up asks, in the current language: one line, or several for a day review. */
export function describeFollowUp(f: FollowUpData): string[] {
  const t = strings.followUps;
  switch (f.kind) {
    case "diet_meal_missed":
      return [
        t.mealMissed({
          meal: strings.mealType[f.params.meal_type],
          title: f.params.title,
          kcal: f.params.kcal,
        }),
      ];
    case "diet_day_off_target": {
      const p = f.params;
      const lines: string[] = [t.dayOffTarget.intro];
      if (p.issues.includes("kcal_over") || p.issues.includes("kcal_under")) {
        lines.push(t.dayOffTarget.kcal({ kcal: p.kcal, target: p.target_kcal, diff: p.kcal_diff }));
      }
      if (p.issues.includes("protein_under")) {
        lines.push(
          t.dayOffTarget.protein({
            protein: p.protein_g,
            target: p.target_protein_g,
            diff: p.protein_diff_g,
          }),
        );
      }
      lines.push(p.has_plan_tomorrow ? t.dayOffTarget.askAdjust : t.dayOffTarget.askGenerate);
      return lines;
    }
  }
}

/** A follow-up answer's button label; an answer kind this build doesn't know shows raw. */
export function followUpAnswerLabel(answer: string, f: FollowUpData): string {
  const answers = strings.followUps.answers;
  if (
    answer === "core_adjust_tomorrow" &&
    f.kind === "diet_day_off_target" &&
    !f.params.has_plan_tomorrow
  ) {
    return answers.generate_tomorrow;
  }
  return answers[answer] ?? answer;
}
