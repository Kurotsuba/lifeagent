import { locale, LOCALE_NAMES, LOCALES, setLocale, strings, type Locale } from "../i18n";

/** The UI language picker. Choosing a language reloads the page (see `setLocale`). */
export function LanguageSelect({ className = "" }: { className?: string }) {
  return (
    <select
      aria-label={strings.settings.sections.language}
      className={
        "rounded-md border border-neutral-300 bg-white px-2 py-1 text-base text-neutral-900 md:text-sm " +
        "dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 " +
        className
      }
      value={locale}
      onChange={(e) => setLocale(e.target.value as Locale)}
    >
      {LOCALES.map((l) => (
        <option key={l} value={l}>
          {LOCALE_NAMES[l]}
        </option>
      ))}
    </select>
  );
}
