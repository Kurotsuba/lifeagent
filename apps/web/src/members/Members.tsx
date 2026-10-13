import { useEffect, useState } from "react";
import { getMembersView } from "../api/commands";
import { onAppDataEvent } from "../api/events";
import { commandErrorMessage, type MembersView, type MemberView } from "../api/types";
import { primaryButtonClass } from "../components/buttonStyles";
import { HelpButton } from "../components/Guide";
import { strings } from "../i18n";
import { MemberCard } from "./MemberCard";
import { MemberForm } from "./MemberForm";
import { WeekAttendance } from "./WeekAttendance";

const t = strings.members;

function Card({ children }: { children: React.ReactNode }) {
  return (
    <section className="rounded-lg border border-neutral-200 p-4 dark:border-neutral-700">
      {children}
    </section>
  );
}

/** The multimember plugin's page: the other people the user shares meals with, when they usually eat
 * together, one-off changes for the coming week, and their dislikes. All data from
 * `get_members_view`; refetched on every app data event (a member added in chat, ...). */
export function Members() {
  const [view, setView] = useState<MembersView | null>(null);
  const [error, setError] = useState<string | null>(null);
  // null: no form; "new": adding; otherwise the member being edited.
  const [editing, setEditing] = useState<MemberView | "new" | null>(null);

  async function refetch() {
    try {
      setView(await getMembersView());
      setError(null);
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  useEffect(() => {
    void refetch();
    return onAppDataEvent(() => void refetch());
  }, []);

  if (!view) {
    return (
      <div className="p-6">
        <p
          className={
            "text-sm " +
            (error ? "text-red-600 dark:text-red-400" : "text-neutral-500 dark:text-neutral-400")
          }
        >
          {error ?? strings.common.loading}
        </p>
      </div>
    );
  }

  const done = () => {
    setEditing(null);
    void refetch();
  };

  return (
    <div className="h-full overflow-y-auto p-6">
      <div className="mx-auto flex max-w-2xl flex-col gap-6">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-lg font-semibold text-neutral-900 dark:text-neutral-50">
              {t.title}
            </h1>
            <HelpButton topic="members" />
          </div>
          <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">{t.intro}</p>
        </div>
        {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}

        {editing ? (
          <Card>
            <MemberForm
              member={editing === "new" ? null : editing}
              onDone={done}
              onCancel={() => setEditing(null)}
            />
          </Card>
        ) : (
          <div>
            <button
              type="button"
              className={primaryButtonClass}
              onClick={() => setEditing("new")}
            >
              {t.add}
            </button>
          </div>
        )}

        {view.members.length === 0 ? (
          <p className="text-sm text-neutral-500 dark:text-neutral-400">{t.noMembers}</p>
        ) : (
          <>
            {view.members.map((m) => (
              <Card key={m.id}>
                <MemberCard
                  member={m}
                  onEdit={() => setEditing(m)}
                  onChanged={() => void refetch()}
                />
              </Card>
            ))}
            <Card>
              <WeekAttendance view={view} onChanged={() => void refetch()} />
            </Card>
          </>
        )}
      </div>
    </div>
  );
}
