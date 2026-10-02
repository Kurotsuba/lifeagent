/**
 * How a command reaches the Rust side. On the desktop, Tauri's `invoke`; in a browser,
 * `POST /api/{command}` on `lifeagent-server`, which runs the same command functions
 * (`crates/api`). Both reject with the same `CommandError { code, message }` value, so callers
 * don't care which one they're on.
 *
 * Arguments are written the Tauri way (camelCase keys, which Tauri maps to the Rust snake_case
 * parameter names). The web transport does the same mapping itself, for top-level keys only:
 * nested values (a `Profile`, a `WeightGoal`) are already snake_case on both sides.
 */

import { invoke } from "@tauri-apps/api/core";
import { zhCN } from "../i18n/zh-CN";
import { isDesktop } from "./platform";
import { isCommandError, type CommandError } from "./types";

type Args = Record<string, unknown>;

/** Fired on `window` when the server answers 401: the session ended (logged out elsewhere,
 * expired, account deleted). The app goes back to the login screen. */
export const SESSION_EXPIRED_EVENT = "lifeagent:session-expired";

function snakeCaseKeys(args: Args): Args {
  const out: Args = {};
  for (const [key, value] of Object.entries(args)) {
    out[key.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`)] = value;
  }
  return out;
}

const networkError: CommandError = { code: "internal", message: zhCN.common.networkError };

/** Sends a request to the server and unwraps its answer: the JSON result, or a rejection with
 * the server's `CommandError` body. `reportExpiry`: announce a 401 as `SESSION_EXPIRED_EVENT`
 * (commands do; the account endpoints, where 401 is an expected answer, don't). */
export async function fetchJson<T>(
  path: string,
  init: RequestInit,
  reportExpiry = false,
): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, { credentials: "same-origin", ...init });
  } catch {
    throw networkError;
  }
  const body: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    if (
      reportExpiry &&
      response.status === 401 &&
      isCommandError(body) &&
      body.code === "unauthorized"
    ) {
      window.dispatchEvent(new Event(SESSION_EXPIRED_EVENT));
    }
    throw isCommandError(body) ? body : networkError;
  }
  return body as T;
}

export function call<T>(command: string, args: Args = {}): Promise<T> {
  if (isDesktop) {
    return invoke<T>(command, args);
  }
  return fetchJson<T>(
    `/api/${command}`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(snakeCaseKeys(args)),
    },
    true,
  );
}

/** A command whose argument is a file's raw bytes, with string headers alongside
 * (`preview_food_import`). */
export function callRaw<T>(
  command: string,
  bytes: Uint8Array<ArrayBuffer>,
  headers: Record<string, string>,
): Promise<T> {
  if (isDesktop) {
    return invoke<T>(command, bytes, { headers });
  }
  return fetchJson<T>(
    `/api/${command}`,
    {
      method: "POST",
      headers: { "content-type": "application/octet-stream", ...headers },
      body: bytes,
    },
    true,
  );
}
