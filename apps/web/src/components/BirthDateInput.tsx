import { useEffect, useState } from "react";
import { strings } from "../i18n";
import { inputClass } from "./Field";

interface BirthDateInputProps {
  /** `YYYY-MM-DD`, or `""` until year, month and day are all picked. */
  value: string;
  onChange: (value: string) => void;
  /** The allowed ages, which bound the year list (the server still checks the exact date). */
  minAge: number;
  maxAge: number;
}

interface Parts {
  year: string;
  month: string;
  day: string;
}

function partsOf(value: string): Parts {
  const [year = "", month = "", day = ""] = value.split("-");
  return { year, month, day };
}

function daysIn(year: string, month: string): number {
  if (!month) return 31;
  // Day 0 of the next month is this month's last day; a leap year only matters for February.
  return new Date(Number(year || 2000), Number(month), 0).getDate();
}

const pad = (n: number) => String(n).padStart(2, "0");

/** Year, month and day as three selects, year first. A native date picker opens on today, so
 * on a phone reaching a birth year means swiping back decades; here the year list starts at the
 * most recent year old enough. */
export function BirthDateInput({ value, onChange, minAge, maxAge }: BirthDateInputProps) {
  const t = strings.common.birthDate;
  const [parts, setParts] = useState<Parts>(() => partsOf(value));

  // A value set from outside (Settings loading the saved profile) replaces the picks; our own
  // `""` for a half-picked date doesn't.
  useEffect(() => {
    if (value !== "") setParts(partsOf(value));
  }, [value]);

  function pick(patch: Partial<Parts>) {
    const next = { ...parts, ...patch };
    const last = daysIn(next.year, next.month);
    if (next.day && Number(next.day) > last) next.day = pad(last);
    setParts(next);
    onChange(next.year && next.month && next.day ? `${next.year}-${next.month}-${next.day}` : "");
  }

  // Calendar years only: the exact age check against the logical date is the server's.
  const thisYear = new Date().getFullYear();
  const years: number[] = [];
  for (let y = thisYear - minAge; y >= thisYear - maxAge - 1; y--) years.push(y);
  const days = daysIn(parts.year, parts.month);

  const selectClass = inputClass + " px-2";
  return (
    <span className="grid grid-cols-[2fr_1fr_1fr] gap-2">
      <select
        className={selectClass}
        aria-label={t.year}
        value={parts.year}
        onChange={(e) => pick({ year: e.target.value })}
      >
        <option value="">{t.year}</option>
        {years.map((y) => (
          <option key={y} value={String(y)}>
            {y}
          </option>
        ))}
      </select>
      <select
        className={selectClass}
        aria-label={t.month}
        value={parts.month}
        onChange={(e) => pick({ month: e.target.value })}
      >
        <option value="">{t.month}</option>
        {Array.from({ length: 12 }, (_, i) => (
          <option key={i} value={pad(i + 1)}>
            {i + 1}
          </option>
        ))}
      </select>
      <select
        className={selectClass}
        aria-label={t.day}
        value={parts.day}
        onChange={(e) => pick({ day: e.target.value })}
      >
        <option value="">{t.day}</option>
        {Array.from({ length: days }, (_, i) => (
          <option key={i} value={pad(i + 1)}>
            {i + 1}
          </option>
        ))}
      </select>
    </span>
  );
}
