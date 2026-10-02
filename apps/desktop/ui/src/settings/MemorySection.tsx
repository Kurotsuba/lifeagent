import { useEffect, useState } from "react";
import { deleteMemory, getMemory } from "../api/commands";
import { commandErrorMessage, type MemoryNote } from "../api/types";
import { ConfirmDeleteButton } from "../components/ConfirmDeleteButton";
import { strings } from "../i18n";
import { LoadState } from "./LoadState";

const MEMORY_LIMIT = 100;

const t = strings.settings.memory;

/** "2026-09-26T10:15:00Z" -> "2026-09-26 10:15", in the viewer's local time. */
function formatCreatedAt(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function MemorySection() {
  const [notes, setNotes] = useState<MemoryNote[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function refetch() {
    try {
      setNotes(await getMemory(MEMORY_LIMIT));
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  useEffect(() => {
    void refetch();
  }, []);

  async function handleDelete(id: string) {
    setError(null);
    try {
      await deleteMemory(id);
      await refetch();
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  if (!notes) {
    return <LoadState error={error} />;
  }

  return (
    <div className="space-y-2">
      {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}
      {notes.length === 0 ? (
        <p className="text-sm text-neutral-500 dark:text-neutral-400">{t.empty}</p>
      ) : (
        notes.map((note) => (
          <div
            key={note.id}
            className="flex items-start justify-between gap-3 rounded-md border border-neutral-200 px-3 py-2 text-sm dark:border-neutral-700"
          >
            <div>
              <p className="text-neutral-900 dark:text-neutral-50">{note.note}</p>
              <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400">
                {t.createdAt(formatCreatedAt(note.created_at))}
              </p>
            </div>
            <ConfirmDeleteButton onConfirm={() => handleDelete(note.id)} />
          </div>
        ))
      )}
    </div>
  );
}
