import { useState } from "react";
import { addMember, updateMember } from "../api/commands";
import {
  commandErrorMessage,
  type ActivityLevel,
  type DietaryPattern,
  type MemberProfile,
  type MemberView,
  type Sex,
} from "../api/types";
import { Field, inputClass } from "../components/Field";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { strings } from "../i18n";
import { splitList } from "../onboarding/draft";

const t = strings.household.form;
const p = strings.onboarding.profile;

const ACTIVITY_LEVELS: ActivityLevel[] = ["sedentary", "light", "moderate", "active", "very_active"];
const PATTERNS: DietaryPattern[] = ["omnivore", "vegetarian", "vegan", "pescatarian"];

interface Draft {
  name: string;
  sex: Sex;
  birthDate: string;
  heightCm: string;
  weightKg: string;
  activityLevel: ActivityLevel;
  dietaryPattern: DietaryPattern;
  allergies: string;
}

function draftFrom(member: MemberView | null): Draft {
  return member
    ? {
        name: member.display_name,
        sex: member.sex,
        birthDate: member.birth_date,
        heightCm: String(member.height_cm),
        weightKg: String(member.weight_kg),
        activityLevel: member.activity_level,
        dietaryPattern: member.dietary_pattern,
        allergies: member.allergies.join(", "),
      }
    : {
        name: "",
        sex: "female",
        birthDate: "",
        heightCm: "",
        weightKg: "",
        activityLevel: "light",
        dietaryPattern: "omnivore",
        allergies: "",
      };
}

interface MemberFormProps {
  /** The member to edit, or null to add one. */
  member: MemberView | null;
  onDone: () => void;
  onCancel: () => void;
}

/** Adds or edits a household member. Age, height and weight bounds are checked by the backend,
 * whose message is shown as is. */
export function MemberForm({ member, onDone, onCancel }: MemberFormProps) {
  const [draft, setDraft] = useState(() => draftFrom(member));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function update(patch: Partial<Draft>) {
    setDraft((d) => ({ ...d, ...patch }));
  }

  const complete =
    draft.name.trim() !== "" &&
    draft.birthDate !== "" &&
    Number(draft.heightCm) > 0 &&
    Number(draft.weightKg) > 0;

  async function handleSave() {
    const profile: MemberProfile = {
      display_name: draft.name,
      sex: draft.sex,
      birth_date: draft.birthDate,
      height_cm: Number(draft.heightCm),
      weight_kg: Number(draft.weightKg),
      activity_level: draft.activityLevel,
      dietary_pattern: draft.dietaryPattern,
      allergies: splitList(draft.allergies),
    };
    setSaving(true);
    setError(null);
    try {
      if (member) await updateMember(member.id, profile);
      else await addMember(profile);
      onDone();
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="space-y-4">
      <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-50">
        {member ? t.editTitle(member.display_name) : t.addTitle}
      </h2>
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <Field label={t.name}>
          <input
            type="text"
            className={inputClass}
            maxLength={40}
            value={draft.name}
            onChange={(e) => update({ name: e.target.value })}
          />
        </Field>
        <Field label={p.sex}>
          <select
            className={inputClass}
            value={draft.sex}
            onChange={(e) => update({ sex: e.target.value as Sex })}
          >
            {(Object.entries(p.sexOptions) as [Sex, string][]).map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </Field>
        <Field label={p.birthDate} hint={t.ageHint}>
          <input
            type="date"
            className={inputClass}
            value={draft.birthDate}
            onChange={(e) => update({ birthDate: e.target.value })}
          />
        </Field>
        <Field label={p.heightCm}>
          <input
            type="number"
            min={120}
            max={230}
            step="0.1"
            className={inputClass}
            value={draft.heightCm}
            onChange={(e) => update({ heightCm: e.target.value })}
          />
        </Field>
        <Field label={t.weightKg} hint={t.weightHint}>
          <input
            type="number"
            min={30}
            max={300}
            step="0.1"
            className={inputClass}
            value={draft.weightKg}
            onChange={(e) => update({ weightKg: e.target.value })}
          />
        </Field>
        <Field label={p.activityLevel}>
          <select
            className={inputClass}
            value={draft.activityLevel}
            onChange={(e) => update({ activityLevel: e.target.value as ActivityLevel })}
          >
            {ACTIVITY_LEVELS.map((level) => (
              <option key={level} value={level}>
                {p.activityLevelHints[level]}
              </option>
            ))}
          </select>
        </Field>
        <Field label={p.dietaryPattern}>
          <select
            className={inputClass}
            value={draft.dietaryPattern}
            onChange={(e) => update({ dietaryPattern: e.target.value as DietaryPattern })}
          >
            {PATTERNS.map((pattern) => (
              <option key={pattern} value={pattern}>
                {p.dietaryPatternOptions[pattern]}
              </option>
            ))}
          </select>
        </Field>
        <Field label={strings.common.withOptional(p.allergies)} hint={p.allergiesPlaceholder}>
          <input
            type="text"
            className={inputClass}
            value={draft.allergies}
            onChange={(e) => update({ allergies: e.target.value })}
          />
        </Field>
      </div>
      {!complete ? <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.invalid}</p> : null}
      {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}
      <div className="flex gap-3">
        <button
          type="button"
          className={primaryButtonClass}
          disabled={saving || !complete}
          onClick={() => void handleSave()}
        >
          {saving ? strings.common.saving : strings.common.save}
        </button>
        <button type="button" className={secondaryButtonClass} onClick={onCancel}>
          {strings.common.cancel}
        </button>
      </div>
    </div>
  );
}
