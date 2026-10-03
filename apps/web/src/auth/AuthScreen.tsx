import { useState, type FormEvent } from "react";
import { login, register } from "../api/auth";
import { commandErrorMessage } from "../api/types";
import { Field, inputClass } from "../components/Field";
import { primaryButtonClass } from "../components/buttonStyles";
import { LanguageSelect } from "../components/LanguageSelect";
import { strings } from "../i18n";

const t = strings.auth;

interface AuthScreenProps {
  /** Shown above the form, e.g. after the session expired. */
  notice: string | null;
  onSignedIn: () => void;
}

/** Web build only: log in, or create an account. Every rule (username format, password
 * length) is checked by the server; the form only checks that the two password
 * fields match, which the server never sees. */
export function AuthScreen({ notice, onSignedIn }: AuthScreenProps) {
  const [mode, setMode] = useState<"login" | "register">("login");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isRegister = mode === "register";

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (isRegister && password !== confirm) {
      setError(t.passwordMismatch);
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      if (isRegister) {
        await register(username, password);
      } else {
        await login(username, password);
      }
      onSignedIn();
    } catch (err) {
      setError(commandErrorMessage(err));
      setSubmitting(false);
    }
  }

  function switchMode() {
    setMode(isRegister ? "login" : "register");
    setError(null);
    setConfirm("");
  }

  return (
    // `m-auto` rather than centering the flex items: when the form is taller than the screen
    // (phone keyboard open), it scrolls instead of losing its top.
    <div className="flex h-full overflow-y-auto px-6 py-8">
      <form className="m-auto w-full max-w-sm space-y-4" onSubmit={(e) => void handleSubmit(e)}>
        <div>
          <div className="flex items-center justify-between">
            <p className="text-sm font-semibold text-neutral-500 dark:text-neutral-400">
              {strings.app.name}
            </p>
            <LanguageSelect />
          </div>
          <h1 className="mt-1 text-xl font-semibold">{isRegister ? t.registerTitle : t.loginTitle}</h1>
        </div>

        {notice ? (
          <p className="rounded-md bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:bg-amber-900/30 dark:text-amber-200">
            {notice}
          </p>
        ) : null}

        <Field label={t.username} hint={isRegister ? t.usernameHint : undefined}>
          <input
            className={inputClass}
            autoComplete="username"
            autoCapitalize="none"
            spellCheck={false}
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            required
          />
        </Field>
        <Field label={t.password} hint={isRegister ? t.passwordHint : undefined}>
          <input
            type="password"
            className={inputClass}
            autoComplete={isRegister ? "new-password" : "current-password"}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
        </Field>
        {isRegister ? (
          <>
            <Field label={t.confirmPassword}>
              <input
                type="password"
                className={inputClass}
                autoComplete="new-password"
                value={confirm}
                onChange={(e) => setConfirm(e.target.value)}
                required
              />
            </Field>
          </>
        ) : null}

        {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}

        <button type="submit" className={`${primaryButtonClass} w-full`} disabled={submitting}>
          {submitting ? t.submitting : isRegister ? t.register : t.login}
        </button>
        <button
          type="button"
          className="w-full text-center text-sm text-neutral-600 underline dark:text-neutral-400"
          onClick={switchMode}
        >
          {isRegister ? t.toLogin : t.toRegister}
        </button>
      </form>
    </div>
  );
}
