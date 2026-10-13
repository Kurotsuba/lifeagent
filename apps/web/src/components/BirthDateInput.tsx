import { useEffect, useState } from "react";
import { strings } from "../i18n";
import { inputClass } from "./Field";

interface BirthDateInputProps {
  /** `YYYY-MM-DD`, or `""` until year and month are both picked. */
  value: string;
  onChange: (value: string) => void;
  /** The allowed ages, which bound the year list (the server still checks the exact date). */
  minAge: number;
  maxAge: number;
}

interface Parts {
  year: string;
  month: string;
  /** Kept from a saved date so an untouched profile round-trips unchanged; a new pick uses 01. */
  day: string;
}

function partsOf(value: string): Parts {
  const [year = "", month = "", day = ""] = value.split("-");
  return { year, month, day };
}

const pad = (n: number) => String(n).padStart(2, "0");

/** Year and month as two selects, year first; the day isn't asked and a picked date falls on the
 * 1st. A native date picker opens on today, so on a phone reaching a birth year means swiping
 * back decades; here the year list starts at the most recent year old enough. */
export function BirthDateInput({ value, onChange, minAge, maxAge }: BirthDateInputProps) {
  const t = strings.common.birthDate;
  const [parts, setParts] = useState<Parts>(() => partsOf(value));

  // A value set from outside (Settings loading the saved profile) replaces the picks; our own
  // `""` for a half-picked date doesn't.
  useEffect(() => {
    if (value !== "") setParts(partsOf(value));
  }, [value]);

  function pick(patch: Partial<Parts>) {
    const next = { ...parts, ...patch, day: "01" };
    setParts(next);
    onChange(next.year && next.month ? `${next.year}-${next.month}-${next.day}` : "");
  }

  // Calendar years only: the exact age check against the logical date is the server's.
  const thisYear = new Date().getFullYear();
  const years: number[] = [];
  for (let y = thisYear - minAge; y >= thisYear - maxAge - 1; y--) years.push(y);

  const selectClass = inputClass + " px-2";
  return (
    <span className="grid grid-cols-[2fr_1fr] gap-2">
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
    </span>
  );
}
