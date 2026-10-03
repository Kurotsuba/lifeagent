import { useEffect, useState } from "react";
import { deleteAccount, me, SIGNED_OUT_EVENT } from "../api/auth";
import { commandErrorMessage } from "../api/types";
import { Field, inputClass } from "../components/Field";
import { primaryButtonClass } from "../components/buttonStyles";
import { strings } from "../i18n";

const t = strings.settings.account;

/** Web build only: who is logged in, and deleting the account with all of its data (the
 * server re-checks the password and drops the user's schema and role). */
export function AccountSection() {
  const [username, setUsername] = useState<string | null>(null);
  const [password, setPassword] = useState("");
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    me()
      .then((m) => !cancelled && setUsername(m.username))
      .catch(() => {
        // A failed lookup only hides the name; a dead session is handled app-wide.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleDelete() {
    setDeleting(true);
    setError(null);
    try {
      await deleteAccount(password);
      window.dispatchEvent(new Event(SIGNED_OUT_EVENT));
    } catch (e) {
      setError(commandErrorMessage(e));
      setDeleting(false);
    }
  }

  return (
    <div className="space-y-5">
      {username ? (
        <p className="text-sm text-neutral-700 dark:text-neutral-300">{t.loggedInAs(username)}</p>
      ) : null}

      <div className="space-y-3">
        <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-50">
          {t.deleteTitle}
        </h3>
        <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.deleteHint}</p>
        <Field label={t.passwordToConfirm}>
          <input
            type="password"
            autoComplete="current-password"
            className={inputClass}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </Field>
        <div className="flex items-center gap-3">
          <button
            type="button"
            className={`${primaryButtonClass} !bg-red-600 hover:!bg-red-700 dark:!text-white`}
            disabled={password.length === 0 || deleting}
            onClick={() => void handleDelete()}
          >
            {deleting ? t.deleting : t.deleteButton}
          </button>
          {error ? <span className="text-sm text-red-600 dark:text-red-400">{error}</span> : null}
        </div>
      </div>
    </div>
  );
}
