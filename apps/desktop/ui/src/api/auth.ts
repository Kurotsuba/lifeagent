/**
 * Account endpoints of the web build (`crates/server/src/auth.rs`). Web only: the desktop app
 * has a single local user and no accounts. The session is an `HttpOnly` cookie the browser
 * keeps; nothing here stores or sees it.
 */

import { fetchJson } from "./transport";

/** Fired on `window` after the user logs out or deletes their account here; the app goes back
 * to the login screen. (`SESSION_EXPIRED_EVENT` in `./transport` is the involuntary case.) */
export const SIGNED_OUT_EVENT = "lifeagent:signed-out";

export interface Me {
  username: string;
}

function post<T>(path: string, body: unknown): Promise<T> {
  return fetchJson<T>(`/api/auth/${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

/** The logged-in user; rejects with `unauthorized` when there is no valid session. */
export function me(): Promise<Me> {
  return fetchJson<Me>("/api/auth/me", { method: "GET" });
}

export function login(username: string, password: string): Promise<Me> {
  return post("login", { username, password });
}

export function register(
  username: string,
  password: string,
  inviteCode: string | null,
): Promise<Me> {
  return post("register", { username, password, invite_code: inviteCode });
}

export function logout(): Promise<void> {
  return post("logout", {});
}

/** Deletes the account and all of its data. Asks for the password again. */
export function deleteAccount(password: string): Promise<void> {
  return post("delete_account", { password });
}
